//! CLI entry points for headless modes.
//! - `claude-switchboard --tick`: run scheduler dispatcher for all eligible
//!   accounts and exit. Future Plan B tasks fill in the per-account walk;
//!   this stub validates the path.
//! - `claude-switchboard --migrate`: re-launch the GUI which re-runs
//!   migration idempotently.

use anyhow::Result;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliMode {
    Tick,
    Migrate,
    Statusline,
    Gui, // default — start the Tauri runtime as usual
}

pub fn parse_args<I, S>(args: I) -> CliMode
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    for a in args {
        match a.as_ref() {
            "--tick" => return CliMode::Tick,
            "--migrate" => return CliMode::Migrate,
            "statusline" => return CliMode::Statusline,
            _ => {}
        }
    }
    CliMode::Gui
}

/// Run `--tick`. Headless: open DB, attempt the per-account warm-up walk.
///
/// ## Trade-off: headless AppState reconstruction
///
/// The full `scheduler_glue::walk_due_accounts` path requires an `Arc<AppState>`,
/// which holds an `AccountManager`, `AuthOrchestrator`, `Arc<reqwest::Client>`,
/// and a per-slot snapshot cache (`cached_usage_by_slot`).
///
/// The snapshot cache starts empty for a headless tick — there is no running
/// poll loop to populate it. This means `five_hour.resets_at` will always be
/// `None` for a headless invocation, which is acceptable: the warm-up module
/// treats `None` as "window inactive", so it will issue the warm-up call
/// (correct behaviour for a launchd-driven pre-window fire).
///
/// Reconstructing the remaining AppState fields (AccountManager, Auth, HTTP
/// client) is straightforward but requires wiring them into this entry point
/// independently of `lib.rs::run()`, which builds them inside the Tauri
/// Builder setup closure. That refactor is deferred to a future task.
///
/// **For now** the in-app 30-second dispatcher (spawned in `lib.rs`) handles
/// all warm-up firing while the GUI is open. The launchd `--tick` path is a
/// documented no-op until the headless AppState reconstruction task lands.
pub async fn run_tick(data_dir: &Path) -> Result<()> {
    use crate::store::Db;

    let _db = Db::open_for_tick(data_dir)?;
    tracing::info!(
        "[--tick] headless AppState reconstruction not yet wired; \
         in-app dispatcher (lib.rs) handles warm-up while GUI is open"
    );
    Ok(())
}

/// Freshness window for the shared-snapshot file. A headless one-shot
/// invocation has no `Settings.polling_interval_secs` to read (no DB, no
/// AppState) — a fixed, generous constant errs toward "shows a number a
/// few minutes longer than strictly necessary after Switchboard quits"
/// rather than plumbing settings into a process that must stay fast and
/// simple (Claude Code invokes this on every prompt render).
///
/// The shared-snapshot file is only rewritten on a successful active-slot
/// poll, so its age tracks `Settings.polling_interval_secs`, which the UI
/// (`SettingsPanel.tsx`'s `POLL_MAX_SECS`) allows up to 1800s (30 minutes).
/// 2100s (35 minutes) stays safely past that ceiling with a 300s buffer, so
/// a user on the longest configurable interval never sees a false
/// "not running" between polls.
const STATUSLINE_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(2100);

/// Core logic for `run_statusline`, taking the shared-snapshot path
/// explicitly so it's testable without touching the real `~/.claude/`
/// directory. `run_statusline` calls this with the real path.
fn run_statusline_for_path(path: &Path) -> String {
    match crate::poll_loop::read_shared_snapshot(path, STATUSLINE_MAX_AGE, None) {
        Some(snap) => match snap.five_hour {
            // Clamp before rounding: a corrupt or malformed shared-snapshot
            // file could otherwise produce a nonsensical percentage.
            Some(u) => format!("5H {}%", u.utilization.clamp(0.0, 100.0).round() as i64),
            None => "Switchboard: not running".to_string(),
        },
        None => "Switchboard: not running".to_string(),
    }
}

/// Run `statusline`. Drains stdin (Claude Code pipes session-context JSON
/// in) without parsing it — out of scope for V1, which only shows 5H%.
/// Prints exactly one line to stdout and always exits 0: Claude Code
/// renders whatever this command prints, so erroring would show nothing
/// useful rather than the honest "not running" placeholder.
pub async fn run_statusline() {
    use std::io::{Read as _, Write as _};
    let mut discard = String::new();
    // Blocks until EOF with no timeout — relies on the caller (Claude Code)
    // closing stdin after writing the session-context JSON. Intentional per
    // spec; documented here since it's not obvious from the call alone.
    let _ = std::io::stdin().read_to_string(&mut discard);

    let path = crate::poll_loop::shared_usage_file_path();
    // Never panics even if stdout is closed/invalid — a broken pipe here
    // should not crash the process.
    let _ = writeln!(std::io::stdout(), "{}", run_statusline_for_path(&path));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tick_flag() {
        assert_eq!(
            parse_args(["claude-switchboard", "--tick"]),
            CliMode::Tick,
        );
    }

    #[test]
    fn parses_migrate_flag() {
        assert_eq!(
            parse_args(["claude-switchboard", "--migrate"]),
            CliMode::Migrate,
        );
    }

    #[test]
    fn defaults_to_gui_when_no_flag() {
        assert_eq!(parse_args(["claude-switchboard"]), CliMode::Gui);
    }

    #[test]
    fn ignores_other_args() {
        assert_eq!(
            parse_args(["claude-switchboard", "--unknown", "--tick"]),
            CliMode::Tick,
        );
    }

    #[test]
    fn parses_statusline_subcommand() {
        assert_eq!(
            parse_args(["claude-switchboard", "statusline"]),
            CliMode::Statusline,
        );
    }
}

#[cfg(test)]
mod statusline_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn prints_the_five_hour_percentage_when_the_snapshot_is_fresh() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("statusline-usage.json");
        let now = chrono::Utc::now().timestamp();
        std::fs::write(
            &p,
            format!(
                r#"{{"five_hour": {{"utilization": 42.5, "resets_at": null}}, "seven_day": null, "fetched_at": {now}}}"#
            ),
        )
        .unwrap();

        let line = run_statusline_for_path(&p);
        assert_eq!(line, "5H 43%");
    }

    #[test]
    fn reports_not_running_when_the_file_is_missing() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("does-not-exist.json");
        assert_eq!(run_statusline_for_path(&p), "Switchboard: not running");
    }

    #[test]
    fn reports_not_running_when_the_snapshot_is_stale() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("statusline-usage.json");
        // 2400s, comfortably past the 2100s STATUSLINE_MAX_AGE threshold.
        let old = (chrono::Utc::now() - chrono::Duration::minutes(40)).timestamp();
        std::fs::write(
            &p,
            format!(
                r#"{{"five_hour": {{"utilization": 42.5, "resets_at": null}}, "seven_day": null, "fetched_at": {old}}}"#
            ),
        )
        .unwrap();
        assert_eq!(run_statusline_for_path(&p), "Switchboard: not running");
    }

    #[test]
    fn reports_not_running_when_five_hour_is_absent() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("statusline-usage.json");
        let now = chrono::Utc::now().timestamp();
        std::fs::write(
            &p,
            format!(r#"{{"five_hour": null, "seven_day": null, "fetched_at": {now}}}"#),
        )
        .unwrap();
        assert_eq!(run_statusline_for_path(&p), "Switchboard: not running");
    }
}
