use crate::auth::accounts::AccountManager;
use crate::auth::keychain_guardian::KeychainGuardian;
use crate::auth::{AuthOrchestrator, AuthSource};
use crate::jsonl_parser::PricingTable;
use crate::store::Db;
use crate::usage_api::{UsageClient, UsageSnapshot};
use chrono::{DateTime, Utc};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Notify;

/// Marker struct for the in-app warm-up scheduler. The tokio JoinHandle is
/// held by lib.rs at task spawn time and does not need to be reachable from
/// Tauri commands. Kept as a named struct so future tasks (T22+) can add
/// UI-visible state here (e.g. `last_outcome_by_account`) without churning
/// AppState's field layout.
#[derive(Default)]
pub struct WarmupState {
    // Reserved for future per-account status badges (Plan B T22+).
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct Settings {
    pub polling_interval_secs: u64,
    /// Base spacing between consecutive per-slot polls within one round.
    /// The poll loop compresses below this when (slots × gap) wouldn't fit
    /// in `polling_interval_secs`. Bounded by the `update_settings` command
    /// to a safe range (see commands.rs).
    pub stagger_gap_secs: u64,
    pub thresholds: Vec<u8>,
    /// Utilization % at which the pay-as-you-go bucket notifies. PAYG has a
    /// single threshold, separate from the shared `thresholds` used by the
    /// rate-limit buckets. The struct-level `#[serde(default)]` above (plus
    /// the custom `Default` impl below, which sets this to 85) already fills
    /// this in per-field for settings blobs written before this field
    /// existed, so no field-level default is needed here — see the
    /// `terminal` field for a case where that field-level attribute is
    /// (redundantly) still present.
    pub payg_threshold: u8,
    pub theme: String,
    pub launch_at_login: bool,
    pub crash_reports: bool,
    pub preferred_auth_source: Option<AuthSource>,
    /// `None` means "use the platform default" (`launcher::default_terminal()`).
    /// `#[serde(default)]` keeps settings written before this field readable.
    #[serde(default)]
    pub terminal: Option<crate::providers::launcher::Terminal>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            polling_interval_secs: 300,
            stagger_gap_secs: crate::poll_loop::DEFAULT_STAGGER_GAP_SECS,
            thresholds: vec![75, 90],
            payg_threshold: 85,
            theme: "system".into(),
            launch_at_login: false,
            crash_reports: false,
            preferred_auth_source: None,
            terminal: None,
        }
    }
}

#[cfg(test)]
mod settings_tests {
    use super::Settings;

    #[test]
    fn settings_without_terminal_field_still_deserialize() {
        let json = r#"{
            "polling_interval_secs": 300,
            "stagger_gap_secs": 30,
            "thresholds": [75, 90],
            "theme": "system",
            "launch_at_login": false,
            "crash_reports": false,
            "preferred_auth_source": null
        }"#;
        let s: Settings = serde_json::from_str(json).expect("legacy settings must still parse");
        assert!(s.terminal.is_none());
    }

    #[test]
    fn settings_without_payg_threshold_field_defaults_to_85() {
        // Same legacy fixture as above (no `terminal`, no `payg_threshold`):
        // proves the struct-level `#[serde(default)]` + custom `Default`
        // impl fill in `payg_threshold` per-field, without needing a
        // field-level `#[serde(default = "...")]` fn.
        let json = r#"{
            "polling_interval_secs": 300,
            "stagger_gap_secs": 30,
            "thresholds": [75, 90],
            "theme": "system",
            "launch_at_login": false,
            "crash_reports": false,
            "preferred_auth_source": null
        }"#;
        let s: Settings = serde_json::from_str(json).expect("legacy settings must still parse");
        assert_eq!(s.payg_threshold, 85);
    }

    #[test]
    fn terminal_choice_round_trips() {
        let s = Settings {
            terminal: Some(crate::providers::launcher::Terminal::Iterm2),
            ..Settings::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.terminal, Some(crate::providers::launcher::Terminal::Iterm2));
    }
}

/// Linear projection of where 5h utilization will land at the current
/// window's reset_at, based on observed slope so far this window. Borrowed
/// from ccusage's burn-rate idea — answers "should I keep coding?" with a
/// concrete number instead of just the bare current %. None when we don't
/// yet have enough samples (need at least 2 polls ≥ 2 minutes apart).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct BurnRateProjection {
    /// Slope of five_hour.utilization, in percentage points per minute.
    /// Positive means consumption is rising; negative is rare but possible
    /// if Anthropic adjusts the metric mid-window.
    pub utilization_per_min: f64,
    /// Projected utilization at five_hour.resets_at if the current pace
    /// continues. Not clamped — values >100 are meaningful (means you'd
    /// hit the cap before reset).
    pub projected_at_reset: f64,
}

