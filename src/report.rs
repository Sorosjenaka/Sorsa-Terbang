use crate::models::{AccountRecord, AccountStatus};
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, Color, ContentArrangement, Table};

pub struct CrmReporter;

impl CrmReporter {
    pub fn render_table(accounts: &[AccountRecord]) -> String {
        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_content_arrangement(ContentArrangement::Dynamic);

        // Header
        table.set_header(vec![
            Cell::new("Username").fg(Color::Cyan),
            Cell::new("Sorsa Score").fg(Color::Cyan),
            Cell::new("Status").fg(Color::Cyan),
            Cell::new("Followed At").fg(Color::Cyan),
            Cell::new("Sisa Waktu Trial").fg(Color::Cyan),
            Cell::new("Jumlah Post Di-reply").fg(Color::Cyan),
            Cell::new("Aksi Terakhir").fg(Color::Cyan),
        ]);

        if accounts.is_empty() {
            table.add_row(vec![
                Cell::new("No accounts tracked yet"),
                Cell::new("-"),
                Cell::new("-"),
                Cell::new("-"),
                Cell::new("-"),
                Cell::new("-"),
                Cell::new("-"),
            ]);
            return table.to_string();
        }

        for acc in accounts {
            let status_cell = match acc.status {
                AccountStatus::Trial48h => Cell::new(acc.status.as_str()).fg(Color::Yellow),
                AccountStatus::MutualActive => Cell::new(acc.status.as_str()).fg(Color::Green),
                AccountStatus::DroppedBlacklist => Cell::new(acc.status.as_str()).fg(Color::Red),
            };

            let followed_at_str = acc.followed_at.format("%Y-%m-%d %H:%M").to_string();
            let trial_remaining = acc.trial_remaining_display();

            table.add_row(vec![
                Cell::new(format!("@{}", acc.username)),
                Cell::new(acc.sorsa_score.to_string()),
                status_cell,
                Cell::new(followed_at_str),
                Cell::new(trial_remaining),
                Cell::new(acc.replied_count.to_string()),
                Cell::new(&acc.last_action),
            ]);
        }

        table.to_string()
    }

    pub fn print_report(accounts: &[AccountRecord]) {
        let table_str = Self::render_table(accounts);
        println!("\n{}", table_str);
        println!(
            "Total Tracked: {} | Active Mutuals: {} | Trials: {} | Blacklisted: {}\n",
            accounts.len(),
            accounts.iter().filter(|a| a.status == AccountStatus::MutualActive).count(),
            accounts.iter().filter(|a| a.status == AccountStatus::Trial48h).count(),
            accounts.iter().filter(|a| a.status == AccountStatus::DroppedBlacklist).count(),
        );
    }
}
