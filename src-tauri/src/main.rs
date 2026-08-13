// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match claude_switchboard_lib::cli::parse_args(&args) {
        claude_switchboard_lib::cli::CliMode::Tick => {
            // Initialize logging for headless tick.
            let log_dir = claude_switchboard_lib::logging::log_dir();
            let _log_guard = claude_switchboard_lib::logging::init(log_dir);

            let rt = tokio::runtime::Runtime::new().expect("tokio rt");
            let data_dir = claude_switchboard_lib::store::default_dir();
            if let Err(e) = rt.block_on(claude_switchboard_lib::cli::run_tick(&data_dir)) {
                eprintln!("--tick failed: {e:#}");
                std::process::exit(1);
            }
        }
        claude_switchboard_lib::cli::CliMode::Statusline => {
            let rt = tokio::runtime::Runtime::new().expect("tokio rt");
            rt.block_on(claude_switchboard_lib::cli::run_statusline());
        }
        claude_switchboard_lib::cli::CliMode::Migrate
        | claude_switchboard_lib::cli::CliMode::Gui => {
            claude_switchboard_lib::run();
        }
    }
}
