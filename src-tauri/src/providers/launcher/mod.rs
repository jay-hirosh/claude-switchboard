//! Launches provider-scoped Claude Code sessions with per-process env.
//!
//! Nothing global is mutated, so several providers can run concurrently and
//! the user's own launch scripts keep working.
//!
//! Secrets are written into a user-only script file rather than passed as
//! process arguments — command lines are world-readable via `ps` on macOS
//! and Task Manager / WMI on Windows.

pub mod script;

use crate::providers::model::Provider;
use anyhow::{anyhow, Context, Result};
use script::ScriptFlavor;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Terminal {
    Ghostty,
    TerminalApp,
    Iterm2,
    Kitty,
    WezTerm,
    WindowsTerminal,
    PowerShell,
}

impl Terminal {
    pub fn label(self) -> &'static str {
        match self {
            Terminal::Ghostty => "Ghostty",
            Terminal::TerminalApp => "Terminal.app",
            Terminal::Iterm2 => "iTerm2",
            Terminal::Kitty => "kitty",
            Terminal::WezTerm => "WezTerm",
            Terminal::WindowsTerminal => "Windows Terminal",
            Terminal::PowerShell => "PowerShell",
        }
    }

    pub fn flavor(self) -> ScriptFlavor {
        match self {
            Terminal::WindowsTerminal | Terminal::PowerShell => ScriptFlavor::PowerShell,
            _ => ScriptFlavor::Sh,
        }
    }
}

pub struct LaunchSpec {
    pub provider: Provider,
    pub cwd: PathBuf,
    pub terminal: Terminal,
    pub resume_session_id: Option<String>,
}

#[cfg(target_os = "macos")]
pub fn default_terminal() -> Terminal {
    Terminal::Ghostty
}

/// `powershell.exe`, not Windows Terminal: `wt.exe` ships with Windows 11 but
/// is a Store install on Windows 10, so defaulting to it drops a large share
/// of Win10 users into the fallback on first use. `powershell.exe` (5.1) is
/// universally present. Never `pwsh` — PowerShell 7 is a separate install.
#[cfg(not(target_os = "macos"))]
pub fn default_terminal() -> Terminal {
    Terminal::PowerShell
}

#[cfg(target_os = "macos")]
const CANDIDATES: &[Terminal] = &[
    Terminal::Ghostty,
    Terminal::TerminalApp,
    Terminal::Iterm2,
    Terminal::Kitty,
    Terminal::WezTerm,
];

#[cfg(not(target_os = "macos"))]
const CANDIDATES: &[Terminal] = &[Terminal::PowerShell, Terminal::WindowsTerminal];

#[cfg(target_os = "macos")]
fn is_installed(t: Terminal) -> bool {
    let app = match t {
        Terminal::Ghostty => "Ghostty.app",
        Terminal::TerminalApp => "Terminal.app",
        Terminal::Iterm2 => "iTerm.app",
        Terminal::Kitty => "kitty.app",
        Terminal::WezTerm => "WezTerm.app",
        _ => return false,
    };
    Path::new("/Applications").join(app).exists()
        || dirs_home()
            .map(|h| h.join("Applications").join(app).exists())
            .unwrap_or(false)
}

#[cfg(not(target_os = "macos"))]
fn is_installed(t: Terminal) -> bool {
    let exe = match t {
        Terminal::WindowsTerminal => "wt.exe",
        Terminal::PowerShell => "powershell.exe",
        _ => return false,
    };
    which::which(exe).is_ok()
}

#[cfg(target_os = "macos")]
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Terminals actually present on this machine. Probed once at settings time
/// rather than at launch, so an unavailable choice surfaces before the user
/// has committed to a session.
pub fn available_terminals() -> Vec<Terminal> {
    CANDIDATES.iter().copied().filter(|t| is_installed(*t)).collect()
}

pub fn resolve_claude_binary() -> Result<PathBuf> {
    which::which("claude").map_err(|_| {
        anyhow!("could not find the `claude` executable on PATH — install Claude Code, or make sure it is on PATH for GUI apps")
    })
}

/// Generated scripts live under the app data dir, which is user-scoped on
/// both platforms (`~/Library/Application Support/...` and `%LOCALAPPDATA%`).
pub fn script_dir() -> PathBuf {
    crate::store::default_dir().join("launch")
}

pub fn write_script(spec: &LaunchSpec, dir: &Path) -> Result<PathBuf> {
    let claude = resolve_claude_binary()?;
    write_script_with_binary(spec, dir, &claude)
}

