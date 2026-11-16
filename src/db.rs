#![allow(dead_code)]

use crate::models::{AccountRecord, AccountStatus};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open SQLite database at {}", path))?;
        let db = Database { conn };
        db.init_schema()?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Database { conn };
        db.init_schema()?;
        Ok(db)
    }

    pub fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS accounts (
                username TEXT PRIMARY KEY,
                sorsa_score INTEGER NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('TRIAL_48H', 'MUTUAL_ACTIVE', 'DROPPED_BLACKLIST')),
                followed_at TEXT NOT NULL,
                last_checked_at TEXT NOT NULL,
                follback_detected_at TEXT,
                dropped_at TEXT,
                replied_count INTEGER NOT NULL DEFAULT 0,
                last_action TEXT NOT NULL,
                notes TEXT
            );

            CREATE TABLE IF NOT EXISTS interacted_posts (
                post_id TEXT PRIMARY KEY,
                username TEXT NOT NULL,
                action_type TEXT NOT NULL,
                reply_text TEXT NOT NULL,
                interacted_at TEXT NOT NULL,
                FOREIGN KEY (username) REFERENCES accounts(username)
            );

            CREATE TABLE IF NOT EXISTS crm_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT NOT NULL,
                event_type TEXT NOT NULL,
                details TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_accounts_status ON accounts(status);
            CREATE INDEX IF NOT EXISTS idx_interacted_posts_user ON interacted_posts(username);
            "#,
        )?;
        Ok(())
    }

    /// Normalize username to standard lowercase without leading '@'
    pub fn normalize_username(username: &str) -> String {
        username.trim().trim_start_matches('@').to_lowercase()
    }

    /// GOLDEN RULE: Check whether the account is permanently blacklisted.
    /// DROPPED_BLACKLIST accounts must never be followed, liked, or replied to.
    pub fn is_blacklisted(&self, username: &str) -> Result<bool> {
        let norm = Self::normalize_username(username);
        let status_str: Option<String> = self
            .conn
            .query_row(
                "SELECT status FROM accounts WHERE username = ?1",
                params![norm],
                |row| row.get(0),
            )
            .optional()?;

        Ok(status_str.as_deref() == Some("DROPPED_BLACKLIST"))
    }

    /// Check if account can be interacted with.
    /// Returns false if already in DROPPED_BLACKLIST.
    pub fn can_interact(&self, username: &str) -> Result<bool> {
        Ok(!self.is_blacklisted(username)?)
    }

    pub fn get_account(&self, username: &str) -> Result<Option<AccountRecord>> {
        let norm = Self::normalize_username(username);
        let mut stmt = self.conn.prepare(
            r#"
            SELECT username, sorsa_score, status, followed_at, last_checked_at,
                   follback_detected_at, dropped_at, replied_count, last_action, notes
            FROM accounts
            WHERE username = ?1
            "#,
        )?;

        let account = stmt
            .query_row(params![norm], |row| {
                let status_str: String = row.get(2)?;
                let status = AccountStatus::from_str_strict(&status_str)
                    .unwrap_or(AccountStatus::Trial48h);
                let followed_at_str: String = row.get(3)?;
                let last_checked_str: String = row.get(4)?;
                let follback_str: Option<String> = row.get(5)?;
                let dropped_str: Option<String> = row.get(6)?;

                Ok(AccountRecord {
                    username: row.get(0)?,
                    sorsa_score: row.get(1)?,
                    status,
                    followed_at: followed_at_str.parse::<DateTime<Utc>>().unwrap_or_else(|_| Utc::now()),
                    last_checked_at: last_checked_str.parse::<DateTime<Utc>>().unwrap_or_else(|_| Utc::now()),
                    follback_detected_at: follback_str.and_then(|s| s.parse::<DateTime<Utc>>().ok()),
                    dropped_at: dropped_str.and_then(|s| s.parse::<DateTime<Utc>>().ok()),
                    replied_count: row.get(7)?,
                    last_action: row.get(8)?,
                    notes: row.get(9)?,
                })
            })
            .optional()?;

        Ok(account)
    }

    /// Insert newly discovered account as TRIAL_48H.
    /// Fails immediately if the account is in DROPPED_BLACKLIST (Golden Rule).
    pub fn insert_trial_account(
        &self,
        username: &str,
        sorsa_score: i64,
        initial_action: &str,
    ) -> Result<bool> {
        let norm = Self::normalize_username(username);

        // Check if account already exists
        if let Some(existing) = self.get_account(&norm)? {
            if existing.status == AccountStatus::DroppedBlacklist {
                bail!(
                    "GOLDEN RULE VIOLATION BLOCKED: Cannot add or interact with account '@{}' because it is permanently DROPPED_BLACKLIST",
                    norm
                );
            }
            // Already tracked, do not re-add
            return Ok(false);
        }

        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            r#"
            INSERT INTO accounts (username, sorsa_score, status, followed_at, last_checked_at, replied_count, last_action)
            VALUES (?1, ?2, 'TRIAL_48H', ?3, ?3, 0, ?4)
            "#,
            params![norm, sorsa_score, now, initial_action],
        )?;

        self.log_event(
            &norm,
            "TRIAL_STARTED",
            &format!("Discovered with score {}, trial 48h initiated", sorsa_score),
        )?;

        Ok(true)
    }

    /// Promote account to MUTUAL_ACTIVE when "Follows you" is detected in DOM
    pub fn promote_to_mutual(&self, username: &str, action_details: &str) -> Result<()> {
        let norm = Self::normalize_username(username);

        if self.is_blacklisted(&norm)? {
            bail!(
                "GOLDEN RULE VIOLATION: Cannot promote blacklisted account '@{}'",
                norm
            );
        }

        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            r#"
            UPDATE accounts
            SET status = 'MUTUAL_ACTIVE',
                follback_detected_at = ?1,
                last_checked_at = ?1,
                last_action = ?2
            WHERE username = ?3
            "#,
            params![now, action_details, norm],
        )?;

        self.log_event(&norm, "PROMOTED_MUTUAL", action_details)?;
        Ok(())
    }

    /// Transition account to DROPPED_BLACKLIST after 48h expired without follback.
    /// GOLDEN RULE: This record must NEVER be deleted!
    pub fn drop_and_blacklist(&self, username: &str, action_details: &str) -> Result<()> {
        let norm = Self::normalize_username(username);
        let now = Utc::now().to_rfc3339();

        self.conn.execute(
            r#"
            UPDATE accounts
            SET status = 'DROPPED_BLACKLIST',
                dropped_at = ?1,
                last_checked_at = ?1,
                last_action = ?2
            WHERE username = ?3
            "#,
            params![now, action_details, norm],
        )?;

        self.log_event(&norm, "UNFOLLOWED_BLACKLISTED", action_details)?;
        Ok(())
    }

    pub fn update_last_action(&self, username: &str, action: &str) -> Result<()> {
        let norm = Self::normalize_username(username);
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE accounts SET last_action = ?1, last_checked_at = ?2 WHERE username = ?3",
            params![action, now, norm],
        )?;
        Ok(())
    }

    pub fn increment_replied_count(&self, username: &str) -> Result<()> {
        let norm = Self::normalize_username(username);
        self.conn.execute(
            "UPDATE accounts SET replied_count = replied_count + 1 WHERE username = ?1",
            params![norm],
        )?;
        Ok(())
    }

    /// Check if post has already been interacted with (anti-duplication)
    pub fn is_post_interacted(&self, post_id: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM interacted_posts WHERE post_id = ?1",
            params![post_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Record interacted post to prevent duplicate replies and likes
    pub fn record_post_interaction(
        &self,
        post_id: &str,
        username: &str,
        action_type: &str,
        reply_text: &str,
    ) -> Result<()> {
        let norm = Self::normalize_username(username);

        if self.is_blacklisted(&norm)? {
            bail!(
                "GOLDEN RULE VIOLATION: Refusing to interact with blacklisted account '@{}'",
                norm
            );
        }

        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            r#"
            INSERT OR IGNORE INTO interacted_posts (post_id, username, action_type, reply_text, interacted_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![post_id, norm, action_type, reply_text, now],
        )?;

        self.increment_replied_count(&norm)?;
        self.log_event(
            &norm,
            "POST_INTERACTION",
            &format!("Action: {}, Post ID: {}", action_type, post_id),
        )?;

        Ok(())
    }

    pub fn get_all_accounts(&self) -> Result<Vec<AccountRecord>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT username, sorsa_score, status, followed_at, last_checked_at,
                   follback_detected_at, dropped_at, replied_count, last_action, notes
            FROM accounts
            ORDER BY
                CASE status
                    WHEN 'TRIAL_48H' THEN 1
                    WHEN 'MUTUAL_ACTIVE' THEN 2
                    WHEN 'DROPPED_BLACKLIST' THEN 3
                    ELSE 4
                END,
                sorsa_score DESC
            "#,
        )?;

        let iter = stmt.query_map([], |row| {
            let status_str: String = row.get(2)?;
            let status = AccountStatus::from_str_strict(&status_str)
                .unwrap_or(AccountStatus::Trial48h);
            let followed_at_str: String = row.get(3)?;
            let last_checked_str: String = row.get(4)?;
            let follback_str: Option<String> = row.get(5)?;
            let dropped_str: Option<String> = row.get(6)?;

            Ok(AccountRecord {
                username: row.get(0)?,
                sorsa_score: row.get(1)?,
                status,
                followed_at: followed_at_str.parse::<DateTime<Utc>>().unwrap_or_else(|_| Utc::now()),
                last_checked_at: last_checked_str.parse::<DateTime<Utc>>().unwrap_or_else(|_| Utc::now()),
                follback_detected_at: follback_str.and_then(|s| s.parse::<DateTime<Utc>>().ok()),
                dropped_at: dropped_str.and_then(|s| s.parse::<DateTime<Utc>>().ok()),
                replied_count: row.get(7)?,
                last_action: row.get(8)?,
                notes: row.get(9)?,
            })
        })?;

        let mut accounts = Vec::new();
        for item in iter {
            accounts.push(item?);
        }
        Ok(accounts)
    }

    pub fn get_accounts_by_status(&self, status: AccountStatus) -> Result<Vec<AccountRecord>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT username, sorsa_score, status, followed_at, last_checked_at,
                   follback_detected_at, dropped_at, replied_count, last_action, notes
            FROM accounts
            WHERE status = ?1
            ORDER BY sorsa_score DESC
            "#,
        )?;

        let iter = stmt.query_map(params![status.as_str()], |row| {
            let status_str: String = row.get(2)?;
            let s = AccountStatus::from_str_strict(&status_str).unwrap_or(status);
            let followed_at_str: String = row.get(3)?;
            let last_checked_str: String = row.get(4)?;
            let follback_str: Option<String> = row.get(5)?;
            let dropped_str: Option<String> = row.get(6)?;

            Ok(AccountRecord {
                username: row.get(0)?,
                sorsa_score: row.get(1)?,
                status: s,
                followed_at: followed_at_str.parse::<DateTime<Utc>>().unwrap_or_else(|_| Utc::now()),
                last_checked_at: last_checked_str.parse::<DateTime<Utc>>().unwrap_or_else(|_| Utc::now()),
                follback_detected_at: follback_str.and_then(|s| s.parse::<DateTime<Utc>>().ok()),
                dropped_at: dropped_str.and_then(|s| s.parse::<DateTime<Utc>>().ok()),
                replied_count: row.get(7)?,
                last_action: row.get(8)?,
                notes: row.get(9)?,
            })
        })?;

        let mut accounts = Vec::new();
        for item in iter {
            accounts.push(item?);
        }
        Ok(accounts)
    }

    pub fn log_event(&self, username: &str, event_type: &str, details: &str) -> Result<()> {
        let norm = Self::normalize_username(username);
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO crm_events (username, event_type, details, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![norm, event_type, details, now],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fsm_lifecycle_and_golden_rules() -> Result<()> {
        let db = Database::open_in_memory()?;

        // 1. Initial trial insert
        let inserted = db.insert_trial_account("cryptogod", 750, "Followed and liked")?;
        assert!(inserted);

        let acc = db.get_account("cryptogod")?.unwrap();
        assert_eq!(acc.status, AccountStatus::Trial48h);
        assert_eq!(acc.sorsa_score, 750);

        // 2. Promotion to MUTUAL_ACTIVE
        db.promote_to_mutual("cryptogod", "Follback verified")?;
        let acc2 = db.get_account("cryptogod")?.unwrap();
        assert_eq!(acc2.status, AccountStatus::MutualActive);

        // 3. Drop and Blacklist another account
        db.insert_trial_account("ghost_user", 600, "Followed")?;
        db.drop_and_blacklist("ghost_user", "48h expired without follback")?;

        let ghost = db.get_account("ghost_user")?.unwrap();
        assert_eq!(ghost.status, AccountStatus::DroppedBlacklist);
        assert!(db.is_blacklisted("ghost_user")?);

        // 4. GOLDEN RULE: Cannot re-insert or interact with DROPPED_BLACKLIST
        let reinsert_attempt = db.insert_trial_account("ghost_user", 800, "Trying again");
        assert!(reinsert_attempt.is_err(), "Golden rule must prevent re-adding blacklisted accounts");

        let interact_attempt = db.record_post_interaction("12345", "ghost_user", "like", "hello");
        assert!(interact_attempt.is_err(), "Golden rule must prevent interacting with blacklisted accounts");

        // 5. Anti-duplication of post interactions
        db.record_post_interaction("99999", "cryptogod", "like_and_reply", "Nice post")?;
        assert!(db.is_post_interacted("99999")?);
        assert!(!db.is_post_interacted("88888")?);

        Ok(())
    }
}
