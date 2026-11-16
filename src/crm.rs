use crate::browser::CdpSession;
use crate::db::Database;
use crate::dom::DomParser;
use crate::models::AccountStatus;
use crate::prompt::PromptEngine;
use anyhow::Result;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};

pub struct CrmEngine<'a> {
    db: &'a Database,
    prompt: &'a PromptEngine,
}

impl<'a> CrmEngine<'a> {
    pub fn new(db: &'a Database, prompt: &'a PromptEngine) -> Self {
        Self { db, prompt }
    }

    /// Process a discovered account (Sorsa > 500).
    /// Enforces Golden Rule (anti-duplication, strict blacklist protection).
    pub async fn process_discovered_account(
        &self,
        cdp: &mut CdpSession,
        username: &str,
        discovered_score: i64,
    ) -> Result<bool> {
        let norm = Database::normalize_username(username);

        // Check if account is permanently blacklisted (GOLDEN RULE)
        if self.db.is_blacklisted(&norm)? {
            warn!(
                "GOLDEN RULE ENFORCED: Refusing to interact with permanently blacklisted account '@{}'",
                norm
            );
            return Ok(false);
        }

        // Check if account is already in database
        if let Some(existing) = self.db.get_account(&norm)? {
            info!(
                "Account '@{}' is already tracked in database (status: {}). Skipping discovery.",
                norm, existing.status
            );
            return Ok(false);
        }

        if discovered_score <= 500 {
            info!("Account '@{}' score ({}) <= 500. Skipping.", norm, discovered_score);
            return Ok(false);
        }

        info!(
            "Processing new discovery: '@{}' (Sorsa Score: {} > 500)",
            norm, discovered_score
        );

        // Navigate to the user's X profile
        let profile_url = format!("https://x.com/{}", norm);
        cdp.navigate(&profile_url).await?;
        sleep(Duration::from_millis(2000)).await;

        // Verify or refine Sorsa score from profile DOM
        let profile_score = DomParser::extract_profile_sorsa_score(cdp).await?
            .unwrap_or(discovered_score);

        if profile_score <= 500 {
            info!(
                "Verified profile score for '@{}' is {} <= 500. Skipping.",
                norm, profile_score
            );
            return Ok(false);
        }

        // Check if already mutual or following
        let is_follows_you = DomParser::check_follows_you(cdp).await?;
        let is_following = DomParser::check_is_following(cdp).await?;

        if is_follows_you {
            info!("Account '@{}' already follows you! Inserting directly as MUTUAL_ACTIVE", norm);
            self.db.insert_trial_account(&norm, profile_score, "Discovered as mutual follower")?;
            self.db.promote_to_mutual(&norm, "Initial discovery already follows you")?;
            return Ok(true);
        }

        // Step 1: Follow the account
        let follow_done = if !is_following {
            let res = DomParser::click_follow(cdp).await?;
            if res {
                info!("Followed account '@{}'", norm);
            }
            res
        } else {
            info!("Already following '@{}'", norm);
            true
        };

        // Step 2: Like & Reply to their latest post
        let mut interaction_summary = if follow_done { "Followed" } else { "Follow failed" }.to_string();

        if let Ok(Some(latest_tweet)) = DomParser::extract_latest_tweet(cdp).await {
            let tweet_id = &latest_tweet.tweet_id;
            let project = PromptEngine::extract_project_name(&latest_tweet.text, &norm);
            let (_opt1, _opt2, _opt3, recommended_reply) =
                self.prompt.generate_draft_replies(&project, &latest_tweet.text);

            // Like post if not yet liked
            if !latest_tweet.is_liked {
                if DomParser::like_latest_tweet(cdp).await? {
                    info!("Liked latest post ({}) of '@{}'", tweet_id, norm);
                }
            }

            // Reply using template prompt
            if DomParser::reply_to_latest_tweet(cdp, &recommended_reply).await? {
                info!(
                    "Replied to post ({}) of '@{}' with: '{}'",
                    tweet_id, norm, recommended_reply
                );
                self.db.record_post_interaction(
                    tweet_id,
                    &norm,
                    "like_and_reply",
                    &recommended_reply,
                )?;
                interaction_summary.push_str(&format!(" & Replied to tweet {}", tweet_id));
            }
        }

        // Step 3: Insert into database with TRIAL_48H status
        self.db.insert_trial_account(&norm, profile_score, &interaction_summary)?;
        info!(
            "Successfully initiated 48h trial for '@{}' (Score: {})",
            norm, profile_score
        );

        Ok(true)
    }

