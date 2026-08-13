//! Guarded, single-key write/undo for `~/.claude/settings.json`'s
//! `statusLine` field. Mirrors `providers::default_env`'s `apply`/`clear`
//! shape, but for one object-valued top-level key instead of a flat map
//! merged into `env`.

use crate::providers::default_env::{backup, read_settings, stamp, write_atomic};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::Path;

/// Write `{"type": "command", "command": command}` as `statusLine`. Returns
/// the prior value (`None` if the key was absent) as the undo record.
pub fn apply(path: &Path, command: &str) -> Result<Option<Value>> {
    let before = stamp(path)?;
    let mut settings = read_settings(path)?;
    let prior = settings.get("statusLine").cloned();

    backup(path)?;
    settings.insert(
        "statusLine".to_string(),
        json!({ "type": "command", "command": command }),
    );
    write_atomic(path, &settings, before)?;
    Ok(prior)
}

/// Restore `prior` (or remove the key if `prior` is `None`). Drift check: if
/// the current `statusLine` no longer equals `written` (the value `apply`
/// last set), the user or another tool changed it since — leave it alone and
/// return `false` rather than silently reverting their edit.
pub fn clear(path: &Path, prior: &Option<Value>, written: &Value) -> Result<bool> {
    let before = stamp(path)?;
    let mut settings = read_settings(path)?;
    let current = settings.get("statusLine");

    if current != Some(written) {
        return Ok(false);
    }

    backup(path)?;
    match prior {
        Some(v) => {
            settings.insert("statusLine".to_string(), v.clone());
        }
        None => {
            settings.remove("statusLine");
        }
    }
    write_atomic(path, &settings, before)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use tempfile::tempdir;

    fn write(path: &std::path::Path, s: &str) {
        std::fs::write(path, s).unwrap();
    }

    #[test]
    fn apply_creates_statusline_when_settings_missing() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        let prior = apply(&p, "/usr/local/bin/switchboard statusline").unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["statusLine"]["type"], "command");
        assert_eq!(v["statusLine"]["command"], "/usr/local/bin/switchboard statusline");
        assert_eq!(prior, None);
    }

    #[test]
    fn apply_preserves_unrelated_keys_and_reports_prior_statusline() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(
            &p,
            r#"{
          "hooks": {"PreToolUse": [{"matcher": "Bash"}]},
          "statusLine": {"type": "command", "command": "bash x.sh"},
          "model": "opus"
        }"#,
        );
        let prior = apply(&p, "/usr/local/bin/switchboard statusline").unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["hooks"]["PreToolUse"][0]["matcher"], "Bash");
        assert_eq!(v["model"], "opus");
        assert_eq!(v["statusLine"]["command"], "/usr/local/bin/switchboard statusline");
        assert_eq!(prior, Some(json!({"type": "command", "command": "bash x.sh"})));
    }

    #[test]
    fn clear_removes_the_key_when_it_was_absent_before() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        let written = json!({"type": "command", "command": "/usr/local/bin/switchboard statusline"});
        apply(&p, "/usr/local/bin/switchboard statusline").unwrap();
        let ok = clear(&p, &None, &written).unwrap();
        assert!(ok);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert!(v.get("statusLine").is_none());
    }

    #[test]
    fn clear_restores_the_prior_value_when_one_existed() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"statusLine": {"type": "command", "command": "bash x.sh"}}"#);
        let prior = apply(&p, "/usr/local/bin/switchboard statusline").unwrap();
        let written = json!({"type": "command", "command": "/usr/local/bin/switchboard statusline"});
        let ok = clear(&p, &prior, &written).unwrap();
        assert!(ok);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["statusLine"]["command"], "bash x.sh");
    }

    #[test]
    fn clear_skips_and_reports_false_when_the_user_changed_it_since() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        apply(&p, "/usr/local/bin/switchboard statusline").unwrap();
        // User hand-edits it after Switchboard installed its own.
        write(&p, r#"{"statusLine": {"type": "command", "command": "bash hand-edited.sh"}}"#);
        let written = json!({"type": "command", "command": "/usr/local/bin/switchboard statusline"});
        let ok = clear(&p, &None, &written).unwrap();
        assert!(!ok);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["statusLine"]["command"], "bash hand-edited.sh");
    }
}
