//! Writes the active account's usage snapshot to the App Group shared
//! container so the macOS desktop widget extension can read it without
//! its own networking or OAuth. See
//! docs/superpowers/specs/2026-07-31-macos-widget-design.md.

use crate::auth::accounts::ManagedAccount;
use crate::usage_api::UsageSnapshot;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

/// App Group identifier shared with the WidgetKit extension. Replace
/// `<TEAM_ID>` with your Apple Developer Team ID once you've created this
/// App Group under Certificates, Identifiers & Profiles — see Task 5,
/// Step 1 of the widget plan. Must match the widget extension's
/// entitlements exactly.
const APP_GROUP_ID: &str = "<TEAM_ID>.com.claude-switchboard.app";

/// Whether `APP_GROUP_ID` has been substituted with a real Apple Developer
/// Team ID (see Task 5, Step 1 of the widget plan). `<` and `>` are legal
/// APFS filename characters, so an un-substituted placeholder still resolves
/// to a real, creatable container path — callers MUST gate the write on
/// this returning `true`, or every build (including public releases) will
/// silently create `~/Library/Group Containers/<TEAM_ID>.com.claude-switchboard.app/`
/// containing the user's account email, for a widget only a locally-signed
/// build can ever read.
pub fn is_configured() -> bool {
    !APP_GROUP_ID.contains('<')
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetSnapshot {
    pub account_label: String,
    pub tier: String,
    pub five_hour_pct: f64,
    pub five_hour_reset_at: Option<DateTime<Utc>>,
    pub color_band: &'static str,
    pub poll_interval_seconds: u64,
    pub written_at: DateTime<Utc>,
}

/// Mirrors the thresholds in `tray_icon::shared::arc_color` (<75 = safe,
/// 75-89 = warn, >=90 = danger). Kept as a standalone string-returning
/// function because the widget extension is Swift and can't consume a
/// `tiny_skia::Color` — if those thresholds change, update both.
pub fn color_band(pct: f64) -> &'static str {
    if pct >= 90.0 {
        "danger"
    } else if pct >= 75.0 {
        "warn"
    } else {
        "safe"
    }
}

pub fn build(
    acc: &ManagedAccount,
    snapshot: &UsageSnapshot,
    poll_interval_seconds: u64,
    now: DateTime<Utc>,
) -> WidgetSnapshot {
    let pct = snapshot.five_hour.as_ref().map(|u| u.utilization).unwrap_or(0.0);
    WidgetSnapshot {
        account_label: acc.email.clone(),
        tier: acc.subscription_type.clone().unwrap_or_else(|| "—".to_string()),
        five_hour_pct: pct,
        five_hour_reset_at: snapshot.five_hour.as_ref().and_then(|u| u.resets_at),
        color_band: color_band(pct),
        poll_interval_seconds,
        written_at: now,
    }
}

/// Resolves the App Group shared container path:
/// `~/Library/Group Containers/<APP_GROUP_ID>/`.
pub fn container_dir() -> PathBuf {
    let home = directories::BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("Library/Group Containers").join(APP_GROUP_ID)
}

/// Atomically writes the snapshot as `snapshot.json` in `dir`, creating
/// `dir` if needed. Writes to a temp file then renames, so the widget
/// extension never observes a partially written file.
pub fn write(dir: &Path, snapshot: &WidgetSnapshot) -> anyhow::Result<()> {
    fs::create_dir_all(dir)?;
    let tmp_path = dir.join("snapshot.json.tmp");
    let final_path = dir.join("snapshot.json");
    fs::write(&tmp_path, serde_json::to_vec_pretty(snapshot)?)?;
    fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::accounts::AddSource;
    use chrono::TimeZone;

    fn test_account() -> ManagedAccount {
        ManagedAccount {
            slot: 1,
            email: "jay@example.com".into(),
            account_uuid: "uuid-1".into(),
            organization_uuid: None,
            organization_name: None,
            subscription_type: Some("MAX".into()),
            source: AddSource::OAuth,
            claude_code_oauth_blob: serde_json::json!({}),
            oauth_account_blob: serde_json::json!({}),
            token_expires_at: Utc::now(),
            added_at: Utc::now(),
            last_seen_active: None,
        }
    }

    fn test_snapshot(five_hour_pct: f64) -> UsageSnapshot {
        UsageSnapshot {
            five_hour: Some(crate::usage_api::Utilization {
                utilization: five_hour_pct,
                resets_at: Some(Utc.with_ymd_and_hms(2026, 7, 31, 18, 0, 0).unwrap()),
            }),
            seven_day: None,
            seven_day_sonnet: None,
            seven_day_opus: None,
            extra_usage: None,
            fetched_at: Utc::now(),
            unknown: Default::default(),
        }
    }

    #[test]
    fn is_configured_is_false_for_the_current_placeholder() {
        // This assertion doubles as a check that catches the day someone
        // substitutes their real Team ID into APP_GROUP_ID and forgets to
        // update this test — at that point it should start failing here,
        // which is the intended signal that the constant now needs a
        // corresponding is_configured() review, not a silent green build.
        assert!(!is_configured());
    }

    #[test]
    fn color_band_uses_safe_below_75() {
        assert_eq!(color_band(0.0), "safe");
        assert_eq!(color_band(74.9), "safe");
    }

    #[test]
    fn color_band_uses_warn_at_75_to_89() {
        assert_eq!(color_band(75.0), "warn");
        assert_eq!(color_band(89.9), "warn");
    }

    #[test]
    fn color_band_uses_danger_at_90_and_above() {
        assert_eq!(color_band(90.0), "danger");
        assert_eq!(color_band(150.0), "danger");
    }

    #[test]
    fn build_maps_account_and_snapshot_fields() {
        let acc = test_account();
        let snapshot = test_snapshot(42.0);
        let now = Utc.with_ymd_and_hms(2026, 7, 31, 15, 32, 0).unwrap();

        let ws = build(&acc, &snapshot, 300, now);

        assert_eq!(ws.account_label, "jay@example.com");
        assert_eq!(ws.tier, "MAX");
        assert_eq!(ws.five_hour_pct, 42.0);
        assert_eq!(ws.color_band, "safe");
        assert_eq!(ws.poll_interval_seconds, 300);
        assert_eq!(ws.written_at, now);
    }

    #[test]
    fn build_defaults_tier_when_subscription_type_missing() {
        let mut acc = test_account();
        acc.subscription_type = None;
        let snapshot = test_snapshot(10.0);

        let ws = build(&acc, &snapshot, 300, Utc::now());

        assert_eq!(ws.tier, "—");
    }

    #[test]
    fn write_then_read_round_trips_and_overwrites_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let snapshot = build(&test_account(), &test_snapshot(42.0), 300, Utc::now());

        write(dir.path(), &snapshot).unwrap();
        let contents = fs::read_to_string(dir.path().join("snapshot.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed["accountLabel"], "jay@example.com");
        assert_eq!(parsed["fiveHourPct"], 42.0);

        // Overwrite with new data — old content must not linger.
        let snapshot2 = build(&test_account(), &test_snapshot(90.0), 300, Utc::now());
        write(dir.path(), &snapshot2).unwrap();
        let contents2 = fs::read_to_string(dir.path().join("snapshot.json")).unwrap();
        let parsed2: serde_json::Value = serde_json::from_str(&contents2).unwrap();
        assert_eq!(parsed2["fiveHourPct"], 90.0);
        assert!(!dir.path().join("snapshot.json.tmp").exists());
    }
}
