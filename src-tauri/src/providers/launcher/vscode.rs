//! Launching a provider-scoped session as a Claude Code tab inside VS Code.
//!
//! Everything here rests on behaviour verified on 2026-07-30 against VS Code
//! 1.131.0 and extension `anthropic.claude-code` 2.1.220:
//!
//! - `vscode://anthropic.claude-code/open?session=<id>` is handled by the
//!   extension's own URI handler, which resumes that session with `--resume`.
//! - VS Code forwards the environment of the *`code` invocation* to the window
//!   it opens — not the environment of the already-running VS Code process —
//!   and the extension spawns its CLI with `{...process.env}`. So a provider's
//!   env reaches the tab with nothing global mutated. Confirmed by reading the
//!   spawned CLI's environment block out of its PEB.
//! - The extension resolves cwd from the receiving window's first workspace
//!   folder, and its `createPanel` performs no workspace check. A URI landing
//!   in a window opened on some other folder therefore resumes in the wrong
//!   directory. We always open a NEW window on the session's own cwd so the
//!   window that receives the URI is the right one.
//! - A panel that already exists for a session id is revealed, not duplicated.
//!   That is what makes the retry schedule below safe.
//!
//! Two provider settings cannot cross this boundary, because the extension
//! builds its own argv: `extra_args` and `permission_mode`. Callers surface
//! that rather than letting it look like it applied.

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

/// The extension's publisher-qualified id, which is also the URI authority.
const EXTENSION_ID: &str = "anthropic.claude-code";

/// A fired URI can arrive before the window's extension host has activated, and
/// there is no readiness signal to wait on. Refiring is safe — the extension
/// reveals an existing panel for the same session id — so we retry on a
/// widening schedule instead of guessing one delay. A cold VS Code start took
/// ~20s to activate in testing; a warm one is near-instant.
pub const URI_RETRY_DELAYS_MS: [u64; 5] = [1_500, 3_000, 6_000, 12_000, 20_000];

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Where VS Code keeps third-party extensions. Insiders is a separate root and
/// a separate install, so both are probed.
fn extension_roots() -> Vec<PathBuf> {
    let Some(h) = home() else {
        return Vec::new();
    };
    vec![
        h.join(".vscode").join("extensions"),
        h.join(".vscode-insiders").join("extensions"),
    ]
}

/// The extension directory carries the version and platform in its name
/// (`anthropic.claude-code-2.1.220-win32-x64`), so this is a prefix match.
pub fn extension_installed() -> bool {
    extension_roots().iter().any(|root| {
        std::fs::read_dir(root)
            .map(|entries| {
                entries.flatten().any(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with(EXTENSION_ID)
                })
            })
            .unwrap_or(false)
    })
}

/// Resolve the `code` CLI. Kept separate from `is_available` so the reason a
/// launch is impossible can be reported precisely.
pub fn code_path() -> Option<PathBuf> {
    which::which("code").ok()
}

/// A VS Code tab is only offerable when both halves are present: the CLI to
/// open a window and fire the URI, and the extension to answer it.
pub fn is_available() -> bool {
    code_path().is_some() && extension_installed()
}

/// `(program, leading_args)` for invoking the `code` CLI.
///
/// On Windows `which` resolves `code` to `code.cmd`, and `CreateProcess` — which
/// is all `std::process::Command` does — cannot execute a batch file directly.
/// It has to go through `cmd /C`, so that wrapping is decided here rather than
/// at each call site.
pub fn code_invocation(code: &Path) -> (String, Vec<String>) {
    let is_batch = code
        .extension()
        .map(|e| {
            let e = e.to_string_lossy().to_lowercase();
            e == "cmd" || e == "bat"
        })
        .unwrap_or(false);
    if is_batch {
        (
            "cmd.exe".to_string(),
            vec!["/C".to_string(), code.to_string_lossy().to_string()],
        )
    } else {
        (code.to_string_lossy().to_string(), Vec::new())
    }
}

/// The deep link that opens (and optionally resumes into) a Claude Code tab.
///
/// `session` is omitted for a fresh session, which the extension answers with a
/// new conversation. Session ids are UUIDs, so no escaping is required; the
/// assertion in tests guards that assumption.
pub fn resume_uri(session_id: Option<&str>) -> String {
    match session_id {
        Some(id) => format!("vscode://{EXTENSION_ID}/open?session={id}"),
        None => format!("vscode://{EXTENSION_ID}/open"),
    }
}

/// Always `-n`: a fresh window is the only configuration where the receiving
/// window is guaranteed to be the one opened on this session's cwd.
pub fn open_window_args(cwd: &Path) -> Vec<String> {
    vec!["-n".to_string(), cwd.to_string_lossy().to_string()]
}

