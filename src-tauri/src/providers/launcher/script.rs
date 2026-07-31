use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptFlavor {
    Sh,
    PowerShell,
}

/// POSIX single-quote quoting. Everything inside `'…'` is literal, so the
/// only escape needed is for `'` itself: close the quote, emit an escaped
/// quote, reopen. Handles `$`, backticks, newlines and `"` for free.
pub fn quote_sh(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// PowerShell single-quote quoting. Inside `'…'` no expansion occurs and a
/// literal quote is written by doubling it.
pub fn quote_ps(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Session-identity markers Claude Code stamps into every process it spawns,
/// which must be cleared before starting an unrelated session.
///
/// Switchboard can itself be launched from inside a Claude Code session (a
/// `pnpm tauri dev` run started from the CLI is the common case). The whole
/// marker set is then in the app's own environment, `Command::spawn`
/// inherits it, and the terminal passes it to `claude` — which sees
/// `CLAUDE_CODE_CHILD_SESSION` and silently disables transcript saving:
///
///   ⚠ Transcript saving is off — inherited CLAUDE_CODE_CHILD_SESSION marker
///
/// A session with no transcript never appears in the Sessions tab and cannot
/// be resumed, so the feature quietly breaks itself.
///
/// The list is the union of the two sets Claude Code itself maintains: what
/// it injects into children (`CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`,
/// `CLAUDE_CODE_CHILD_SESSION`, `CLAUDE_PID`, plus `AI_AGENT`,
/// `CLAUDE_EFFORT` and `TRACEPARENT` when set) and what it deletes when
/// spawning a detached session (the same four plus
/// `CLAUDE_CODE_BRIDGE_SESSION_ID` and `CLAUDE_BG_AUTH_SNAPSHOT_PATH`).
///
/// Unsetting is deliberately preferred over `CLAUDE_CODE_FORCE_SESSION_PERSISTENCE=1`,
/// which suppresses the warning while leaving the session still marked as
/// somebody else's child.
const SESSION_MARKERS: [&str; 9] = [
    "CLAUDECODE",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_BRIDGE_SESSION_ID",
    "CLAUDE_BG_AUTH_SNAPSHOT_PATH",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
    "AI_AGENT",
    "TRACEPARENT",
];

/// Colour-suppression variables Claude Code exports into every process it
/// spawns, so that tool output it reads back is not full of escape codes.
///
/// They are correct for a captured subprocess and wrong for an interactive
/// terminal. Switchboard started from inside a session (`pnpm tauri dev` run
/// from the CLI) inherits `NO_COLOR=1`, passes it down the whole chain —
/// app → terminal → `claude` — and the launched session renders totally
/// unstyled: no syntax highlighting, no diff colours, no dimmed chrome.
///
/// Cleared only when Switchboard is itself inside a session, because
/// `NO_COLOR` is a documented cross-tool standard: a user who exports it
/// deliberately means it, and a launcher must not quietly override that.
/// Inheriting it from a parent agent is not the same as choosing it.
const COLOR_SUPPRESSORS: [&str; 2] = ["NO_COLOR", "FORCE_COLOR"];

/// An empty environment, so `ScriptSpec` can have a `Default` and call sites can
/// name only the fields they care about.
static NO_ENV: std::sync::LazyLock<BTreeMap<String, String>> =
    std::sync::LazyLock::new(BTreeMap::new);

/// Everything that varies between generated scripts.
///
/// A struct rather than nine positional parameters: most call sites care about
/// two or three of these, and `..Default::default()` lets them say so. As
/// arguments they were also trivially transposable — `resume` and
/// `permission_mode` are both `Option<&str>`, and swapping them would have
/// compiled and produced a plausible-looking wrong script.
#[derive(Debug, Clone)]
pub struct ScriptSpec<'a> {
    pub flavor: ScriptFlavor,
    pub cwd: &'a str,
    pub env: &'a BTreeMap<String, String>,
    pub claude_path: &'a str,
    pub extra_args: &'a [String],
    pub resume: Option<&'a str>,
    pub permission_mode: Option<&'a str>,
    pub clear_inherited_color: bool,
    /// Provider variables present in Switchboard's own environment that the
    /// chosen provider does not set, and so would otherwise leak into the
    /// session. See `launcher::stale_provider_keys`.
    pub stale_provider_keys: &'a [String],
}

impl Default for ScriptSpec<'_> {
    fn default() -> Self {
        Self {
            flavor: ScriptFlavor::Sh,
            cwd: "",
            env: &NO_ENV,
            claude_path: "",
            extra_args: &[],
            resume: None,
            permission_mode: None,
            clear_inherited_color: false,
            stale_provider_keys: &[],
        }
    }
}

