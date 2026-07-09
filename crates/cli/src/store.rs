use coworker_core::config::{self, Config};
use coworker_core::error::Result;
use coworker_core::store;

use super::args::StoreCommands;
use super::terminal::{bold, progress_bar, table, use_color_stdout, yellow};

pub(crate) fn parse_storage_backend(name: &str) -> Result<config::StorageBackend> {
    use config::StorageBackend;
    match name.to_ascii_lowercase().as_str() {
        "json" => Ok(StorageBackend::Json),
        "sqlite" => Ok(StorageBackend::Sqlite),
        other => Err(coworker_core::error::CoworkerError::Config(format!(
            "unknown storage backend `{other}` (use json or sqlite)"
        ))),
    }
}

pub(crate) async fn run_store_cmd(config: Config, cmd: StoreCommands) -> Result<()> {
    use config::expand_tilde;
    use store::{compact, migrate, CompactOptions};

    match cmd {
        StoreCommands::Migrate {
            from,
            to,
            source,
            dest,
        } => {
            let from = parse_storage_backend(&from)?;
            let to = parse_storage_backend(&to)?;
            let source_path = expand_tilde(&source);
            let dest_path = expand_tilde(&dest);
            let stats =
                migrate(from, to, source_path, dest_path.clone(), config.storage.wal).await?;
            let tty = use_color_stdout();
            println!("{}", render_migrate_summary(&stats, tty));
            eprintln!(
                "Update coworker.yaml storage.backend to {:?} and storage.path to {}",
                to,
                dest_path.display()
            );
        }
        StoreCommands::Compact {
            audit_days,
            dry_run,
        } => {
            let opts = CompactOptions {
                audit_days,
                dry_run,
            };
            let path = config.storage_path();
            let stats = compact(
                config.storage.backend,
                path.clone(),
                config.storage.wal,
                &opts,
            )?;
            let tty = use_color_stdout();
            if dry_run {
                if tty {
                    eprintln!("{}", yellow("⚠ DRY-RUN — nothing was deleted.", true));
                } else {
                    eprintln!("DRY RUN — nothing was deleted.");
                }
                println!("{}", render_compact_summary(&stats, true, tty));
                eprintln!(
                    "(would compact {:?} store at {})",
                    config.storage.backend,
                    path.display()
                );
            } else {
                println!("{}", render_compact_summary(&stats, false, tty));
                eprintln!(
                    "compacted {:?} store at {}",
                    config.storage.backend,
                    path.display()
                );
            }
        }
    }
    Ok(())
}

/// Two-column, colored summary of a store migration (P0-3).
pub(crate) fn render_migrate_summary(stats: &store::MigrateStats, tty: bool) -> String {
    table(
        &["category", "count"],
        &[
            vec!["approvals".into(), stats.approvals.to_string()],
            vec!["backport_items".into(), stats.backport_items.to_string()],
            vec!["chat_messages".into(), stats.chat_messages.to_string()],
        ],
        tty,
    )
}

/// Two-column summary with a proportion bar per category (P0-3 + P1-2).
pub(crate) fn render_compact_summary(
    stats: &store::CompactStats,
    dry_run: bool,
    tty: bool,
) -> String {
    let rows = [
        ("audit entries", stats.audit_entries_removed),
        ("audit files", stats.audit_files_removed),
        ("legacy artifacts", stats.legacy_artifacts_removed),
        ("legacy workflow runs", stats.legacy_workflow_runs_removed),
    ];
    let max = rows.iter().map(|(_, n)| *n).max().unwrap_or(0).max(1);
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    for (label, n) in rows {
        let bar = if tty {
            progress_bar((n as f64 / max as f64) * 100.0, 12, true)
        } else {
            String::new()
        };
        table_rows.push(vec![label.into(), n.to_string(), bar]);
    }
    let title = if dry_run { "would remove" } else { "removed" };
    if tty {
        format!(
            "{}\n{}",
            bold(title, true),
            table(&["category", "count", "bar"], &table_rows, tty)
        )
    } else {
        table(&["category", "count"], &table_rows, tty)
    }
}
