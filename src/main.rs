mod browser;
mod crm;
mod db;
mod dom;
mod extension;
mod models;
mod prompt;
mod report;
mod shortcut;
mod watcher;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use db::Database;
use extension::SorsaExtensionProvisioner;
use prompt::PromptEngine;
use report::CrmReporter;
use shortcut::{BrowserShortcutManager, BrowserType};
use std::path::PathBuf;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;
use watcher::WatcherDaemon;

#[derive(Parser, Debug)]
#[command(
    name = "sorsa-crm-watcher",
    about = "Local Social CRM & DOM State Tracker with Auto Sorsa Extension Provisioning, 48h FSM, and Golden Rules",
    version = "0.1.0"
)]
struct Cli {
    /// Remote debugging port to connect or check
    #[arg(short, long, default_value_t = 9222)]
    port: u16,

    /// Path to SQLite database file
    #[arg(short, long, default_value = "sorsa_crm.db")]
    db: String,

    /// Directory or path containing reply_prompt.md
    #[arg(long, default_value = ".")]
    prompt_dir: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the 5-minute Watcher Daemon (Interval 300s)
    Run {
        /// Watcher interval in seconds (default: 300 = 5 minutes)
        #[arg(short, long, default_value_t = 300)]
        interval: u64,

        /// Automatically ensure browser is launched with debugging port and Sorsa extension
        #[arg(long)]
        auto_launch: bool,

        /// Target browser to launch if auto_launch is enabled (chrome, brave, edge, firefox)
        #[arg(long, default_value = "brave")]
        browser: String,
    },

    /// Run a single CRM cycle (discovery, FSM 48h check, new post scan, and table report)
    Cycle,

    /// Print current ASCII CRM report table from SQLite database
    Report,

    /// Automatically provision Sorsa extension, generate debugging .lnk shortcut in current folder, and optionally launch browser
    AutoShortcut {
        /// Browser to generate shortcut for (chrome, brave, edge, firefox)
        #[arg(short, long, default_value = "brave")]
        browser: String,

        /// Custom port (default: 9222)
        #[arg(short, long, default_value_t = 9222)]
        port: u16,

        /// Automatically launch the browser process if port is not listening
        #[arg(short, long)]
        launch: bool,
    },

    /// Explicitly download or provision Sorsa extension into ./sorsa_extension
    InstallExtension,

    /// Detect and list all installed browsers on this Windows PC
    DetectBrowsers,

    /// Inspect an existing Windows .lnk shortcut file
    InspectShortcut {
        /// Path to .lnk file
        path: PathBuf,
    },

    /// Evaluate 48h trial deadlines and follback status
    FsmCheck,

    /// Show the active reply prompt template and test reply draft generation
    ShowPrompt {
        #[arg(long, default_value = "sorsa")]
        project: String,
        #[arg(long, default_value = "Excited to share our newest protocol milestone today!")]
        test_tweet: String,
    },