/// The body of `write_script` with the `claude` path supplied rather than
/// resolved. Split out so the permission and content assertions run on
/// machines without Claude Code installed — the `0700` mode is the C1
/// regression (a `0600` script fails to exec with 126 and an empty window),
/// and a test that silently skips would not have caught it.
pub fn write_script_with_binary(
    spec: &LaunchSpec,
    dir: &Path,
    claude: &Path,
) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).context("create launch script dir")?;
    let body = script::render(
        spec.terminal.flavor(),
        &spec.cwd.to_string_lossy(),
        &spec.provider.resolved_env(),
        &claude.to_string_lossy(),
        &spec.provider.extra_args,
        spec.resume_session_id.as_deref(),
    );
    let ext = match spec.terminal.flavor() {
        ScriptFlavor::Sh => "sh",
        ScriptFlavor::PowerShell => "ps1",
    };
    let path = dir.join(format!("{}.{}", uuid_v4(), ext));
    std::fs::write(&path, body).context("write launch script")?;

    // Owner-only read/write/execute. The exec bit is required because the
    // terminal runs the script directly rather than through an interpreter.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .context("restrict launch script permissions")?;
    }
    Ok(path)
}

/// `(program, args)` for spawning. Pure — asserted in tests without spawning.
pub fn build_command(terminal: Terminal, script: &Path, cwd: &Path) -> (String, Vec<String>) {
    let s = script.to_string_lossy().to_string();
    let d = cwd.to_string_lossy().to_string();
    match terminal {
        Terminal::Ghostty => (
            "open".into(),
            vec!["-na".into(), "Ghostty.app".into(), "--args".into(), "-e".into(), s],
        ),
        Terminal::TerminalApp => ("open".into(), vec!["-a".into(), "Terminal".into(), s]),
        Terminal::Iterm2 => ("open".into(), vec!["-a".into(), "iTerm".into(), s]),
        Terminal::Kitty => (
            "open".into(),
            vec!["-na".into(), "kitty.app".into(), "--args".into(), "-e".into(), s],
        ),
        Terminal::WezTerm => (
            "open".into(),
            vec![
                "-na".into(),
                "WezTerm.app".into(),
                "--args".into(),
                "start".into(),
                "--".into(),
                s,
            ],
        ),
        Terminal::WindowsTerminal => (
            "wt.exe".into(),
            vec![
                "-d".into(),
                d,
                "powershell.exe".into(),
                "-NoExit".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                s,
            ],
        ),
        Terminal::PowerShell => (
            "powershell.exe".into(),
            vec![
                "-NoExit".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                s,
            ],
        ),
    }
}

pub fn launch(spec: &LaunchSpec) -> Result<PathBuf> {
    let dir = script_dir();
    let script_path = write_script(spec, &dir)?;
    let (program, args) = build_command(spec.terminal, &script_path, &spec.cwd);
    std::process::Command::new(&program)
        .args(&args)
        .spawn()
        .with_context(|| format!("spawn {program} for {}", spec.terminal.label()))?;
    Ok(script_path)
}

/// Deletes scripts older than one hour. Called at app start **and on a
/// recurring timer** — never right after a launch, because the terminal reads
/// the file asynchronously and eager deletion races the spawn.
///
/// A startup-only sweep is insufficient: this is a menu-bar app that stays
/// resident for weeks, so token-bearing files would accumulate for the whole
/// uptime.
pub fn sweep_scripts(dir: &Path) -> Result<usize> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(0);
    };
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    let mut removed = 0;
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else { continue };
        if modified < cutoff && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