    /// Evaluate FSM state for all TRIAL_48H accounts:
    /// - Check if 48h expired without follback -> Unfollow & Blacklist
    /// - Check if follback received -> Promote to MUTUAL_ACTIVE
    pub async fn evaluate_fsm_cycle(&self, cdp: &mut CdpSession) -> Result<()> {
        let trials = self.db.get_accounts_by_status(AccountStatus::Trial48h)?;
        if trials.is_empty() {
            return Ok(());
        }

        info!("Evaluating {} accounts in TRIAL_48H status...", trials.len());

        for account in trials {
            let norm = &account.username;
            let profile_url = format!("https://x.com/{}", norm);
            cdp.navigate(&profile_url).await?;
            sleep(Duration::from_millis(2000)).await;

            // Check if account has follback
            let follows_you = DomParser::check_follows_you(cdp).await?;

            if follows_you {
                info!(
                    "Follback detected for '@{}'! Promoting to MUTUAL_ACTIVE.",
                    norm
                );
                self.db.promote_to_mutual(norm, "Follows you detected in profile DOM")?;
                continue;
            }

            // Check if 48 hours have expired
            if account.is_trial_expired() {
                warn!(
                    "Trial 48h expired for '@{}' without follback! Executing Unfollow and moving to DROPPED_BLACKLIST.",
                    norm
                );

                let unfollowed = DomParser::click_unfollow(cdp).await?;
                let action_msg = if unfollowed {
                    "Unfollowed after 48h expired without follback -> Blacklisted permanently"
                } else {
                    "48h expired without follback (Unfollow button clicked) -> Blacklisted"
                };

                // GOLDEN RULE: Update status to DROPPED_BLACKLIST, never delete record!
                self.db.drop_and_blacklist(norm, action_msg)?;
                info!("Account '@{}' is now permanently in DROPPED_BLACKLIST", norm);
            } else {
                let remaining = account.trial_remaining_display();
                info!("Account '@{}' still in trial ({} remaining)", norm, remaining);
            }
        }

        Ok(())
    }

    /// Scan active accounts (TRIAL_48H within 48h and MUTUAL_ACTIVE) for new tweets.
    /// If a new tweet is detected that hasn't been interacted with, Like and Reply using prompt template.
    pub async fn scan_and_interact_new_posts(&self, cdp: &mut CdpSession) -> Result<()> {
        let mut active_accounts = self.db.get_accounts_by_status(AccountStatus::MutualActive)?;
        let mut trial_accounts = self.db.get_accounts_by_status(AccountStatus::Trial48h)?;

        // Only include trials that haven't expired
        trial_accounts.retain(|a| !a.is_trial_expired());
        active_accounts.extend(trial_accounts);

        if active_accounts.is_empty() {
            return Ok(());
        }

        info!(
            "Scanning new posts for {} active / trial accounts...",
            active_accounts.len()
        );

        for account in active_accounts {
            let norm = &account.username;

            // Skip if somehow blacklisted
            if self.db.is_blacklisted(norm)? {
                continue;
            }

            let profile_url = format!("https://x.com/{}", norm);
            cdp.navigate(&profile_url).await?;
            sleep(Duration::from_millis(2000)).await;

            if let Ok(Some(tweet)) = DomParser::extract_latest_tweet(cdp).await {
                let tweet_id = &tweet.tweet_id;

                // Check anti-duplication
                if self.db.is_post_interacted(tweet_id)? {
                    continue;
                }

                info!(
                    "Found new post ({}) for '@{}': '{}'",
                    tweet_id, norm, tweet.text
                );

                // Like the tweet
                if !tweet.is_liked {
                    let _ = DomParser::like_latest_tweet(cdp).await;
                }

                // Generate and post reply
                let project = PromptEngine::extract_project_name(&tweet.text, norm);
                let (_opt1, _opt2, _opt3, recommended) =
                    self.prompt.generate_draft_replies(&project, &tweet.text);

                if DomParser::reply_to_latest_tweet(cdp, &recommended).await? {
                    info!(
                        "Successfully sent reply to post ({}) for '@{}': '{}'",
                        tweet_id, norm, recommended
                    );
                    self.db.record_post_interaction(
                        tweet_id,
                        norm,
                        "like_and_reply",
                        &recommended,
                    )?;
                    let action = format!("Replied to new tweet {}", tweet_id);
                    self.db.update_last_action(norm, &action)?;
                }
            }
        }

        Ok(())
    }
}