    /// Manually register a test account into TRIAL_48H (for testing FSM)
    AddTrial {
        username: String,
        #[arg(short, long, default_value_t = 650)]
        score: i64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_target(false)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .context("Failed to set tracing subscriber")?;

    let cli = Cli::parse();
    let db = Database::open(&cli.db)?;
    let prompt_engine = PromptEngine::load_from_dir(&cli.prompt_dir);

    match cli.command.unwrap_or(Commands::Report) {
        Commands::Run {
            interval,
            auto_launch,
            browser,
        } => {
            let workdir = PathBuf::from(".");
            if auto_launch {
                if let Some(b_type) = BrowserType::from_str(&browser) {
                    let _ = BrowserShortcutManager::ensure_browser_running(b_type, cli.port, &workdir);
                } else {
                    eprintln!("Unknown browser type: '{}'. Valid options: chrome, brave, edge, firefox", browser);
                }
            }

            let daemon = WatcherDaemon::new(&db, &prompt_engine, cli.port, interval);
            daemon.start_daemon().await?;
        }

        Commands::Cycle => {
            let daemon = WatcherDaemon::new(&db, &prompt_engine, cli.port, 300);
            daemon.run_single_cycle().await?;
        }

        Commands::Report => {
            let accounts = db.get_all_accounts()?;
            CrmReporter::print_report(&accounts);
        }

        Commands::AutoShortcut { browser, port, launch } => {
            let b_type = BrowserType::from_str(&browser)
                .with_context(|| format!("Unknown browser '{}'. Choose: chrome, brave, edge, firefox", browser))?;

            let workdir = PathBuf::from(".");
            println!("Detecting {} installation...", b_type.display_name());

            if let Some(path) = BrowserShortcutManager::detect_browser_path(b_type) {
                println!("Found: {:?}", path);
            } else {
                eprintln!("WARNING: Browser executable not found in standard paths or registry!");
            }

            if launch {
                println!("Ensuring Sorsa extension is provisioned and browser is running with debugging port {}...", port);
                let sc = BrowserShortcutManager::ensure_browser_running(b_type, port, &workdir)?;
                println!("Shortcut and session ready: {:?}", sc);
            } else {
                println!("Provisioning Sorsa extension and creating shortcut...");
                let sc = BrowserShortcutManager::create_shortcut(b_type, port, &workdir, None)?;
                println!("Generated debugging shortcut at: {:?}", sc);
                println!("Run this shortcut or launch with --launch to start debugging session.");
            }
        }

        Commands::InstallExtension => {
            let workdir = PathBuf::from(".");
            println!("Provisioning Sorsa Chrome extension (ID: fbjnlbncangbglgcinecoijkkeejcjan)...");
            let ext_dir = SorsaExtensionProvisioner::ensure_sorsa_extension(&workdir)?;
            println!("Sorsa extension ready at: {:?}", ext_dir);
        }

        Commands::DetectBrowsers => {
            println!("\n=== Detected Windows Browser Installations ===");
            let detected = BrowserShortcutManager::detect_all_installed();
            if detected.is_empty() {
                println!("No standard browser installations detected.");
            } else {
                for (b, p) in detected {
                    println!("- {}: {:?}", b.display_name(), p);
                }
            }
            println!();
        }

        Commands::InspectShortcut { path } => {
            println!("Inspecting shortcut: {:?}", path);
            let (target, args, work) = BrowserShortcutManager::inspect_shortcut(&path)?;
            println!("Target Path:       {}", target);
            println!("Arguments:         {}", args);
            println!("Working Directory: {}", work);
        }

        Commands::FsmCheck => {
            println!("Evaluating FSM status for all tracked accounts...");
            let daemon = WatcherDaemon::new(&db, &prompt_engine, cli.port, 300);
            daemon.run_single_cycle().await?;
        }

        Commands::ShowPrompt { project, test_tweet } => {
            let clean_proj = PromptEngine::clean_project_name(&project);
            println!("\n=== Active Reply Prompt Template ===");
            println!("{}", prompt_engine.get_template());
            println!("\n=== Generated 3-Option Draft Sample (Project: {}) ===", clean_proj);
            let (opt1, opt2, opt3, recommended) =
                prompt_engine.generate_draft_replies(&clean_proj, &test_tweet);
            println!("Option 1 (Discussion): {}", opt1);
            println!("Option 2:             {}", opt2);
            println!("Option 3:             {}", opt3);
            println!("Selected (No Question): {}\n", recommended);
        }

        Commands::AddTrial { username, score } => {
            let res = db.insert_trial_account(
                &username,
                score,
                "Manually registered for testing",
            )?;
            if res {
                println!("Account '@{}' added to TRIAL_48H with score {}.", username, score);
            } else {
                println!("Account '@{}' was already in database.", username);
            }
            let accounts = db.get_all_accounts()?;
            CrmReporter::print_report(&accounts);
        }
    }

    Ok(())
}
