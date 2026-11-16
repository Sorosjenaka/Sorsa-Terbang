#![allow(dead_code)]

use crate::browser::CdpSession;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredAccount {
    pub username: String,
    pub sorsa_score: i64,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestTweetInfo {
    pub tweet_id: String,
    pub tweet_url: String,
    pub text: String,
    pub is_liked: bool,
}

pub struct DomParser;

impl DomParser {
    /// Read Sorsa score from an X profile page DOM (injected by Sorsa extension)
    /// Sorsa extension injects `.score-badge` containing e.g. "S:650 | T:420 | Flw:12.5K"
    pub async fn extract_profile_sorsa_score(cdp: &mut CdpSession) -> Result<Option<i64>> {
        let js = r#"
        (() => {
            // Strategy 1: Sorsa Extension Official .score-badge element
            const badges = document.querySelectorAll('.score-badge');
            for (const badge of badges) {
                const text = badge.innerText || badge.textContent || '';
                const m = text.match(/S:\s*([1-9]\d{1,4})/i);
                if (m) return parseInt(m[1]);
            }

            // Strategy 2: data-attributes injected by Sorsa extension
            const sorsaDataEl = document.querySelector('[data-sorsa-score], [data-sorsa], [data-score]');
            if (sorsaDataEl) {
                const val = sorsaDataEl.getAttribute('data-sorsa-score') || sorsaDataEl.getAttribute('data-score') || sorsaDataEl.getAttribute('data-sorsa');
                if (val && !isNaN(parseInt(val))) return parseInt(val);
            }

            // Strategy 3: Elements containing 'Sorsa' in header / profile items
            const header = document.querySelector('[data-testid="UserProfileHeader_Items"]') || document.querySelector('[data-testid="UserName"]');
            if (header) {
                const spans = Array.from(header.querySelectorAll('span, div, a'));
                for (const el of spans) {
                    const t = (el.innerText || '').trim();
                    const mSorsa = t.match(/S:\s*([1-9]\d{1,4})/i) || t.match(/Sorsa\s*Score[:\s]*([1-9]\d{1,4})/i);
                    if (mSorsa) return parseInt(mSorsa[1]);
                }
            }

            // Strategy 4: Fallback scan near username / bio
            const allSpans = Array.from(document.querySelectorAll('div[data-testid="primaryColumn"] span'));
            for (const s of allSpans) {
                const t = (s.innerText || '').trim();
                if (t.toLowerCase().includes('sorsa score:')) {
                    const m = t.match(/:\s*([1-9]\d{2,4})/);
                    if (m) return parseInt(m[1]);
                }
            }

            return null;
        })()
        "#;

        let res = cdp.eval_js(js).await?;
        if let Some(num) = res.as_i64() {
            Ok(Some(num))
        } else {
            Ok(None)
        }
    }

    /// Check if profile has "Follows you" indicator in DOM
    pub async fn check_follows_you(cdp: &mut CdpSession) -> Result<bool> {
        let js = r#"
        (() => {
            // Standard X Follows You badge
            const indicator = document.querySelector('[data-testid="userFollowIndicator"]');
            if (indicator) return true;

            // Text search in primary column
            const header = document.querySelector('[data-testid="UserName"]') || document.body;
            const spans = Array.from(header.querySelectorAll('span'));
            for (const s of spans) {
                const txt = (s.innerText || s.textContent || '').trim().toLowerCase();
                if (txt === 'follows you' || txt === 'mengikuti anda') {
                    return true;
                }
            }
            return false;
        })()
        "#;

        let res = cdp.eval_js(js).await?;
        Ok(res.as_bool() == Some(true))
    }

    /// Check if current account is already following this profile
    pub async fn check_is_following(cdp: &mut CdpSession) -> Result<bool> {
        let js = r#"
        (() => {
            const unfollowBtn = document.querySelector('[data-testid$="-unfollow"]');
            if (unfollowBtn) return true;
            const followBtn = document.querySelector('[data-testid$="-follow"]');
            if (followBtn) {
                const txt = (followBtn.innerText || '').toLowerCase();
                if (txt.includes('following') || txt.includes('mengikuti')) return true;
            }
            return false;
        })()
        "#;
        let res = cdp.eval_js(js).await?;
        Ok(res.as_bool() == Some(true))
    }

    /// Click Follow button on profile
    pub async fn click_follow(cdp: &mut CdpSession) -> Result<bool> {
        let js = r#"
        (() => {
            const followBtn = document.querySelector('[data-testid$="-follow"]');
            if (followBtn) {
                followBtn.click();
                return true;
            }
            return false;
        })()
        "#;
        let res = cdp.eval_js(js).await?;
        let clicked = res.as_bool() == Some(true);
        if clicked {
            sleep(Duration::from_millis(1500)).await;
        }
        Ok(clicked)
    }

    /// Click Unfollow button and confirm dialog on profile
    pub async fn click_unfollow(cdp: &mut CdpSession) -> Result<bool> {
        let js_unfollow = r#"
        (() => {
            const unfollowBtn = document.querySelector('[data-testid$="-unfollow"]');
            if (unfollowBtn) {
                unfollowBtn.click();
                return true;
            }
            return false;
        })()
        "#;

        let res = cdp.eval_js(js_unfollow).await?;
        if res.as_bool() != Some(true) {
            return Ok(false);
        }

        // Wait for confirmation modal
        sleep(Duration::from_millis(800)).await;

        let js_confirm = r#"
        (() => {
            const confirmBtn = document.querySelector('[data-testid="confirmationSheetConfirm"]');
            if (confirmBtn) {
                confirmBtn.click();
                return true;
            }
            return false;
        })()
        "#;

        let confirm_res = cdp.eval_js(js_confirm).await?;
        sleep(Duration::from_millis(1000)).await;
        Ok(confirm_res.as_bool() == Some(true))
    }

    /// Extract latest tweet from the profile timeline
    pub async fn extract_latest_tweet(cdp: &mut CdpSession) -> Result<Option<LatestTweetInfo>> {
        let js = r#"
        (() => {
            const articles = document.querySelectorAll('article[data-testid="tweet"]');
            if (articles.length === 0) return null;

            for (const article of articles) {
                // Find status link
                const links = Array.from(article.querySelectorAll('a[href*="/status/"]'));
                let tweetId = null;
                let tweetUrl = null;

                for (const link of links) {
                    const href = link.getAttribute('href') || '';
                    const match = href.match(/\/status\/(\d+)/);
                    if (match) {
                        tweetId = match[1];
                        tweetUrl = href.startsWith('http') ? href : ('https://x.com' + href);
                        break;
                    }
                }

                if (!tweetId) continue;

                // Extract tweet text
                const textEl = article.querySelector('[data-testid="tweetText"]');
                const text = textEl ? (textEl.innerText || textEl.textContent || '') : '';

                // Check if already liked
                const unlikeBtn = article.querySelector('[data-testid="unlike"]');
                const isLiked = unlikeBtn !== null;

                return {
                    tweet_id: tweetId,
                    tweet_url: tweetUrl,
                    text: text,
                    is_liked: isLiked
                };
            }

            return null;
        })()
        "#;

        let res = cdp.eval_js(js).await?;
        if res.is_null() {
            return Ok(None);
        }

        let info: LatestTweetInfo = serde_json::from_value(res)
            .context("Failed to deserialize tweet info")?;
        Ok(Some(info))
    }

    /// Like the latest tweet on profile
    pub async fn like_latest_tweet(cdp: &mut CdpSession) -> Result<bool> {
        let js = r#"
        (() => {
            const article = document.querySelector('article[data-testid="tweet"]');
            if (!article) return false;

            const likeBtn = article.querySelector('[data-testid="like"]');
            if (likeBtn) {
                likeBtn.scrollIntoView({ behavior: 'instant', block: 'center' });
                likeBtn.click();
                return true;
            }
            return false;
        })()
        "#;

        let res = cdp.eval_js(js).await?;
        let liked = res.as_bool() == Some(true);
        if liked {
            sleep(Duration::from_millis(1000)).await;
        }
        Ok(liked)
    }

    /// Reply to the latest tweet with specified text
    pub async fn reply_to_latest_tweet(cdp: &mut CdpSession, reply_text: &str) -> Result<bool> {
        // Step 1: Click reply button on the first tweet
        let js_click_reply = r#"
        (() => {
            const article = document.querySelector('article[data-testid="tweet"]');
            if (!article) return false;
            const replyBtn = article.querySelector('[data-testid="reply"]');
            if (replyBtn) {
                replyBtn.scrollIntoView({ behavior: 'instant', block: 'center' });
                replyBtn.click();
                return true;
            }
            return false;
        })()
        "#;

        let clicked = cdp.eval_js(js_click_reply).await?.as_bool() == Some(true);
        if !clicked {
            warn!("Could not find reply button on the latest tweet");
            return Ok(false);
        }

        // Step 2: Wait for reply modal/textarea
        sleep(Duration::from_millis(1200)).await;
        let textarea_sel = r#"[data-testid="tweetTextarea_0"]"#;
        if !cdp.wait_for_selector(textarea_sel, 5).await? {
            warn!("Reply textarea did not appear");
            return Ok(false);
        }

        // Step 3: Insert reply text
        cdp.type_into_selector(textarea_sel, reply_text).await?;
        sleep(Duration::from_millis(800)).await;

        // Step 4: Click Tweet/Reply button
        let js_send = r#"
        (() => {
            const btn = document.querySelector('[data-testid="tweetButton"]') ||
                        document.querySelector('[data-testid="tweetButtonInline"]');
            if (btn && !btn.disabled) {
                btn.click();
                return true;
            }
            return false;
        })()
        "#;

        let sent = cdp.eval_js(js_send).await?.as_bool() == Some(true);
        if sent {
            info!("Successfully sent reply: '{}'", reply_text);
            sleep(Duration::from_millis(2000)).await;
        } else {
            warn!("Send reply button was disabled or not found");
        }

        Ok(sent)
    }

    /// Scan sorsa.io directory/leaderboard page DOM for accounts with Sorsa Score > 500
    pub async fn scan_sorsa_io(cdp: &mut CdpSession) -> Result<Vec<DiscoveredAccount>> {
        let js = r#"
        (() => {
            const results = [];
            // Check for official Sorsa extension badge .score-badge
            const badges = document.querySelectorAll('.score-badge');
            for (const b of badges) {
                const handle = b.dataset.handle;
                const txt = b.innerText || '';
                const m = txt.match(/S:\s*([1-9]\d{1,4})/i);
                if (handle && m) {
                    const score = parseInt(m[1]);
                    if (score > 500 && !results.some(r => r.username === handle)) {
                        results.push({ username: handle, sorsa_score: score, source: 'sorsa.io' });
                    }
                }
            }

            // Strategy 2: tables or cards on sorsa.io
            const rows = document.querySelectorAll('tr, [data-row], [class*="leaderboard-item"], [class*="card"]');
            for (const row of rows) {
                const text = row.innerText || '';
                const handleMatch = text.match(/@([a-zA-Z0-9_]{1,20})/);
                const scoreMatch = text.match(/\b([1-9]\d{2,4})\b/);

                if (handleMatch && scoreMatch) {
                    const handle = handleMatch[1];
                    const score = parseInt(scoreMatch[1]);
                    if (score > 500 && !results.some(r => r.username === handle)) {
                        results.push({
                            username: handle,
                            sorsa_score: score,
                            source: 'sorsa.io'
                        });
                    }
                }
            }

            return results;
        })()
        "#;

        let res = cdp.eval_js(js).await?;
        let accounts: Vec<DiscoveredAccount> = serde_json::from_value(res).unwrap_or_default();
        Ok(accounts)
    }

    /// Scan X replies / timeline for users with Sorsa extension badges / scores > 500
    pub async fn scan_x_replies_timeline(cdp: &mut CdpSession) -> Result<Vec<DiscoveredAccount>> {
        let js = r#"
        (() => {
            const results = [];
            const articles = document.querySelectorAll('article[data-testid="tweet"]');

            for (const article of articles) {
                // Find author handle
                const userLink = article.querySelector('a[href*="/"] div[dir="ltr"] span');
                const authorText = userLink ? (userLink.innerText || '') : '';
                const handleMatch = authorText.match(/@([a-zA-Z0-9_]{1,20})/);
                if (!handleMatch) continue;
                const handle = handleMatch[1];

                // Sorsa Extension Official .score-badge
                let score = null;
                const badge = article.querySelector('.score-badge');
                if (badge) {
                    const txt = badge.innerText || '';
                    const m = txt.match(/S:\s*([1-9]\d{1,4})/i);
                    if (m) score = parseInt(m[1]);
                }

                // Fallback: check tweet text for Sorsa score pattern
                if (!score) {
                    const tweetTextEl = article.querySelector('[data-testid="tweetText"]');
                    const text = tweetTextEl ? (tweetTextEl.innerText || '') : '';
                    if (text.toLowerCase().includes('sorsa')) {
                        const m = text.match(/(?:sorsa\s*score|score)[:\s]*([1-9]\d{2,4})/i) || text.match(/\b([1-9]\d{2,4})\b/);
                        if (m) score = parseInt(m[1]);
                    }
                }

                if (score && score > 500 && !results.some(r => r.username === handle)) {
                    results.push({
                        username: handle,
                        sorsa_score: score,
                        source: 'x_feed'
                    });
                }
            }

            return results;
        })()
        "#;

        let res = cdp.eval_js(js).await?;
        let accounts: Vec<DiscoveredAccount> = serde_json::from_value(res).unwrap_or_default();
        Ok(accounts)
    }
}