/// Linear projection of pay-as-you-go spend, sibling to BurnRateProjection:
/// in-memory samples only (empty right after launch), >=2 samples spanning
/// >=2 minutes. Dollars are tracked as cents to match the API payload.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ExtraBurnRate {
    /// Spend velocity in cents per minute.
    pub cents_per_min: f64,
    /// Projected used_credits_cents at extra_usage.resets_at. None when the
    /// account reports no reset date (common for PAYG).
    pub projected_cents_at_reset: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct CachedUsage {
    pub snapshot: UsageSnapshot,
    pub account_id: String,
    pub account_email: String,
    pub last_error: Option<String>,
    #[serde(default)]
    pub burn_rate: Option<BurnRateProjection>,
    #[serde(default)]
    pub extra_burn_rate: Option<ExtraBurnRate>,
    pub auth_source: AuthSource,
}

impl CachedUsage {
    #[allow(dead_code)]
    pub fn is_stale(&self, now: DateTime<Utc>) -> bool {
        (now - self.snapshot.fetched_at) > chrono::Duration::minutes(15)
            || now < self.snapshot.fetched_at
            || self.last_error.is_some()
    }
}

pub struct AppState {
    pub db: Arc<Db>,
    pub auth: Arc<AuthOrchestrator>,
    pub usage: Arc<UsageClient>,
    /// Shared HTTP client — used by the usage API, token exchange, and the
    /// warm-up dispatcher. Stored here so `scheduler_glue` can reach it
    /// without going through UsageClient's private `inner` field.
    pub http_client: Arc<reqwest::Client>,
    pub pricing: Arc<PricingTable>,
    pub settings: RwLock<Settings>,
    pub cached_usage: RwLock<Option<CachedUsage>>,
    pub force_refresh: Notify,
    pub accounts: Arc<AccountManager>,
    pub cached_usage_by_slot: RwLock<HashMap<u32, CachedUsage>>,
    pub active_slot: RwLock<Option<u32>>,
    /// When the live Claude Code account last changed — a swap, an external
    /// `cswap`, or a Claude Code / cowork instance started on another machine.
    ///
    /// Guards the shared-snapshot fast path: `statusline-usage.json` carries no
    /// account identity, so a snapshot written before this instant describes
    /// the previous account and must not be adopted for the new one.
    pub active_since: RwLock<Option<DateTime<Utc>>>,
    pub backoff_by_slot: RwLock<HashMap<u32, BackoffState>>,
    pub schedule_by_slot: RwLock<HashMap<u32, ScheduleState>>,
    pub keychain_guardian: Mutex<Option<KeychainGuardian>>,
    pub warmup: WarmupState,
    /// Set once a real `TrayIconEvent` has reached `tauri_plugin_positioner`.
    /// `Position::TrayCenter` panics (and aborts the process under the
    /// release profile's `panic = "abort"`) if no tray position has been
    /// recorded yet — which can happen on a cold launch, since the popover's
    /// webview mounts and requests a resize before the user has ever
    /// interacted with the tray icon. `resize_window` checks this before
    /// asking for `TrayCenter`.
    pub tray_position_known: AtomicBool,
    /// Cached session-browser scan, invalidated when any transcript's mtime
    /// advances. `ExpandedReport` remounts tab components on every tab
    /// change, so an unmemoized scan would re-parse ~100 MB per switch.
    pub sessions_cache: RwLock<Option<(std::time::SystemTime, Vec<crate::sessions::SessionSummary>)>>,
}

#[derive(Debug, Clone, Copy)]
pub struct BackoffState {
    pub until: Instant,
}

#[derive(Debug, Clone, Copy)]
pub struct ScheduleState {
    /// Earliest moment this slot is eligible for a usage fetch. The poll
    /// loop will not fetch this slot before this instant. Updated after
    /// each successful fetch to `now + polling_interval_secs`.
    pub next_poll_at: Instant,
}

impl AppState {
    pub fn snapshot(&self) -> Option<CachedUsage> {
        let active = *self.active_slot.read();
        if let Some(slot) = active {
            if let Some(c) = self.cached_usage_by_slot.read().get(&slot) {
                return Some(c.clone());
            }
        }
        self.cached_usage.read().clone()
    }
}
