#![allow(dead_code)]

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountStatus {
    Trial48h,
    MutualActive,
    DroppedBlacklist,
}

impl AccountStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AccountStatus::Trial48h => "TRIAL_48H",
            AccountStatus::MutualActive => "MUTUAL_ACTIVE",
            AccountStatus::DroppedBlacklist => "DROPPED_BLACKLIST",
        }
    }

    pub fn from_str_strict(s: &str) -> Option<Self> {
        match s.trim().to_uppercase().as_str() {
            "TRIAL_48H" => Some(AccountStatus::Trial48h),
            "MUTUAL_ACTIVE" => Some(AccountStatus::MutualActive),
            "DROPPED_BLACKLIST" => Some(AccountStatus::DroppedBlacklist),
            _ => None,
        }
    }
}

impl fmt::Display for AccountStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountRecord {
    pub username: String,
    pub sorsa_score: i64,
    pub status: AccountStatus,
    pub followed_at: DateTime<Utc>,
    pub last_checked_at: DateTime<Utc>,
    pub follback_detected_at: Option<DateTime<Utc>>,
    pub dropped_at: Option<DateTime<Utc>>,
    pub replied_count: i64,
    pub last_action: String,
    pub notes: Option<String>,
}

impl AccountRecord {
    pub const TRIAL_DURATION_HOURS: i64 = 48;

    /// Calculate remaining duration in the 48h trial period.
    pub fn trial_remaining_duration(&self) -> Option<Duration> {
        if self.status != AccountStatus::Trial48h {
            return None;
        }
        let now = Utc::now();
        let deadline = self.followed_at + Duration::hours(Self::TRIAL_DURATION_HOURS);
        if now >= deadline {
            Some(Duration::zero())
        } else {
            Some(deadline - now)
        }
    }

    /// Check if the 48-hour trial has expired without follback
    pub fn is_trial_expired(&self) -> bool {
        if self.status != AccountStatus::Trial48h {
            return false;
        }
        let now = Utc::now();
        let deadline = self.followed_at + Duration::hours(Self::TRIAL_DURATION_HOURS);
        now >= deadline
    }

    /// Formatted display string for CLI report table
    pub fn trial_remaining_display(&self) -> String {
        match self.status {
            AccountStatus::Trial48h => match self.trial_remaining_duration() {
                Some(rem) if rem > Duration::zero() => {
                    let total_secs = rem.num_seconds();
                    let hours = total_secs / 3600;
                    let mins = (total_secs % 3600) / 60;
                    format!("{}h {}m remaining", hours, mins)
                }
                _ => "Expired (Awaiting drop)".to_string(),
            },
            AccountStatus::MutualActive => "N/A (Mutual Follback)".to_string(),
            AccountStatus::DroppedBlacklist => "Blacklisted (Permanent)".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractedPost {
    pub post_id: String,
    pub username: String,
    pub action_type: String,
    pub reply_text: String,
    pub interacted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrmEvent {
    pub id: i64,
    pub username: String,
    pub event_type: String,
    pub details: String,
    pub created_at: DateTime<Utc>,
}