pub fn render(spec: &ScriptSpec) -> String {
    let &ScriptSpec {
        flavor,
        cwd,
        env,
        claude_path,
        extra_args,
        resume,
        permission_mode,
        clear_inherited_color,
        stale_provider_keys,
    } = spec;

    let q: fn(&str) -> String = match flavor {
        ScriptFlavor::Sh => quote_sh,
        ScriptFlavor::PowerShell => quote_ps,
    };

    let mut s = String::new();
    match flavor {
        ScriptFlavor::Sh => {
            s.push_str("#!/bin/sh\n# Generated by Claude Switchboard. Safe to delete.\n");
            s.push_str(&format!("cd {} || exit 1\n", q(cwd)));
            // Before the provider's own exports: this is a new session, not a
            // child of whatever process happened to start Switchboard.
            s.push_str("# Start a session of our own, not a child of this process.\n");
            for k in SESSION_MARKERS {
                s.push_str(&format!("unset {k}\n"));
            }
            if clear_inherited_color {
                for k in COLOR_SUPPRESSORS {
                    s.push_str(&format!("unset {k}\n"));
                }
            }
            if !stale_provider_keys.is_empty() {
                s.push_str("# Another provider's variables, inherited rather than chosen.\n");
                for k in stale_provider_keys {
                    s.push_str(&format!("unset {k}\n"));
                }
            }
            for (k, v) in env {
                s.push_str(&format!("export {}={}\n", k, q(v)));
            }
            s.push_str(&format!("exec {}", q(claude_path)));
        }
        ScriptFlavor::PowerShell => {
            s.push_str("# Generated by Claude Switchboard. Safe to delete.\n");
            s.push_str(&format!("Set-Location {}\n", q(cwd)));
            s.push_str("# Start a session of our own, not a child of this process.\n");
            for k in SESSION_MARKERS {
                // Remove-Item on a non-existent Env: entry is an error, so
                // -ErrorAction SilentlyContinue is required, not decorative.
                s.push_str(&format!(
                    "Remove-Item Env:\\{k} -ErrorAction SilentlyContinue\n"
                ));
            }
            if clear_inherited_color {
                for k in COLOR_SUPPRESSORS {
                    s.push_str(&format!(
                        "Remove-Item Env:\\{k} -ErrorAction SilentlyContinue\n"
                    ));
                }
            }
            if !stale_provider_keys.is_empty() {
                s.push_str("# Another provider's variables, inherited rather than chosen.\n");
                for k in stale_provider_keys {
                    s.push_str(&format!(
                        "Remove-Item Env:\\{k} -ErrorAction SilentlyContinue\n"
                    ));
                }
            }
            for (k, v) in env {
                s.push_str(&format!("$env:{} = {}\n", k, q(v)));
            }
            s.push_str(&format!("& {}", q(claude_path)));
        }
    }

    // Provider-supplied flags, each quoted individually — never joined into a
    // single string, which would make one argument out of several.
    for a in extra_args {
        s.push(' ');
        s.push_str(&q(a));
    }

    // `--fork-session` is not optional (Spec B §7.1): resuming a session that
    // is still open elsewhere would otherwise put two Claude Code processes on
    // the same transcript file, corrupting history that cannot be rebuilt.
    if let Some(id) = resume {
        s.push_str(&format!(" --resume {} --fork-session", q(id)));
    }

    // Restores how the original session behaved: a session run under
    // `bypassPermissions` that comes back asking to approve every edit has not
    // really been resumed.
    //
    // Skipped when the provider already names a permission mode. That is
    // standing configuration the user typed, and it must not be silently
    // overridden by a mode inferred from history — nor should correctness here
    // rest on which of two duplicate flags the CLI's parser happens to prefer.
    if let Some(mode) = permission_mode {
        let provider_sets_mode = extra_args.iter().any(|a| {
            a == "--permission-mode"
                || a.starts_with("--permission-mode=")
                || a == "--dangerously-skip-permissions"
        });
        if !provider_sets_mode {
            s.push_str(&format!(" --permission-mode {}", q(mode)));
        }
    }

    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use tempfile::tempdir;

    fn env() -> BTreeMap<String, String> {
        BTreeMap::from([
            (
                "ANTHROPIC_BASE_URL".to_string(),
                "https://api.z.ai/api/anthropic".to_string(),
            ),
            ("ANTHROPIC_MODEL".to_string(), "glm-5.2".to_string()),
        ])
    }

    #[test]
    fn sh_quoting_neutralizes_single_quotes() {
        assert_eq!(quote_sh("ab'cd"), r"'ab'\''cd'");
    }

    #[test]
    fn sh_quoting_leaves_shell_metacharacters_inert() {
        let q = quote_sh("$(rm -rf /) `whoami` \"x\" ; echo pwned");
        assert!(q.starts_with('\'') && q.ends_with('\''));
        assert!(!q.contains(r"'\''"), "no quote-breaks expected in this input");
    }

    #[test]
    fn ps_quoting_doubles_single_quotes() {
        assert_eq!(quote_ps("ab'cd"), "'ab''cd'");
    }

    /// The official provider sets no env, so an add-only script left an
    /// inherited third-party endpoint in place and the session ran on it.
    #[test]
    fn sh_script_unsets_inherited_provider_keys_before_exporting() {
        let stale = vec![
            "ANTHROPIC_BASE_URL".to_string(),
            "ANTHROPIC_MODEL".to_string(),
        ];
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &BTreeMap::new(),
            claude_path: "/usr/bin/claude",
            stale_provider_keys: &stale,
            ..Default::default()
        });
        assert!(s.contains("unset ANTHROPIC_BASE_URL\n"), "{s}");
        assert!(s.contains("unset ANTHROPIC_MODEL\n"), "{s}");
    }

    /// Order is load-bearing: an unset after the export would wipe the very
    /// endpoint the launch is supposed to use.
    #[test]
    fn ps_script_removes_inherited_keys_before_setting_the_providers_own() {
        let stale = vec!["ANTHROPIC_MODEL".to_string()];
        let env = BTreeMap::from([(
            "ANTHROPIC_BASE_URL".to_string(),
            "https://api.kimi.com/coding".to_string(),
        )]);
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::PowerShell,
            cwd: r"C:\w",
            env: &env,
            claude_path: r"C:\claude.exe",
            stale_provider_keys: &stale,
            ..Default::default()
        });
        let removed = s.find("Remove-Item Env:\\ANTHROPIC_MODEL").expect("removal");
        let set = s.find("$env:ANTHROPIC_BASE_URL").expect("export");
        assert!(removed < set, "removals must precede exports:\n{s}");
    }

    /// Nothing inherited means no cleanup block at all, rather than a comment
    /// header with nothing under it.
    #[test]
    fn no_cleanup_block_when_nothing_was_inherited() {
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &env(),
            claude_path: "/usr/bin/claude",
            ..Default::default()
        });
        assert!(!s.contains("inherited rather than chosen"), "{s}");
    }

    #[test]
    fn sh_script_has_shebang_cd_exports_and_exec() {
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp/my project",
            env: &env(),
            claude_path: "/opt/homebrew/bin/claude",
            ..Default::default()
        });
        assert!(s.starts_with("#!/bin/sh\n"));
        assert!(s.contains("cd '/tmp/my project' || exit 1"));
        assert!(s.contains("export ANTHROPIC_MODEL='glm-5.2'"));
        assert!(s.trim_end().ends_with("exec '/opt/homebrew/bin/claude'"));
    }

    #[test]
    fn sh_script_appends_resume_flag() {
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &env(),
            claude_path: "/usr/bin/claude",
            resume: Some("57ca2089-1111"),
            ..Default::default()
        });
        assert!(s
            .trim_end()
            .ends_with("exec '/usr/bin/claude' --resume '57ca2089-1111' --fork-session"));
    }

    #[test]
    fn ps_script_sets_env_and_invokes_claude() {
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::PowerShell,
            cwd: r"C:\Users\me\my project",
            env: &env(),
            claude_path: r"C:\Program Files\claude.exe",
            ..Default::default()
        });
        assert!(s.contains(r"Set-Location 'C:\Users\me\my project'"));
        assert!(s.contains("$env:ANTHROPIC_MODEL = 'glm-5.2'"));
        assert!(s.trim_end().ends_with(r"& 'C:\Program Files\claude.exe'"));
    }

    /// Switchboard is often started from inside a Claude Code session, so its
    /// own environment carries that session's identity markers. Passing them
    /// through makes `claude` treat the new session as a child and disable
    /// transcript saving entirely — the session then never appears in the
    /// Sessions tab and cannot be resumed. It fails with a one-line warning,
    /// not an error, which is exactly the kind of thing a test has to catch.
    #[test]
    fn sh_script_clears_inherited_session_markers_before_exporting() {
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &env(),
            claude_path: "/usr/bin/claude",
            ..Default::default()
        });
        for k in SESSION_MARKERS {
            assert!(
                s.contains(&format!("unset {k}\n")),
                "{k} must be unset; leaving it disables transcript saving"
            );
        }
        // Ordering matters: an unset after the provider's exports would undo
        // them if the two sets ever overlapped.
        let unset_at = s.rfind("unset ").expect("unsets present");
        let export_at = s.find("export ANTHROPIC_").expect("exports present");
        assert!(unset_at < export_at, "unsets must precede exports:\n{s}");
    }

    #[test]
    fn ps_script_clears_inherited_session_markers_before_assigning() {
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::PowerShell,
            cwd: r"C:\work",
            env: &env(),
            claude_path: r"C:\claude.exe",
            ..Default::default()
        });
        for k in SESSION_MARKERS {
            assert!(
                s.contains(&format!("Remove-Item Env:\\{k} -ErrorAction SilentlyContinue\n")),
                "{k} must be removed, and silently — Remove-Item errors on a missing entry"
            );
        }
        let remove_at = s.rfind("Remove-Item ").expect("removals present");
        let assign_at = s.find("$env:ANTHROPIC_").expect("assignments present");
        assert!(remove_at < assign_at, "removals must precede assignments:\n{s}");
    }

    /// The generated script must actually clear the markers when run, not
    /// merely contain the right text.
    #[cfg(unix)]
    #[test]
    fn a_launched_session_does_not_inherit_a_child_session_marker() {
        let d = tempdir().unwrap();
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &env(),
            claude_path: "/bin/sh",
            extra_args: &[
                "-c".to_string(),
                "printf '[%s][%s]' \"$CLAUDE_CODE_CHILD_SESSION\" \"$CLAUDECODE\"".to_string(),
            ],
            ..Default::default()
        });
        let script = d.path().join("t.sh");
        std::fs::write(&script, &s).unwrap();

        let out = std::process::Command::new("/bin/sh")
            .arg(&script)
            // Exactly the situation this guards: the parent is a Claude Code
            // session, so the markers are live in the inherited environment.
            .env("CLAUDE_CODE_CHILD_SESSION", "1")
            .env("CLAUDECODE", "1")
            .env("CLAUDE_CODE_SESSION_ID", "029a3e04-fa36")
            .output()
            .expect("run generated script");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "[][]",
            "markers leaked into the launched session:\n{s}"
        );
    }

    #[test]
    fn injection_attempt_in_token_stays_inside_the_string() {
        let mut e = env();
        e.insert(
            "ANTHROPIC_AUTH_TOKEN".to_string(),
            "x'; rm -rf ~; echo '".to_string(),
        );
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &e,
            claude_path: "/usr/bin/claude",
            ..Default::default()
        });
        // The dangerous text must appear only inside a quoted export line.
        let line = s
            .lines()
            .find(|l| l.starts_with("export ANTHROPIC_AUTH_TOKEN="))
            .unwrap();
        assert_eq!(
            line,
            r"export ANTHROPIC_AUTH_TOKEN='x'\''; rm -rf ~; echo '\'''"
        );
        assert!(!s.contains("\nrm -rf"), "payload must never start its own line");
    }

    /// Spec §3.1 — a newline in any interpolated value must not be able to
    /// become a live command. The header comment is a fixed string, so a
    /// provider name cannot reach the script at all; this guards the values
    /// that do.
    ///
    /// Asserted by *executing* the rendered script rather than by scanning it
    /// for textual line starts. POSIX single-quoting deliberately lets a value
    /// span textual lines — `export X='a⏎rm -rf y⏎#'` is one word to `/bin/sh`
    /// and the `rm` never runs. A line-start scan would therefore fail on
    /// correct output while proving nothing about what the shell does; only
    /// running it settles the question.
    #[cfg(unix)]
    #[test]
    fn newlines_in_values_cannot_introduce_a_script_line() {
        let d = tempdir().unwrap();
        let canary = d.path().join("CANARY");
        std::fs::write(&canary, "x").unwrap();
        let payload = format!("glm\nrm -f {}\n#", canary.display());

        let mut e = env();
        e.insert("ANTHROPIC_MODEL".to_string(), payload.clone());
        // `true` stands in for the claude binary so the script terminates.
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &e,
            claude_path: "/usr/bin/true",
            ..Default::default()
        });

        let script = d.path().join("t.sh");
        std::fs::write(&script, &s).unwrap();
        let out = std::process::Command::new("/bin/sh")
            .arg(&script)
            .output()
            .expect("run generated script");
        assert!(out.status.success(), "generated script must be valid sh: {s}");
        assert!(
            canary.exists(),
            "the newline payload executed as a command — quoting failed:\n{s}"
        );

        // And the value survives intact rather than being truncated at the
        // newline, which would silently misconfigure the session.
        let echo = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &e,
            claude_path: "/bin/sh",
            extra_args: &["-c".to_string(), "printf %s \"$ANTHROPIC_MODEL\"".to_string()],
            ..Default::default()
        });
        let script2 = d.path().join("t2.sh");
        std::fs::write(&script2, &echo).unwrap();
        let out2 = std::process::Command::new("/bin/sh")
            .arg(&script2)
            .output()
            .expect("run generated script");
        assert_eq!(String::from_utf8_lossy(&out2.stdout), payload);

        // Same for the working directory: a newline in `cwd` must not escape
        // its quoting either.
        let canary2 = d.path().join("CANARY2");
        std::fs::write(&canary2, "x").unwrap();
        let s2 = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: &format!("/tmp\nrm -f {}\n", canary2.display()),
            env: &env(),
            claude_path: "/usr/bin/true",
            ..Default::default()
        });
        let script3 = d.path().join("t3.sh");
        std::fs::write(&script3, &s2).unwrap();
        // `cd` to a bogus path exits 1 by design; what matters is that the
        // payload did not run as its own command.
        let _ = std::process::Command::new("/bin/sh").arg(&script3).output();
        assert!(canary2.exists(), "cwd escaped its quoting:\n{s2}");
    }

    #[test]
    fn generated_script_never_interpolates_a_provider_name() {
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &env(),
            claude_path: "/usr/bin/claude",
            ..Default::default()
        });
        assert!(
            s.contains("# Generated by Claude Switchboard. Safe to delete."),
            "header must be a fixed string, not built from provider data"
        );
    }

    /// The generated script execs the binary directly, bypassing any shell
    /// `claude()` wrapper — so flags the user relies on must be reproduced.
    #[test]
    fn extra_args_are_appended_and_quoted_individually() {
        let args = vec![
            "--dangerously-skip-permissions".to_string(),
            "--append-system-prompt".to_string(),
            "be terse; don't stop".to_string(),
        ];
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &env(),
            claude_path: "/usr/bin/claude",
            extra_args: &args,
            ..Default::default()
        });
        assert!(
            s.trim_end().ends_with(
                "exec '/usr/bin/claude' '--dangerously-skip-permissions' '--append-system-prompt' 'be terse; don'\\''t stop'"
            ),
            "got: {}",
            s.trim_end()
        );
    }

    #[test]
    fn resume_always_forks_the_session() {
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &env(),
            claude_path: "/usr/bin/claude",
            resume: Some("abc-123"),
            ..Default::default()
        });
        assert!(
            s.contains("--fork-session"),
            "resuming without --fork-session lets two processes share one transcript"
        );
        let ps = render(&ScriptSpec {
            flavor: ScriptFlavor::PowerShell,
            cwd: "C:\\w",
            env: &env(),
            claude_path: "claude.exe",
            resume: Some("abc-123"),
            ..Default::default()
        });
        assert!(ps.contains("--fork-session"), "PowerShell path must fork too");
    }

    #[test]
    fn no_fork_flag_when_not_resuming() {
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &env(),
            claude_path: "/usr/bin/claude",
            ..Default::default()
        });
        assert!(!s.contains("--fork-session"));
        assert!(!s.contains("--resume"));
    }

    /// A session run under `bypassPermissions` that comes back asking to
    /// approve every edit has not really been resumed.
    #[test]
    fn resume_carries_the_sessions_permission_mode() {
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &env(),
            claude_path: "/usr/bin/claude",
            resume: Some("abc-123"),
            permission_mode: Some("bypassPermissions"),
            ..Default::default()
        });
        assert!(
            s.trim_end().ends_with(
                "--resume 'abc-123' --fork-session --permission-mode 'bypassPermissions'"
            ),
            "got: {}",
            s.trim_end()
        );

        let ps = render(&ScriptSpec {
            flavor: ScriptFlavor::PowerShell,
            cwd: r"C:\w",
            env: &env(),
            claude_path: "claude.exe",
            resume: Some("abc-123"),
            permission_mode: Some("bypassPermissions"),
            ..Default::default()
        });
        assert!(ps.contains("--permission-mode 'bypassPermissions'"));
    }

    #[test]
    fn no_permission_flag_when_the_session_recorded_none() {
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &env(),
            claude_path: "/usr/bin/claude",
            ..Default::default()
        });
        assert!(!s.contains("--permission-mode"));
    }

    /// The provider's own flags are standing configuration the user typed. A
    /// mode inferred from transcript history must not silently override them,
    /// and the outcome must not depend on which duplicate flag the CLI's
    /// parser happens to prefer.
    #[test]
    fn provider_permission_flags_win_over_the_inferred_mode() {
        for provider_flag in [
            "--dangerously-skip-permissions",
            "--permission-mode",
            "--permission-mode=plan",
        ] {
            let args = vec![provider_flag.to_string()];
            let s = render(&ScriptSpec {
                flavor: ScriptFlavor::Sh,
                cwd: "/tmp",
                env: &env(),
                claude_path: "/usr/bin/claude",
                extra_args: &args,
                resume: Some("abc-123"),
                permission_mode: Some("bypassPermissions"),
                ..Default::default()
            });
            assert!(
                !s.contains("--permission-mode 'bypassPermissions'"),
                "{provider_flag} must not be overridden; got: {}",
                s.trim_end()
            );
        }
    }

    /// The mode reaches the script as a quoted argument like any other, so a
    /// hostile value cannot break out.
    #[test]
    fn permission_mode_is_quoted_like_every_other_value() {
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &env(),
            claude_path: "/usr/bin/claude",
            permission_mode: Some("x'; rm -rf ~; echo '"),
            ..Default::default()
        });
        assert!(s.contains(r"--permission-mode 'x'\''; rm -rf ~; echo '\'''"));
        assert!(!s.contains("\nrm -rf"));
    }

    /// Switchboard run from inside a session inherits `NO_COLOR=1`, and
    /// passing it down renders the launched session completely unstyled.
    #[test]
    fn inherited_color_suppression_is_cleared_when_inside_a_session() {
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &env(),
            claude_path: "/usr/bin/claude",
            clear_inherited_color: true,
            ..Default::default()
        });
        assert!(s.contains("unset NO_COLOR\n"));
        assert!(s.contains("unset FORCE_COLOR\n"));
        // Same ordering rule as the session markers: clearing must precede the
        // provider's own exports, or it would undo them.
        let unset_at = s.rfind("unset ").expect("unsets present");
        let export_at = s.find("export ANTHROPIC_").expect("exports present");
        assert!(unset_at < export_at, "clears must precede exports:\n{s}");

        let ps = render(&ScriptSpec {
            flavor: ScriptFlavor::PowerShell,
            cwd: r"C:\w",
            env: &env(),
            claude_path: "claude.exe",
            clear_inherited_color: true,
            ..Default::default()
        });
        for k in ["NO_COLOR", "FORCE_COLOR"] {
            assert!(ps.contains(&format!(
                "Remove-Item Env:\\{k} -ErrorAction SilentlyContinue\n"
            )));
        }
    }

    /// `NO_COLOR` is a documented cross-tool standard. A user who exports it
    /// deliberately means it, so a launcher outside a session must leave it be.
    #[test]
    fn a_deliberate_no_color_is_left_alone_outside_a_session() {
        let s = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &env(),
            claude_path: "/usr/bin/claude",
            ..Default::default()
        });
        assert!(!s.contains("NO_COLOR"), "must not override the user's own setting");
        assert!(!s.contains("FORCE_COLOR"));
        // The session markers are unconditional and must still be there.
        assert!(s.contains("unset CLAUDECODE\n"));
    }

    #[test]
    fn env_order_is_deterministic() {
        let a = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &env(),
            claude_path: "/usr/bin/claude",
            ..Default::default()
        });
        let b = render(&ScriptSpec {
            flavor: ScriptFlavor::Sh,
            cwd: "/tmp",
            env: &env(),
            claude_path: "/usr/bin/claude",
            ..Default::default()
        });
        assert_eq!(a, b);
    }
}
