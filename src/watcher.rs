use crate::browser::CdpSession;
use crate::crm::CrmEngine;
use crate::db::Database;
use crate::dom::DomParser;
use crate::prompt::PromptEngine;
use crate::report::CrmReporter;
use anyhow::Result;
use std::time::Duration;
use tokio::time::interval;
use tracing::{error, info, warn};

pub struct WatcherDaemon<'a> {
    db: &'a Database,
    prompt: &'a PromptEngine,
    port: u16,
    interval_secs: u64,
}

impl<'a> WatcherDaemon<'a> {
    pub fn new(db: &'a Database, prompt: &'a PromptEngine, port: u16, interval_secs: u64) -> Self {
        Self {
            db,
            prompt,
            port,
            interval_secs,
        }
    }

    /// Execute one complete cycle of:
    /// 1. Discovery on sorsa.io and X feed (find accounts with Sorsa score > 500)
    /// 2. Evaluate FSM (48h trial deadline & follback check)
    /// 3. Scan new posts for active trials & mutuals -> Like & Reply
    /// 4. Print CLI report table
    pub async fn run_single_cycle(&self) -> Result<()> {
        info!("--- Starting Watcher CRM Cycle ---");

        // Connect to browser tab via CDP
        let mut cdp = match CdpSession::connect(self.port, None).await {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    "Could not connect to browser CDP session on port {}: {}. Retrying next cycle...",
                    self.port, e
                );
                return Ok(());
            }
        };

        let crm = CrmEngine::new(self.db, self.prompt);

        // Phase 1: Discovery (sorsa.io & X replies)
        info!("[Watcher] Phase 1: Checking discovery channels...");
        // Check sorsa.io if open or navigate
        if let Ok(accounts) = DomParser::scan_sorsa_io(&mut cdp).await {
            for acc in accounts {
                if acc.sorsa_score > 500 {
                    let _ = crm
                        .process_discovered_account(&mut cdp, &acc.username, acc.sorsa_score)
                        .await;
                }
            }
        }

        // Phase 2: Evaluate FSM deadlines (48 hours) and follbacks
        info!("[Watcher] Phase 2: Evaluating 48h trials and follbacks...");
        if let Err(e) = crm.evaluate_fsm_cycle(&mut cdp).await {
            error!("Error during FSM evaluation: {}", e);
        }

        // Phase 3: Scan active accounts for new posts to Like and Reply
        info!("[Watcher] Phase 3: Checking new posts for active accounts...");
        if let Err(e) = crm.scan_and_interact_new_posts(&mut cdp).await {
            error!("Error scanning new posts: {}", e);
        }

        // Phase 4: Output ASCII report table
        let all_accounts = self.db.get_all_accounts()?;
        CrmReporter::print_report(&all_accounts);

        info!("--- Watcher CRM Cycle Completed ---");
        Ok(())
    }

    /// Background daemon running every 5 minutes (or custom interval)
    pub async fn start_daemon(&self) -> Result<()> {
        info!(
            "Starting Sorsa CRM Watcher Daemon (Interval: {}s = {} mins) on port {}...",
            self.interval_secs,
            self.interval_secs / 60,
            self.port
        );

        let mut timer = interval(Duration::from_secs(self.interval_secs));

        loop {
            tokio::select! {
                _ = timer.tick() => {
                    if let Err(e) = self.run_single_cycle().await {
                        error!("Cycle encountered an error: {}", e);
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    info!("Received Ctrl+C interrupt. Shutting down Sorsa CRM Watcher gracefully...");
                    break;
                }
            }
        }

        Ok(())
    }
}
