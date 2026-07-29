//! The opt-in global default: merges a provider's env into
//! `~/.claude/settings.json` so bare `claude` invocations use it.
//!
//! Claude Code applies this block with `Object.assign(process.env, …)` at
//! startup, so it OVERRIDES variables exported by the user's own launch
//! scripts. That is why this path is opt-in and loudly warned about, while
//! the launcher (which mutates nothing global) is the default.

use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const MAX_BACKUPS: usize = 5;

/// Read + parse. A missing file is an empty object; a malformed file is an
/// error, never something we overwrite.
fn read_settings(path: &Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let text = std::fs::read_to_string(path).context("read settings.json")?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    let value: Value = serde_json::from_str(&text)
        .context("settings.json is not valid JSON — refusing to overwrite it")?;
    match value {
        Value::Object(map) => Ok(map),
        _ => anyhow::bail!("settings.json must contain a JSON object at the top level"),
    }
}

fn backup_path(path: &Path, ts: i64) -> PathBuf {
    let name = format!(
        "{}.switchboard-{ts}",
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "settings.json".into())
    );
    path.with_file_name(name)
}

fn backup(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    // Nanoseconds, not seconds: two writes within the same second would
    // otherwise overwrite each other's backup.
    let ts = chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| chrono::Utc::now().timestamp());
    let dest = backup_path(path, ts);
    // fs::copy preserves the source mode on Unix, so a 0600 settings.json
    // yields a 0600 backup. Asserted in tests rather than assumed — the
    // backup will contain a plaintext API key once a default is set.
    std::fs::copy(path, &dest).context("back up settings.json")?;
    prune_backups(path)?;
    Ok(())
}

/// Snapshot used to detect a concurrent writer. Claude Code writes this file
/// itself (`/config`, plugin toggles, statusLine, theme), so an atomic rename
/// is not enough — it prevents a torn file, not a lost update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    mtime_ns: i128,
    len: u64,
}

fn stamp(path: &Path) -> Result<Option<FileStamp>> {
    match std::fs::metadata(path) {
        Ok(m) => {
            let mtime_ns = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as i128)
                .unwrap_or(0);
            Ok(Some(FileStamp { mtime_ns, len: m.len() }))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).context("stat settings.json"),
    }
}

fn prune_backups(path: &Path) -> Result<()> {
    let Some(dir) = path.parent() else {
        return Ok(());
    };
    let prefix = format!(
        "{}.switchboard-",
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    );
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .context("list settings dir")?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().starts_with(&prefix))
                .unwrap_or(false)
        })
        .collect();
    found.sort();
    while found.len() > MAX_BACKUPS {
        let oldest = found.remove(0);
        let _ = std::fs::remove_file(oldest);
    }
    Ok(())
}

/// Write via a sibling temp file then rename, so a crash mid-write cannot
/// leave a truncated settings.json.
///
/// `expected` is the stamp captured when the file was read. If the file has
/// changed since, another writer (Claude Code) got there first and we abort
/// rather than clobber their change.
///
/// The temp file is created 0600 *before* any content is written. A plain
/// `fs::write` produces 0644 under a typical umask, and since we rename over
/// the target, that would silently downgrade a 0600 settings.json — in a file
/// that holds a plaintext API key once a default is set.
fn write_atomic(path: &Path, map: &Map<String, Value>, expected: Option<FileStamp>) -> Result<()> {
    if stamp(path)? != expected {
        anyhow::bail!(
            "settings.json changed while Switchboard was editing it — no changes were written. \
             Close any running Claude Code /config screen and try again."
        );
    }

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).context("create settings dir")?;
    let tmp = path.with_extension("switchboard-tmp");
    let text =
        serde_json::to_string_pretty(&Value::Object(map.clone())).context("serialize settings.json")?;

    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).context("create temp settings.json")?;
        use std::io::Write as _;
        f.write_all(format!("{text}\n").as_bytes())
            .context("write temp settings.json")?;
        f.sync_all().ok();
    }

    // Carry the original mode forward when the target already existed, so we
    // never loosen (or tighten) what the user chose.
    #[cfg(unix)]
    if let Ok(meta) = std::fs::metadata(path) {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode() & 0o777;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode));
    }

    std::fs::rename(&tmp, path).context("replace settings.json")?;
    Ok(())
}