fn uuid_v4() -> String {
    // Matches the existing idiom in auth/oauth_paste_back.rs (rand 0.9).
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::rng().fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::model::ProviderKind;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    fn provider() -> Provider {
        Provider {
            id: "p1".into(),
            name: "GLM".into(),
            kind: ProviderKind::ThirdParty,
            base_url: Some("https://api.z.ai/api/anthropic".into()),
            auth_token: Some("tok".into()),
            env: BTreeMap::from([("ANTHROPIC_MODEL".to_string(), "glm-5.2".to_string())]),
            extra_args: vec!["--dangerously-skip-permissions".to_string()],
            preset_id: Some("glm".into()),
            sort_index: 1,
        }
    }

    #[test]
    fn ghostty_command_passes_script_after_dash_e() {
        let (prog, args) = build_command(
            Terminal::Ghostty,
            Path::new("/tmp/launch/a.sh"),
            Path::new("/work"),
        );
        assert_eq!(prog, "open");
        assert_eq!(
            args,
            vec!["-na", "Ghostty.app", "--args", "-e", "/tmp/launch/a.sh"]
        );
    }

    #[test]
    fn windows_terminal_command_sets_working_directory() {
        let (prog, args) = build_command(
            Terminal::WindowsTerminal,
            Path::new(r"C:\scripts\a.ps1"),
            Path::new(r"C:\work"),
        );
        assert_eq!(prog, "wt.exe");
        assert_eq!(args[0], "-d");
        assert_eq!(args[1], r"C:\work");
        assert_eq!(args.last().unwrap(), r"C:\scripts\a.ps1");
    }

    #[test]
    fn no_command_carries_a_secret_in_its_arguments() {
        let script = Path::new("/tmp/launch/a.sh");
        for t in [
            Terminal::Ghostty,
            Terminal::TerminalApp,
            Terminal::Iterm2,
            Terminal::Kitty,
            Terminal::WezTerm,
            Terminal::WindowsTerminal,
            Terminal::PowerShell,
        ] {
            let (prog, args) = build_command(t, script, Path::new("/work"));
            let joined = format!("{prog} {}", args.join(" "));
            assert!(!joined.contains("tok"), "{t:?} leaked a token into argv");
            assert!(!joined.contains("ANTHROPIC"), "{t:?} leaked env into argv");
        }
    }

    #[test]
    fn terminal_flavor_matches_platform_family() {
        assert_eq!(Terminal::Ghostty.flavor(), ScriptFlavor::Sh);
        assert_eq!(Terminal::WindowsTerminal.flavor(), ScriptFlavor::PowerShell);
        assert_eq!(Terminal::PowerShell.flavor(), ScriptFlavor::PowerShell);
    }

    #[test]
    fn sweep_removes_only_old_scripts() {
        let dir = tempdir().unwrap();
        let fresh = dir.path().join("fresh.sh");
        std::fs::write(&fresh, "x").unwrap();
        let stale = dir.path().join("stale.sh");
        std::fs::write(&stale, "x").unwrap();
        let two_hours_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        filetime::set_file_mtime(&stale, filetime::FileTime::from_system_time(two_hours_ago))
            .unwrap();

        let removed = sweep_scripts(dir.path()).unwrap();
        assert_eq!(removed, 1);
        assert!(fresh.exists(), "recent script must survive");
        assert!(!stale.exists(), "stale script must be removed");
    }

    #[test]
    fn sweep_on_missing_dir_is_not_an_error() {
        assert_eq!(sweep_scripts(Path::new("/definitely/not/here")).unwrap(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn written_script_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let spec = LaunchSpec {
            provider: provider(),
            cwd: PathBuf::from("/work"),
            terminal: Terminal::Ghostty,
            resume_session_id: None,
        };
        // The binary path is supplied rather than resolved, so this runs even
        // where Claude Code is not installed — see write_script_with_binary.
        let path =
            write_script_with_binary(&spec, dir.path(), Path::new("/opt/homebrew/bin/claude"))
                .expect("write launch script");

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "launch script must be owner-only");
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("export ANTHROPIC_MODEL='glm-5.2'"));
        assert!(body.contains("cd '/work' || exit 1"));
        assert!(body.contains("export ANTHROPIC_AUTH_TOKEN='tok'"));
        assert!(body.trim_end().ends_with(
            "exec '/opt/homebrew/bin/claude' '--dangerously-skip-permissions'"
        ));
    }

    /// C1: the terminal execs the script directly, so a script without the
    /// owner execute bit dies with exit 126 and an empty window.
    #[cfg(unix)]
    #[test]
    fn written_script_is_executable_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let spec = LaunchSpec {
            provider: provider(),
            cwd: PathBuf::from("/work"),
            terminal: Terminal::Ghostty,
            resume_session_id: None,
        };
        let path = write_script_with_binary(&spec, dir.path(), Path::new("/usr/bin/true")).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o100, 0o100, "owner execute bit is required to exec");
        assert_eq!(mode & 0o077, 0, "group and other must have no access");

        // Prove it: run the file as a command, not through an interpreter.
        let status = std::process::Command::new(&path)
            .status()
            .expect("script must be directly executable");
        assert_ne!(
            status.code(),
            Some(126),
            "exit 126 means the permission bits are wrong"
        );
    }
}