/// `--open-url` rather than the OS protocol handler: it goes straight to the
/// running instance instead of through a ShellExecute registration that may
/// point at a different install.
pub fn open_uri_args(uri: &str) -> Vec<String> {
    vec!["--open-url".to_string(), uri.to_string()]
}

/// Suppress the console flash from the `cmd /C` hop on Windows.
#[cfg(windows)]
fn hide_window(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_window(_cmd: &mut std::process::Command) {}

/// Opens a new VS Code window on `cwd` carrying `env`, then fires the URI that
/// turns it into a resumed Claude Code tab.
///
/// Returns the URI. The window spawn is synchronous so a missing `code` is
/// reported to the user; the URI fires from a background thread because the
/// window needs time to activate its extension host and nothing useful can be
/// awaited.
pub fn launch(
    cwd: &Path,
    env: &std::collections::BTreeMap<String, String>,
    session_id: Option<&str>,
    // Inherited provider variables to strip — see `launcher::stale_provider_keys`.
    // Without this the tab inherits whatever provider launched Switchboard, so
    // picking the official provider still lands on the third-party endpoint.
    stale_provider_keys: &[String],
) -> Result<String> {
    let code = code_path().ok_or_else(|| {
        anyhow!("could not find the `code` command on PATH — install the VS Code shell command via Command Palette → 'Shell Command: Install code command in PATH'")
    })?;
    if !extension_installed() {
        return Err(anyhow!(
            "the Claude Code extension for VS Code is not installed — install `{EXTENSION_ID}` and try again"
        ));
    }

    let (program, leading) = code_invocation(&code);

    let mut open = std::process::Command::new(&program);
    open.args(&leading).args(open_window_args(cwd));
    // Removals before additions, so a key the provider sets is set rather than
    // stripped.
    for k in stale_provider_keys {
        open.env_remove(k);
    }
    // The whole point of this path: the window inherits our per-launch env, and
    // the extension passes it to the CLI it spawns. Env, not argv — a command
    // line is world-readable via Task Manager and `ps`.
    for (k, v) in env {
        open.env(k, v);
    }
    hide_window(&mut open);
    open.spawn()
        .map_err(|e| anyhow!("spawn {program} to open a VS Code window: {e}"))?;

    let uri = resume_uri(session_id);
    let fire_uri = uri.clone();
    let fire_program = program.clone();
    let fire_leading = leading.clone();
    // A fresh session gets one shot: refiring a URI with no session id would
    // open another blank conversation each time, whereas refiring with one
    // reveals the panel already created.
    let attempts = if session_id.is_some() {
        URI_RETRY_DELAYS_MS.len()
    } else {
        1
    };
    std::thread::spawn(move || {
        for delay in URI_RETRY_DELAYS_MS.iter().take(attempts) {
            std::thread::sleep(std::time::Duration::from_millis(*delay));
            let mut c = std::process::Command::new(&fire_program);
            c.args(&fire_leading).args(open_uri_args(&fire_uri));
            hide_window(&mut c);
            let _ = c.spawn().map(|mut child| child.wait());
        }
    });

    Ok(uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_uri_targets_the_extension_authority_and_open_route() {
        let uri = resume_uri(Some("1fa5f520-5b60-4e0a-a373-4e4153d28f97"));
        assert_eq!(
            uri,
            "vscode://anthropic.claude-code/open?session=1fa5f520-5b60-4e0a-a373-4e4153d28f97"
        );
    }

    /// A session id reaches the extension verbatim, so it must never need
    /// escaping. The CLI's ids are UUIDs; this pins that expectation.
    #[test]
    fn resume_uri_needs_no_escaping_for_a_uuid() {
        let uri = resume_uri(Some("c78c06f0-b7a4-4052-bbd3-a01a6e82a3ac"));
        assert!(
            uri.chars().all(|c| c.is_ascii_alphanumeric()
                || matches!(c, ':' | '/' | '.' | '-' | '?' | '=')),
            "unexpected character needing escaping in {uri}"
        );
    }

    #[test]
    fn resume_uri_omits_the_session_for_a_fresh_conversation() {
        assert_eq!(resume_uri(None), "vscode://anthropic.claude-code/open");
    }

    /// The wrong-window bug class: opening in the current window would let the
    /// extension resolve cwd from whatever folder happened to be focused.
    #[test]
    fn a_window_is_always_opened_fresh_on_the_sessions_own_cwd() {
        let args = open_window_args(Path::new(r"C:\work\life-os"));
        assert_eq!(args, vec!["-n", r"C:\work\life-os"]);
    }

    #[test]
    fn uri_is_fired_through_the_cli_not_the_os_handler() {
        assert_eq!(
            open_uri_args("vscode://x/open"),
            vec!["--open-url", "vscode://x/open"]
        );
    }

    /// CreateProcess cannot execute a .cmd, and `which` resolves `code` to
    /// `code.cmd` on Windows — so a bare spawn fails with "program not found".
    #[test]
    fn a_batch_launcher_is_run_through_cmd() {
        let (prog, leading) = code_invocation(Path::new(r"C:\VS Code\bin\code.cmd"));
        assert_eq!(prog, "cmd.exe");
        assert_eq!(leading, vec!["/C", r"C:\VS Code\bin\code.cmd"]);
    }

    #[test]
    fn a_native_executable_is_run_directly() {
        let (prog, leading) = code_invocation(Path::new("/usr/local/bin/code"));
        assert_eq!(prog, "/usr/local/bin/code");
        assert!(leading.is_empty());
    }

    /// Retries must widen, and the first must not be so eager that it lands
    /// before the window exists at all.
    #[test]
    fn retry_schedule_is_strictly_increasing() {
        let d = URI_RETRY_DELAYS_MS;
        assert!(d[0] >= 1_000, "first retry must not race the window spawn");
        assert!(d.windows(2).all(|w| w[1] > w[0]), "delays must widen: {d:?}");
    }

    /// Availability needs both halves; a machine with the CLI but no extension
    /// would open a window that never answers the URI.
    #[test]
    fn availability_requires_cli_and_extension() {
        assert_eq!(is_available(), code_path().is_some() && extension_installed());
    }

    /// Opt-in smoke test for the whole chain, which no unit test can cover: it
    /// opens a real VS Code window and resumes a real session in it.
    ///
    /// Needs VS Code, the extension, and two env vars naming what to resume:
    /// ```text
    /// SWITCHBOARD_VSCODE_SMOKE_CWD=C:\path\to\project \
    /// SWITCHBOARD_VSCODE_SMOKE_SESSION=<uuid> \
    ///   cargo test --lib -- --ignored launches_a_real_vscode_tab --nocapture
    /// ```
    /// Then confirm the tab resumed by looking for `resume: <uuid>` in
    /// `%APPDATA%\Code\logs\*\window*\exthost\Anthropic.claude-code\`.
    #[test]
    #[ignore]
    fn launches_a_real_vscode_tab() {
        let Ok(cwd) = std::env::var("SWITCHBOARD_VSCODE_SMOKE_CWD") else {
            panic!("set SWITCHBOARD_VSCODE_SMOKE_CWD to a real project folder");
        };
        let session = std::env::var("SWITCHBOARD_VSCODE_SMOKE_SESSION").ok();
        let mut env = std::collections::BTreeMap::new();
        // Neutral marker rather than a real provider: this proves env reaches
        // the tab's CLI process without pointing it at another endpoint.
        env.insert("SWITCHBOARD_SMOKE".to_string(), "1".to_string());

        // Mirrors production: the same stale-key computation `launcher::launch`
        // performs, so any inherited provider variables are stripped here too.
        let stale = super::super::stale_provider_keys(&env);
        println!("stripping: {stale:?}");
        let uri = launch(Path::new(&cwd), &env, session.as_deref(), &stale).expect("launch");
        println!("fired: {uri}");
        // The URI retries run on a detached thread; hold the process open long
        // enough for the last one, or the test exits before it fires.
        let total: u64 = URI_RETRY_DELAYS_MS.iter().sum();
        std::thread::sleep(std::time::Duration::from_millis(total + 5_000));
    }

    /// Opt-in, because it needs VS Code installed:
    /// `cargo test -- --ignored vscode_cli_hop_executes`
    ///
    /// Worth having as a test rather than a one-off check. On Windows the CLI is
    /// `code.cmd` inside "Microsoft VS Code\bin", so this exercises both the
    /// `cmd /C` hop that `CreateProcess` requires for a batch file and the
    /// space-bearing path that hop is notorious for mangling.
    #[test]
    #[ignore]
    fn vscode_cli_hop_executes() {
        let code = code_path().expect("`code` must be on PATH for this test");
        let (program, leading) = code_invocation(&code);
        let out = std::process::Command::new(&program)
            .args(&leading)
            .arg("--version")
            .output()
            .expect("spawn the code CLI");
        assert!(
            out.status.success(),
            "status {:?}, stderr: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains('.'),
            "expected a version number, got {stdout:?}"
        );
    }
}