fn env_object(map: &Map<String, Value>) -> Map<String, Value> {
    map.get("env")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// Merge `env` into `settings.json`. Returns the undo manifest: every key
/// written, mapped to the value it held beforehand (`None` = absent).
pub fn apply(
    path: &Path,
    env: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, Option<String>>> {
    let before = stamp(path)?;
    let mut settings = read_settings(path)?;
    let mut env_map = env_object(&settings);

    let mut manifest = BTreeMap::new();
    for (k, v) in env {
        let prior = env_map.get(k).and_then(Value::as_str).map(str::to_string);
        manifest.insert(k.clone(), prior);
        env_map.insert(k.clone(), Value::String(v.clone()));
    }

    backup(path)?;
    settings.insert("env".to_string(), Value::Object(env_map));
    write_atomic(path, &settings, before)?;
    Ok(manifest)
}

/// Replay the manifest in reverse: keys recorded as `None` are removed,
/// keys with a recorded prior value are restored to it. Keys we never wrote
/// are untouched.
/// Returns the keys that were *skipped* because the user changed them by hand
/// while the default was active. The manifest claims we own those keys, so a
/// naive replay would silently revert the user's edit.
pub fn clear(
    path: &Path,
    manifest: &BTreeMap<String, Option<String>>,
    written: &BTreeMap<String, String>,
) -> Result<Vec<String>> {
    if manifest.is_empty() {
        return Ok(Vec::new());
    }
    let before = stamp(path)?;
    let mut settings = read_settings(path)?;
    let mut env_map = env_object(&settings);
    let mut skipped = Vec::new();

    for (k, prior) in manifest {
        // Drift check: if the current value differs from what we wrote, the
        // user edited it. Leave it alone and report it.
        let current = env_map.get(k).and_then(Value::as_str);
        if let (Some(cur), Some(ours)) = (current, written.get(k)) {
            if cur != ours {
                skipped.push(k.clone());
                continue;
            }
        }
        match prior {
            Some(v) => {
                env_map.insert(k.clone(), Value::String(v.clone()));
            }
            None => {
                env_map.remove(k);
            }
        }
    }

    backup(path)?;
    if env_map.is_empty() {
        settings.remove("env");
    } else {
        settings.insert("env".to_string(), Value::Object(env_map));
    }
    write_atomic(path, &settings, before)?;
    skipped.sort();
    Ok(skipped)
}

/// Every env key this feature can own. A prefix-only filter is wrong:
/// presets also write `API_TIMEOUT_MS`, which matches no prefix.
pub fn is_provider_key(k: &str) -> bool {
    k.starts_with("ANTHROPIC_") || k.starts_with("CLAUDE_CODE_") || k == "API_TIMEOUT_MS"
}

/// Provider keys already present that we do not own. The caller requires
/// explicit confirmation before overwriting these — they indicate hand-editing
/// or another tool such as cc-switch.
pub fn foreign_provider_keys(
    path: &Path,
    manifest: &BTreeMap<String, Option<String>>,
) -> Result<Vec<String>> {
    let settings = read_settings(path)?;
    let env_map = env_object(&settings);
    let mut out: Vec<String> = env_map
        .keys()
        .filter(|k| is_provider_key(k))
        .filter(|k| !manifest.contains_key(*k))
        .cloned()
        .collect();
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn write(path: &Path, s: &str) {
        std::fs::write(path, s).unwrap();
    }

    #[test]
    fn apply_creates_env_when_settings_missing() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        let manifest = apply(&p, &env()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["env"]["ANTHROPIC_MODEL"], "glm-5.2");
        assert_eq!(manifest.get("ANTHROPIC_MODEL"), Some(&None));
    }

    #[test]
    fn apply_preserves_hooks_plugins_and_statusline() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(
            &p,
            r#"{
          "hooks": {"PreToolUse": [{"matcher": "Bash"}]},
          "enabledPlugins": {"superpowers@official": true},
          "statusLine": {"type": "command", "command": "bash x.sh"},
          "model": "opus"
        }"#,
        );
        apply(&p, &env()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["hooks"]["PreToolUse"][0]["matcher"], "Bash");
        assert_eq!(v["enabledPlugins"]["superpowers@official"], true);
        assert_eq!(v["statusLine"]["command"], "bash x.sh");
        assert_eq!(v["model"], "opus");
        assert_eq!(v["env"]["ANTHROPIC_MODEL"], "glm-5.2");
    }

    #[test]
    fn apply_preserves_unrelated_env_entries() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"env": {"CLAUDE_CODE_RETRY_WATCHDOG": "1"}}"#);
        apply(&p, &env()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["env"]["CLAUDE_CODE_RETRY_WATCHDOG"], "1");
    }

    #[test]
    fn clear_removes_keys_that_did_not_exist_before() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"env": {"CLAUDE_CODE_RETRY_WATCHDOG": "1"}}"#);
        let manifest = apply(&p, &env()).unwrap();
        clear(&p, &manifest, &env()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert!(v["env"].get("ANTHROPIC_MODEL").is_none());
        assert_eq!(v["env"]["CLAUDE_CODE_RETRY_WATCHDOG"], "1");
    }

    #[test]
    fn clear_restores_a_preexisting_value() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"env": {"ANTHROPIC_MODEL": "claude-opus-5"}}"#);
        let manifest = apply(&p, &env()).unwrap();
        assert_eq!(
            manifest.get("ANTHROPIC_MODEL"),
            Some(&Some("claude-opus-5".to_string()))
        );
        clear(&p, &manifest, &env()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["env"]["ANTHROPIC_MODEL"], "claude-opus-5");
    }

    #[test]
    fn apply_then_clear_round_trips_to_equivalent_json() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        let original =
            r#"{"model":"opus","env":{"CLAUDE_CODE_RETRY_WATCHDOG":"1"},"tui":"fullscreen"}"#;
        write(&p, original);
        let before: Value = serde_json::from_str(original).unwrap();
        let manifest = apply(&p, &env()).unwrap();
        clear(&p, &manifest, &env()).unwrap();
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(before, after, "round-trip must restore the original document");
    }

    #[test]
    fn clear_drops_the_env_object_when_it_becomes_empty() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"model":"opus"}"#);
        let manifest = apply(&p, &env()).unwrap();
        clear(&p, &manifest, &env()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert!(v.get("env").is_none(), "empty env object should be removed");
        assert_eq!(v["model"], "opus");
    }

    #[test]
    fn malformed_json_is_refused_not_overwritten() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, "{ this is not json");
        assert!(apply(&p, &env()).is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{ this is not json");
    }

    #[test]
    fn non_object_settings_is_refused() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, "[1,2,3]");
        assert!(apply(&p, &env()).is_err());
    }

    #[test]
    fn unmanaged_keys_are_reported() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(
            &p,
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://other","CLAUDE_CODE_MAX_CONTEXT_TOKENS":"9","THEME":"x"}}"#,
        );
        let found = foreign_provider_keys(&p, &BTreeMap::new()).unwrap();
        assert_eq!(found, vec!["ANTHROPIC_BASE_URL", "CLAUDE_CODE_MAX_CONTEXT_TOKENS"]);
    }

    #[test]
    fn keys_we_already_manage_are_not_reported_as_unmanaged() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"env":{"ANTHROPIC_BASE_URL":"https://other"}}"#);
        let manifest = BTreeMap::from([("ANTHROPIC_BASE_URL".to_string(), None)]);
        assert!(foreign_provider_keys(&p, &manifest).unwrap().is_empty());
    }

    #[test]
    fn a_backup_is_written_before_each_change() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"model":"opus"}"#);
        apply(&p, &env()).unwrap();
        let backups: Vec<_> = std::fs::read_dir(d.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".switchboard-"))
            .collect();
        assert_eq!(backups.len(), 1, "exactly one backup after one apply");
    }

    #[test]
    fn no_temp_file_is_left_behind() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        apply(&p, &env()).unwrap();
        assert!(!d.path().join("settings.switchboard-tmp").exists());
    }

    #[test]
    fn backups_are_capped_at_five() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"model":"opus"}"#);
        // Pre-create seven backups with sortable, distinct timestamps.
        for ts in 1_700_000_000..1_700_000_007i64 {
            std::fs::write(backup_path(&p, ts), "{}").unwrap();
        }
        prune_backups(&p).unwrap();
        let count = std::fs::read_dir(d.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".switchboard-"))
            .count();
        assert_eq!(count, MAX_BACKUPS);
    }

    fn kimi_env() -> BTreeMap<String, String> {
        BTreeMap::from([
            (
                "ANTHROPIC_BASE_URL".to_string(),
                "https://api.kimi.com/coding".to_string(),
            ),
            ("ANTHROPIC_MODEL".to_string(), "k3".to_string()),
            (
                "CLAUDE_CODE_MAX_CONTEXT_TOKENS".to_string(),
                "262144".to_string(),
            ),
        ])
    }

    /// Spec §4.1 — the C2 regression. A → B → clear must restore the user's
    /// ORIGINAL value, not provider A's.
    #[test]
    fn switching_default_a_to_b_then_clearing_restores_the_users_own_value() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"env":{"ANTHROPIC_MODEL":"claude-opus-5"}}"#);

        // Enable A (GLM).
        let m_a = apply(&p, &env()).unwrap();
        assert_eq!(
            m_a.get("ANTHROPIC_MODEL"),
            Some(&Some("claude-opus-5".to_string()))
        );

        // Switch to B (Kimi) the correct way: clear A first, then apply B.
        clear(&p, &m_a, &env()).unwrap();
        let m_b = apply(&p, &kimi_env()).unwrap();
        assert_eq!(
            m_b.get("ANTHROPIC_MODEL"),
            Some(&Some("claude-opus-5".to_string())),
            "B's manifest must record the USER's value, not A's"
        );

        // Clear B.
        clear(&p, &m_b, &kimi_env()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["env"]["ANTHROPIC_MODEL"], "claude-opus-5");
        assert!(
            v["env"].get("ANTHROPIC_BASE_URL").is_none(),
            "no orphaned key from either provider"
        );
        assert!(
            v["env"].get("CLAUDE_CODE_MAX_CONTEXT_TOKENS").is_none(),
            "B's extra key must not orphan"
        );
    }

    /// Spec §4.2 — a key the user edited while the default was active is left
    /// alone on clear, and reported.
    #[test]
    fn clear_does_not_revert_a_key_the_user_edited() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        let manifest = apply(&p, &env()).unwrap();

        // User hand-edits the model while the default is active.
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        v["env"]["ANTHROPIC_MODEL"] = Value::String("glm-5.2-air".into());
        std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap()).unwrap();

        let skipped = clear(&p, &manifest, &env()).unwrap();
        assert_eq!(
            skipped,
            vec!["ANTHROPIC_MODEL"],
            "user's edit must be reported as skipped"
        );
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(
            after["env"]["ANTHROPIC_MODEL"], "glm-5.2-air",
            "user's edit must survive"
        );
    }

    /// Spec §4 defense #5 — a prefix-only filter passes ANTHROPIC_* and
    /// silently overwrites the rest of what presets write.
    #[test]
    fn foreign_key_detection_covers_every_preset_written_key() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(
            &p,
            r#"{"env":{
            "ANTHROPIC_MODEL":"x",
            "CLAUDE_CODE_MAX_CONTEXT_TOKENS":"1",
            "CLAUDE_CODE_AUTO_COMPACT_WINDOW":"2",
            "CLAUDE_CODE_SUBAGENT_MODEL":"y",
            "API_TIMEOUT_MS":"3",
            "EDITOR":"vim"
        }}"#,
        );
        let found = foreign_provider_keys(&p, &BTreeMap::new()).unwrap();
        assert!(
            found.contains(&"API_TIMEOUT_MS".to_string()),
            "API_TIMEOUT_MS matches no prefix"
        );
        assert!(found.contains(&"CLAUDE_CODE_SUBAGENT_MODEL".to_string()));
        assert!(
            !found.contains(&"EDITOR".to_string()),
            "unrelated keys are not ours"
        );
        assert_eq!(found.len(), 5);
    }

    /// Spec §4 defense #7 — a concurrent writer must not be clobbered.
    #[test]
    fn write_aborts_when_the_file_changed_under_us() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"model":"opus"}"#);
        let stale = stamp(&p).unwrap();

        // Someone else writes — e.g. Claude Code enabling a plugin.
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(&p, r#"{"model":"opus","enabledPlugins":{"x":true}}"#);

        let mut settings = read_settings(&p).unwrap();
        settings.insert("env".into(), Value::Object(Map::new()));
        let err = write_atomic(&p, &settings, stale).unwrap_err();
        assert!(
            format!("{err}").contains("changed while"),
            "must abort, got: {err}"
        );

        let after: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(
            after["enabledPlugins"]["x"], true,
            "the other writer's change must survive"
        );
    }

    /// Spec §4 defense #8 — never loosen the target's permissions.
    #[cfg(unix)]
    #[test]
    fn apply_and_clear_preserve_a_0600_settings_file() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"model":"opus"}"#);
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();

        let manifest = apply(&p, &env()).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "apply must not downgrade settings.json to 0644");

        // The backup holds the same secrets and must be just as tight.
        let backup = std::fs::read_dir(d.path())
            .unwrap()
            .flatten()
            .find(|e| e.file_name().to_string_lossy().contains(".switchboard-"))
            .expect("a backup exists");
        let bmode = backup.metadata().unwrap().permissions().mode() & 0o777;
        assert_eq!(bmode, 0o600, "backup must inherit the source mode");

        clear(&p, &manifest, &env()).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "clear must not downgrade either");
    }
}
