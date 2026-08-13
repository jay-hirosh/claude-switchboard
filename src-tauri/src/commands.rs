use crate::app_state::{AppState, CachedUsage, Settings};
use crate::auth::accounts::{AddSource, ManagedAccount};
use crate::process_detection::{self, RunningClaudeCode};
use crate::store::StoredSessionEvent;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tauri::{command, State};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum RefreshScope {
    /// Re-fetch only the currently active slot. Inactive slots stay on
    /// their staggered schedule. Triggered by the popover home view's
    /// refresh icon.
    Active,
    /// Re-fetch every managed slot, staggered by 30 s starting from now.
    /// Triggered by the AccountsPanel header refresh button.
    All,
}

#[derive(Debug, Serialize, Deserialize, specta::Type)]
pub struct DailyBucket {
    pub date: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub request_count: u64,
}

#[derive(Debug, Serialize, Deserialize, specta::Type)]
pub struct ModelStats {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Serialize, Deserialize, specta::Type)]
pub struct DailyModelBucket {
    pub date: String,
    pub models: Vec<ModelStats>,
}

#[derive(Debug, Serialize, Deserialize, specta::Type)]
pub struct ProjectStats {
    pub project: String,
    pub session_count: u64,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
}

/// One `cwd` within a repository — a monorepo package, or a plain project
/// with no sibling packages, or a git worktree.
#[derive(Debug, Serialize, Deserialize, specta::Type)]
pub struct RepoProjectStats {
    pub project: String,
    pub cwd: String,
    pub session_count: u64,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
}

/// A git repository, which can hold more than one `project`/`cwd` — a
/// monorepo run from several package directories, or the same clone
/// resumed from more than one subdirectory. Distinct from `ProjectStats`,
/// which groups by `cwd` alone and so double-counts a repo worked from two
/// directories.
#[derive(Debug, Serialize, Deserialize, specta::Type)]
pub struct RepoStats {
    pub repo: String,
    pub session_count: u64,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
    pub projects: Vec<RepoProjectStats>,
}

#[derive(Debug, Serialize, Deserialize, specta::Type)]
pub struct CacheStats {
    pub total_cache_read_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub estimated_savings_usd: f64,
    pub hit_ratio: f64,
}

fn err_to_string<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[command]
#[specta::specta]
pub async fn get_current_usage(state: State<'_, Arc<AppState>>) -> Result<Option<CachedUsage>, String> {
    Ok(state.snapshot())
}

#[command]
#[specta::specta]
pub async fn get_pricing(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<crate::jsonl_parser::pricing::PricingEntry>, String> {
    Ok(state.pricing.entries().to_vec())
}

#[command]
#[specta::specta]
pub async fn get_session_history(
    days: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<StoredSessionEvent>, String> {
    let to = Utc::now();
    let from = to - Duration::days(days as i64);
    state.db.events_between(from, to).map_err(err_to_string)
}

/// Sessions whose JSONL transcript received a write within the last
/// LIVE_QUIET_SECS (120s). Purely a read of the in-memory registry — no DB
/// query of its own (the registry's entries are already the result of one).
#[command]
#[specta::specta]
pub async fn get_live_sessions(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<crate::live_sessions::LiveSessionInfo>, String> {
    Ok(state.live_sessions.live_snapshot())
}

/// Compaction boundaries in the same window as `get_session_history`, so the
/// Cost tab can mark which sessions had their context reset partway through.
#[command]
#[specta::specta]
pub async fn get_compactions(
    days: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<crate::store::StoredCompaction>, String> {
    let to = Utc::now();
    let from = to - Duration::days(days as i64);
    state.db.compactions_between(from, to).map_err(err_to_string)
}

#[derive(Debug, Serialize, Deserialize, specta::Type)]
pub struct WarmupSuggestion {
    pub anchor: crate::scheduler::presets::HhMm,
    pub active_days: u32,
}

/// Median first-event-of-day local time across every distinct active day in
/// `events`, or `None` if fewer than `min_active_days` are present — too
/// small a sample would anchor a suggestion on noise. Multiple events on the
/// same day only count once, keyed by that day's earliest timestamp.
fn compute_warmup_suggestion(
    events: &[StoredSessionEvent],
    min_active_days: usize,
) -> Option<WarmupSuggestion> {
    use chrono::Timelike;
    use std::collections::BTreeMap;

    let mut first_of_day: BTreeMap<String, chrono::DateTime<Utc>> = BTreeMap::new();
    for e in events {
        let local = e.ts.with_timezone(&chrono::Local);
        let date = local.format("%Y-%m-%d").to_string();
        first_of_day
            .entry(date)
            .and_modify(|existing| {
                if e.ts < *existing {
                    *existing = e.ts;
                }
            })
            .or_insert(e.ts);
    }

    if first_of_day.len() < min_active_days {
        return None;
    }

    let mut minutes: Vec<u32> = first_of_day
        .values()
        .map(|ts| {
            let local = ts.with_timezone(&chrono::Local);
            local.hour() * 60 + local.minute()
        })
        .collect();
    minutes.sort_unstable();
    let median = minutes[minutes.len() / 2];

    Some(WarmupSuggestion {
        anchor: crate::scheduler::presets::HhMm::new((median / 60) as u8, (median % 60) as u8),
        active_days: first_of_day.len() as u32,
    })
}

/// Suggests a warm-up anchor time from the trailing 90 days of local
/// activity: the median time-of-day the user's first session started on an
/// active day. Returns `None` (not an error) below the 10-active-day floor
/// — the frontend renders nothing in that case rather than an empty state.
#[command]
#[specta::specta]
pub async fn get_warmup_suggestion(
    state: State<'_, Arc<AppState>>,
) -> Result<Option<WarmupSuggestion>, String> {
    const MIN_ACTIVE_DAYS: usize = 10;
    const LOOKBACK_DAYS: u32 = 90;
    let events = get_session_history(LOOKBACK_DAYS, state).await?;
    Ok(compute_warmup_suggestion(&events, MIN_ACTIVE_DAYS))
}

#[derive(Debug, Serialize, Deserialize, specta::Type)]
pub struct LimitHitReport {
    pub accounts: Vec<crate::store::AccountLimitHits>,
}

/// Per-account limit-hit stats over the trailing `days` window, using the
/// second configured threshold (index 1, the "danger" tier) as the bar for
/// what counts as a limit hit. One `limit_hit_stats` query per managed
/// account, not just the active one.
///
/// A single account's query failing doesn't abort the whole report: it's
/// logged and skipped so the other accounts' data still reaches the
/// frontend (same log-and-continue shape as `reconcile_sqlite_account_mirror`).
#[command]
#[specta::specta]
pub async fn get_limit_hit_history(
    days: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<LimitHitReport, String> {
    let to = Utc::now();
    let from = to - Duration::days(days as i64);
    let danger_threshold = {
        let s = state.settings.read();
        s.thresholds.get(1).copied().unwrap_or(90) as f64
    };
    let accounts = state.accounts.list().map_err(err_to_string)?;
    let mut out = Vec::new();
    for acc in accounts {
        match state
            .db
            .limit_hit_stats(&acc.account_uuid, &acc.email, from, to, danger_threshold)
        {
            Ok(hits) => out.push(hits),
            Err(e) => {
                tracing::warn!(
                    "get_limit_hit_history: failed for {}: {e:#}",
                    acc.account_uuid
                );
            }
        }
    }
    Ok(LimitHitReport { accounts: out })
}

#[command]
#[specta::specta]
pub async fn get_daily_trends(
    days: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<DailyBucket>, String> {
    let events = get_session_history(days, state).await?;
    Ok(bucket_daily_trends(&events))
}

/// Serializes daily buckets to CSV — one row per day, header always present
/// even when there's no data, so an empty export is still a valid file.
fn daily_trends_to_csv(buckets: &[DailyBucket]) -> String {
    let mut out = String::from("date,input_tokens,output_tokens,cost_usd,request_count\n");
    for b in buckets {
        out.push_str(&format!(
            "{},{},{},{},{}\n",
            b.date, b.input_tokens, b.output_tokens, b.cost_usd, b.request_count
        ));
    }
    out
}

#[command]
#[specta::specta]
pub async fn export_trends_csv(
    path: String,
    days: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let events = get_session_history(days, state).await?;
    let csv = daily_trends_to_csv(&bucket_daily_trends(&events));
    std::fs::write(&path, csv).map_err(err_to_string)
}

/// Groups events by local calendar day, summing tokens/cost and counting
/// requests (one row per API call/response, not one per conversation —
/// see `session_events`' schema comment for why there's no true session
/// concept to count here).
fn bucket_daily_trends(events: &[StoredSessionEvent]) -> Vec<DailyBucket> {
    use std::collections::BTreeMap;
    let mut by_day: BTreeMap<String, DailyBucket> = BTreeMap::new();
    for e in events {
        let date = e
            .ts
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d")
            .to_string();
        let slot = by_day
            .entry(date.clone())
            .or_insert_with(|| DailyBucket {
                date,
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: 0.0,
                request_count: 0,
            });
        slot.input_tokens += e.input_tokens;
        slot.output_tokens += e.output_tokens;
        slot.cost_usd += e.cost_usd;
        slot.request_count += 1;
    }
    by_day.into_values().collect()
}

#[command]
#[specta::specta]
pub async fn get_model_breakdown(
    days: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<ModelStats>, String> {
    let events = get_session_history(days, state).await?;
    use std::collections::HashMap;
    let mut by_model: HashMap<String, ModelStats> = HashMap::new();
    for e in events {
        let entry = by_model
            .entry(e.model.clone())
            .or_insert_with(|| ModelStats {
                model: e.model.clone(),
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                cost_usd: 0.0,
            });
        entry.input_tokens += e.input_tokens;
        entry.output_tokens += e.output_tokens;
        entry.cache_read_tokens += e.cache_read_tokens;
        entry.cache_creation_tokens += e.cache_creation_5m_tokens + e.cache_creation_1h_tokens;
        entry.cost_usd += e.cost_usd;
    }
    Ok(by_model.into_values().collect())
}

#[command]
#[specta::specta]
pub async fn get_daily_model_breakdown(
    days: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<DailyModelBucket>, String> {
    let events = get_session_history(days, state).await?;
    use std::collections::{BTreeMap, HashMap};
    let mut by_day: BTreeMap<String, HashMap<String, ModelStats>> = BTreeMap::new();
    for e in events {
        let date = e
            .ts
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d")
            .to_string();
        let by_model = by_day.entry(date).or_default();
        let entry = by_model
            .entry(e.model.clone())
            .or_insert_with(|| ModelStats {
                model: e.model.clone(),
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                cost_usd: 0.0,
            });
        entry.input_tokens += e.input_tokens;
        entry.output_tokens += e.output_tokens;
        entry.cache_read_tokens += e.cache_read_tokens;
        entry.cache_creation_tokens += e.cache_creation_5m_tokens + e.cache_creation_1h_tokens;
        entry.cost_usd += e.cost_usd;
    }
    Ok(by_day
        .into_iter()
        .map(|(date, models)| {
            let mut models: Vec<ModelStats> = models.into_values().collect();
            models.sort_by(|a, b| b.cost_usd.total_cmp(&a.cost_usd));
            DailyModelBucket { date, models }
        })
        .collect())
}

#[command]
#[specta::specta]
pub async fn get_project_breakdown(
    days: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<ProjectStats>, String> {
    let events = get_session_history(days, state).await?;
    use std::collections::HashMap;
    let mut by_project: HashMap<String, ProjectStats> = HashMap::new();
    for e in events {
        let entry = by_project
            .entry(e.project.clone())
            .or_insert_with(|| ProjectStats {
                project: e.project.clone(),
                session_count: 0,
                total_tokens: 0,
                total_cost_usd: 0.0,
            });
        entry.session_count += 1;
        entry.total_tokens += e.input_tokens + e.output_tokens;
        entry.total_cost_usd += e.cost_usd;
    }
    Ok(by_project.into_values().collect())
}

/// Walks up from `cwd` looking for a `.git` entry (directory for a normal
/// clone, file for a worktree) and returns the name of the directory it
/// lives in. Falls back to `cwd`'s own last component — same rule
/// `SessionSummary.project_name` uses — when no `.git` is found, which
/// covers a deleted project folder or a directory that was never a repo.
fn resolve_repo_name(cwd: &str) -> String {
    let leaf = |p: &std::path::Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| cwd.to_string())
    };
    let mut dir = std::path::Path::new(cwd);
    loop {
        if dir.join(".git").exists() {
            return leaf(dir);
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent,
            _ => break,
        }
    }
    leaf(std::path::Path::new(cwd))
}

#[command]
#[specta::specta]
pub async fn get_repo_breakdown(state: State<'_, Arc<AppState>>) -> Result<Vec<RepoStats>, String> {
    let sessions = list_resumable_sessions(state).await?;
    use std::collections::HashMap;

    let mut by_repo: HashMap<String, RepoStats> = HashMap::new();
    let mut by_project: HashMap<(String, String), RepoProjectStats> = HashMap::new();

    for s in &sessions {
        let repo = resolve_repo_name(&s.cwd);

        let repo_entry = by_repo.entry(repo.clone()).or_insert_with(|| RepoStats {
            repo: repo.clone(),
            session_count: 0,
            total_tokens: 0,
            total_cost_usd: 0.0,
            projects: Vec::new(),
        });
        repo_entry.session_count += 1;
        repo_entry.total_tokens += s.total_tokens;
        repo_entry.total_cost_usd += s.total_cost_usd;

        let proj_entry = by_project
            .entry((repo, s.cwd.clone()))
            .or_insert_with(|| RepoProjectStats {
                project: s.project_name.clone(),
                cwd: s.cwd.clone(),
                session_count: 0,
                total_tokens: 0,
                total_cost_usd: 0.0,
            });
        proj_entry.session_count += 1;
        proj_entry.total_tokens += s.total_tokens;
        proj_entry.total_cost_usd += s.total_cost_usd;
    }

    for ((repo, _cwd), proj) in by_project {
        if let Some(entry) = by_repo.get_mut(&repo) {
            entry.projects.push(proj);
        }
    }

    let mut out: Vec<RepoStats> = by_repo.into_values().collect();
    for repo in &mut out {
        repo.projects
            .sort_by(|a, b| b.total_cost_usd.total_cmp(&a.total_cost_usd));
    }
    out.sort_by(|a, b| b.total_cost_usd.total_cmp(&a.total_cost_usd));
    Ok(out)
}

#[command]
#[specta::specta]
pub async fn get_cache_stats(
    days: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<CacheStats, String> {
    let pricing = state.pricing.clone();
    let events = get_session_history(days, state).await?;
    let mut read = 0u64;
    let mut created = 0u64;
    for e in &events {
        read += e.cache_read_tokens;
        created += e.cache_creation_5m_tokens + e.cache_creation_1h_tokens;
    }
    let total = read + created;
    let hit_ratio = if total > 0 {
        (read as f64) / (total as f64)
    } else {
        0.0
    };
    // Savings are per-model: cache-read tokens × (input − cache-read rate).
    // The previous flat $2.7/MTok was only right for Sonnet — it understated
    // Opus ($4.50) and Fable ($9.00) savings and priced unknown models' cache
    // reads as free when they simply can't be estimated.
    let savings: f64 = events
        .iter()
        .map(|e| {
            pricing.cache_savings_per_mtok(&e.model).unwrap_or(0.0)
                * (e.cache_read_tokens as f64)
                / 1_000_000.0
        })
        .sum();
    Ok(CacheStats {
        total_cache_read_tokens: read,
        total_cache_creation_tokens: created,
        estimated_savings_usd: savings,
        hit_ratio,
    })
}

#[command]
#[specta::specta]
pub async fn start_oauth_flow(
    long_lived: bool,
    state: State<'_, Arc<AppState>>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    tracing::info!(
        target: "switchboard.auth",
        "OAuth flow starting (long_lived={long_lived})"
    );
    use crate::auth::oauth_paste_back::{
        build_authorize_url, generate_pkce, start_local_callback_server,
        LONG_LIVED_EXPIRES_IN_SECS,
    };

    let pkce = generate_pkce();
    let (port, rx) = start_local_callback_server().await.map_err(err_to_string)?;
    let redirect_uri = format!("http://localhost:{port}/callback");
    let url = build_authorize_url(&pkce, &redirect_uri, long_lived).map_err(err_to_string)?;
    let expires_in = if long_lived { Some(LONG_LIVED_EXPIRES_IN_SECS) } else { None };

    let state_clone = Arc::clone(state.inner());

    tokio::spawn(async move {
        use tauri::Emitter;

        let result: Result<u32, String> = async {
            let (code, callback_state) = rx
                .await
                .map_err(|_| "OAuth server closed before callback arrived".to_string())?
                .map_err(err_to_string)?;

            if callback_state != pkce.state {
                return Err("State mismatch — possible replay attack".to_string());
            }

            let token = state_clone
                .auth
                .exchange
                .exchange_code(&code, &pkce.verifier, &redirect_uri, &pkce.state, expires_in)
                .await
                .map_err(err_to_string)?;

            let userinfo = state_clone
                .auth
                .identity
                .fetch(&token.access_token)
                .await
                .map_err(err_to_string)?;

            let slot = state_clone
                .accounts
                .add_from_oauth(token, userinfo)
                .await
                .map_err(err_to_string)?;

            // Mirror the new account into the SQLite `accounts` table so
            // warm-up commands (which key off SQLite, not accounts.json)
            // can find a row. Failure here is non-fatal: the add succeeded;
            // warm-up will just default to disabled until the next mirror
            // (set_warmup_enabled also INSERT-OR-IGNOREs as a backstop).
            if let Err(e) = mirror_account_to_sqlite(&state_clone, slot) {
                tracing::warn!("oauth_complete: SQLite mirror failed: {e:#}");
            }

            state_clone.force_refresh.notify_one();
            Ok(slot)
        }
        .await;

        match result {
            Ok(slot) => {
                tracing::info!(
                    target: "switchboard.auth",
                    "OAuth flow complete (slot={slot})"
                );
                let _ = app.emit("oauth_complete", slot);
            }
            Err(e) => {
                tracing::warn!(
                    target: "switchboard.auth",
                    "OAuth flow failed: {e}"
                );
                let _ = app.emit("oauth_error", e);
            }
        }
    });

    Ok(url)
}

#[command]
#[specta::specta]
pub async fn force_refresh(
    scope: RefreshScope,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    use crate::app_state::ScheduleState;

    tracing::info!(target: "switchboard.poll", "force_refresh scope={scope:?}");
    let now = Instant::now();
    match scope {
        RefreshScope::Active => {
            if let Some(active) = *state.active_slot.read() {
                state.schedule_by_slot.write().insert(
                    active,
                    ScheduleState { next_poll_at: now },
                );
            }
        }
        RefreshScope::All => {
            let accounts = state.accounts.list().map_err(err_to_string)?;
            let active = *state.active_slot.read();
            let interval = std::time::Duration::from_secs(
                state.settings.read().polling_interval_secs.max(60),
            );
            let slot_ids: Vec<u32> = accounts.iter().map(|a| a.slot).collect();
            // Manual refresh is a rare user-initiated burst: use the minimum
            // stagger so every slot refreshes within seconds rather than
            // riding the steady-state 30s stagger, which left later slots
            // stale for minutes after clicking "refresh".
            *state.schedule_by_slot.write() = crate::poll_loop::seed_schedules(
                &slot_ids,
                active,
                now,
                interval,
                std::time::Duration::from_secs(5),
            );
        }
    }
    state.force_refresh.notify_one();
    Ok(())
}

#[command]
#[specta::specta]
pub async fn has_claude_code_creds() -> Result<bool, String> {
    Ok(crate::auth::claude_code_creds::has_creds().await)
}

#[command]
#[specta::specta]
pub async fn update_settings(s: Settings, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    if s.polling_interval_secs < 60 {
        return Err("polling_interval_secs must be at least 60".to_string());
    }
    // 5s lower bound prevents thrashing the upstream usage endpoint when many
    // slots are present; 120s upper bound keeps the round-trip across all
    // slots inside the polling interval at reasonable account counts.
    if !(5..=120).contains(&s.stagger_gap_secs) {
        return Err("stagger_gap_secs must be between 5 and 120".to_string());
    }
    if s.thresholds.iter().any(|&t| t > 100) {
        return Err("threshold values must be between 0 and 100".to_string());
    }
    if s.payg_threshold > 100 {
        return Err("payg_threshold must be between 0 and 100".to_string());
    }
    state.db.save_settings(&s).map_err(|e| e.to_string())?;
    *state.settings.write() = s;
    Ok(())
}

#[command]
#[specta::specta]
pub async fn get_settings(state: State<'_, Arc<AppState>>) -> Result<Settings, String> {
    Ok(state.settings.read().clone())
}

#[cfg(debug_assertions)]
#[command]
#[specta::specta]
pub async fn debug_force_threshold(
    bucket: String,
    pct: u8,
    _state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    tracing::info!("debug_force_threshold({bucket}, {pct})");
    Ok(())
}

/// Target top-left for compact-mode resizes: the TOP edge stays glued
/// (menu-bar popovers hang downward from the tray icon; a center-fixed
/// resize visibly detaches the popover from the menu bar on every
/// collapse/expand). Width is recentered — a no-op when width is unchanged
/// (the common case; heights are the only thing that differs today).
fn compact_target_xy(from_x: f64, from_y: f64, from_w: f64, target_w: f64) -> (f64, f64) {
    (from_x + (from_w - target_w) / 2.0, from_y)
}

#[command]
#[specta::specta]
pub async fn resize_window(mode: String, extra_height: f64, app: tauri::AppHandle) -> Result<(), String> {    use tauri::{LogicalPosition, LogicalSize, Manager, Position, Size};

    // `Position::TrayCenter` panics (aborting the process under this crate's
    // `panic = "abort"` release profile) if `tauri_plugin_positioner` hasn't
    // recorded a tray position yet — which happens on a cold launch, since
    // the popover's webview mounts and calls this before the user has ever
    // interacted with the tray icon. Skip the tray-relative repositioning
    // until a real `TrayIconEvent` has arrived; the window is still sized
    // correctly, just not yet re-anchored, and the next call (e.g. the
    // user's first tray click, which itself supplies that position) fixes it.
    let tray_position_known = app
        .state::<Arc<AppState>>()
        .tray_position_known
        .load(std::sync::atomic::Ordering::Relaxed);

    let Some(w) = app.get_webview_window("popover") else {
        return Ok(());
    };

    let target_size = match mode.as_str() {
        "compact" => (360.0_f64, 380.0_f64),
        // Hero numbers + details disclosure + footer only — the popover's
        // default glance view. Details expand back to full compact height.
        "compact-minimal" => (360.0_f64, 208.0_f64),
        "expanded" => (960.0_f64, 640.0_f64),
        _ => return Ok(()),
    };
    // Extra room for the "Now running" live-session rows, driven by the
    // caller's current live-session row count (see CompactPopover.tsx's
    // resize effect). Clamped defensively — this is a caller-supplied f64
    // crossing the IPC boundary, so a runaway value must not be able to
    // blow the popover off-screen.
    let target_size = (target_size.0, target_size.1 + extra_height.clamp(0.0, 200.0));

    // Apply flag changes upfront so the rest of the animation runs in the
    // target mode's resize profile (resizable + always-on-top affect how
    // window-managers respond to subsequent set_size calls on some
    // platforms).
    match mode.as_str() {
        "compact" | "compact-minimal" => {
            let _ = w.set_always_on_top(true);
            let _ = w.set_resizable(false);
        }
        "expanded" => {
            let _ = w.set_resizable(true);
            let _ = w.set_always_on_top(false);
        }
        _ => {}
    }

    // Capture the starting frame in logical coordinates so the math is
    // resolution-independent across retina/non-retina displays.
    let scale = w.scale_factor().map_err(|e| e.to_string())?;
    let cur_size = w.outer_size().map_err(|e| e.to_string())?;
    let cur_pos = w.outer_position().map_err(|e| e.to_string())?;
    let from_w = cur_size.width as f64 / scale;
    // Read on every platform, but only *used* by the animation loop below,
    // which is `cfg(not(windows))`. Windows resizes in one step, so there it is
    // genuinely unused — and renaming it to `_from_h` breaks every other
    // platform's build instead of silencing one warning.
    #[cfg_attr(target_os = "windows", allow(unused_variables))]
    let from_h = cur_size.height as f64 / scale;
    let from_x = cur_pos.x as f64 / scale;
    let from_y = cur_pos.y as f64 / scale;

    // Where to end up. Compact modes keep the TOP edge glued to its current
    // position — a menu-bar popover hangs downward from the tray icon, so a
    // height change must rise/extend from the bottom. (The previous
    // center-fixed math detached the popover from the menu bar on every
    // collapse/expand.) Expanded glides to the monitor's center so the
    // bigger window doesn't shoot off-screen from the tray-anchored view.
    let (to_x, to_y) = if mode == "expanded" {
        match w.current_monitor().map_err(|e| e.to_string())? {
            Some(m) => {
                let m_size = m.size();
                let m_pos = m.position();
                let mw = m_size.width as f64 / scale;
                let mh = m_size.height as f64 / scale;
                let mx = m_pos.x as f64 / scale;
                let my = m_pos.y as f64 / scale;
                (mx + (mw - target_size.0) / 2.0, my + (mh - target_size.1) / 2.0)
            }
            None => compact_target_xy(from_x, from_y, from_w, target_size.0),
        }
    } else {
        compact_target_xy(from_x, from_y, from_w, target_size.0)
    };

    #[cfg(not(target_os = "windows"))]
    {
        // Target duration for the animation. Cubic ease-out so the motion
        // feels native (fast start, gentle settle), matching macOS
        // Control Center / window-resize timing. Time-based interpolation
        // ensures smooth animation across OS timer resolution limits.
        let start_time = std::time::Instant::now();
        let duration = std::time::Duration::from_millis(280);

        loop {
            let elapsed = start_time.elapsed();
            if elapsed >= duration {
                break;
            }
            let t = elapsed.as_secs_f64() / duration.as_secs_f64();
            let eased = 1.0 - (1.0 - t).powi(3);
            let nw = from_w + (target_size.0 - from_w) * eased;
            let nh = from_h + (target_size.1 - from_h) * eased;
            let nx = from_x + (to_x - from_x) * eased;
            let ny = from_y + (to_y - from_y) * eased;

            let _ = w.set_size(Size::Logical(LogicalSize::new(nw, nh)));
            let _ = w.set_position(Position::Logical(LogicalPosition::new(nx, ny)));
            tokio::time::sleep(std::time::Duration::from_millis(8)).await;
        }

        // Snap exactly to final target at the end.
        let _ = w.set_size(Size::Logical(LogicalSize::new(target_size.0, target_size.1)));
        let _ = w.set_position(Position::Logical(LogicalPosition::new(to_x, to_y)));

        // Compact modes re-anchor to the tray after the animation so the
        // popover lives where the user's eye expects it (this is also the
        // horizontal centering correction — the top edge is already glued by
        // compact_target_xy). Expanded was already animated to monitor
        // center, no follow-up needed.
        if tray_position_known && (mode == "compact" || mode == "compact-minimal") {
            crate::move_to_tray_center(&w);
        }
    }

    #[cfg(target_os = "windows")]
    {
        // On Windows, rapid resizing is extremely laggy. We instead animate the DOM.
        // We just need to ensure the window is large enough for the DOM animation.
        if mode == "expanded" {
            let _ = w.set_size(Size::Logical(LogicalSize::new(target_size.0, target_size.1)));
            let _ = w.set_position(Position::Logical(LogicalPosition::new(to_x, to_y)));
        } else {
            // Wait for the DOM animation (280ms) to finish before shrinking the OS window.
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                let _ = w.set_size(Size::Logical(LogicalSize::new(target_size.0, target_size.1)));
                let _ = w.set_position(Position::Logical(LogicalPosition::new(to_x, to_y)));
                if tray_position_known {
                    crate::move_to_tray_center(&w);
                }
            });
        }
    }


    Ok(())
}

#[command]
#[specta::specta]
pub async fn check_for_updates_now(app: tauri::AppHandle) -> Result<(), String> {
    crate::updater::check_and_emit(&app).await;
    Ok(())
}

#[command]
#[specta::specta]
pub async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    crate::updater::install_now(&app).await
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct AccountListEntry {
    pub slot: u32,
    pub email: String,
    /// The stable UUID that identifies this account in the SQLite `accounts`
    /// table. Pass this as `accountId` to all warmup-related Tauri commands.
    pub account_uuid: String,
    pub org_name: Option<String>,
    pub org_uuid: Option<String>,
    pub subscription_type: Option<String>,
    pub source: AddSource,
    pub is_active: bool,
    pub cached_usage: Option<CachedUsage>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SwapReport {
    pub new_active_slot: u32,
    pub running: RunningClaudeCode,
}

pub(crate) fn entry_for(
    state: &AppState,
    acc: &ManagedAccount,
    active: Option<u32>,
) -> AccountListEntry {
    let cache = state.cached_usage_by_slot.read();
    let cached = cache.get(&acc.slot).cloned();
    let last_error = cached.as_ref().and_then(|c| c.last_error.clone());

    // Prefer the live subscriptionType from the blob (which capture_live_into_slot
    // keeps current on every swap) over the snapshot stored at import time —
    // the latter goes stale when the user upgrades their plan (e.g. pro → max).
    let live_sub = acc
        .claude_code_oauth_blob
        .get("subscriptionType")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    AccountListEntry {
        slot: acc.slot,
        email: acc.email.clone(),
        account_uuid: acc.account_uuid.clone(),
        org_name: acc.organization_name.clone(),
        org_uuid: acc.organization_uuid.clone(),
        subscription_type: live_sub.or_else(|| acc.subscription_type.clone()),
        source: acc.source,
        is_active: Some(acc.slot) == active,
        cached_usage: cached,
        last_error,
    }
}

#[command]
#[specta::specta]
pub async fn list_accounts(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<AccountListEntry>, String> {
    let accounts = state.accounts.list().map_err(err_to_string)?;
    let active = *state.active_slot.read();
    Ok(accounts
        .iter()
        .map(|a| entry_for(&state, a, active))
        .collect())
}

#[command]
#[specta::specta]
pub async fn add_account_from_claude_code(
    state: State<'_, Arc<AppState>>,
) -> Result<u32, String> {
    let slot = state
        .accounts
        .add_from_claude_code()
        .await
        .map_err(err_to_string)?;
    tracing::info!(
        target: "switchboard.accounts",
        "added account from upstream-CLI (slot={slot})"
    );
    if let Err(e) = mirror_account_to_sqlite(&state, slot) {
        tracing::warn!("add_from_claude_code: SQLite mirror failed: {e:#}");
    }
    state.force_refresh.notify_one();
    Ok(slot)
}

#[command]
#[specta::specta]
pub async fn remove_account(
    slot: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    // Look up the account_uuid before removal so we can also drop the
    // SQLite mirror row. accounts.json holds the canonical mapping
    // (slot → account_uuid); after .remove() it's gone.
    let account_uuid = state
        .accounts
        .get(slot)
        .map_err(err_to_string)?
        .map(|a| a.account_uuid);

    state.accounts.remove(slot).map_err(err_to_string)?;
    state.cached_usage_by_slot.write().remove(&slot);
    state.backoff_by_slot.write().remove(&slot);
    state.schedule_by_slot.write().remove(&slot);

    if let Some(uuid) = account_uuid {
        if let Err(e) = state.db.delete_account(&uuid) {
            tracing::warn!("remove_account: SQLite delete failed: {e:#}");
        }
        tracing::info!(
            target: "switchboard.accounts",
            "removed account slot={slot} account={uuid}"
        );
    } else {
        tracing::info!(
            target: "switchboard.accounts",
            "removed account slot={slot}"
        );
    }
    Ok(())
}

#[command]
#[specta::specta]
pub async fn swap_to_account(
    slot: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<SwapReport, String> {
    tracing::info!(target: "switchboard.swap", "swap_to_account(slot={slot}) starting");
    // Refresh the target slot's token if it's expired or about to expire
    // before handing its blob to swap_to. swap_to writes whatever is
    // stored in accounts.json as the live CC credentials; if the stored
    // accessToken is already past expiry, the next poll-loop tick reads
    // those just-written creds, fetches usage, and gets 401 — surfacing
    // "token expired — re-authenticate" the instant the user switches.
    //
    // refresh_inactive is the same routine that keeps inactive slots
    // current during polling, so reusing it here keeps the refresh story
    // in one place. We swallow refresh errors: if the stored AT happens
    // to still be live, the swap still succeeds; if it's also dead, the
    // post-swap poll will surface auth_required the same way it does
    // today, and the user can re-authenticate.
    if let Ok(Some(target)) = state.accounts.get(slot) {
        let near_expiry = target.token_expires_at
            <= chrono::Utc::now() + chrono::Duration::minutes(2);
        if near_expiry {
            tracing::info!(
                target: "switchboard.swap",
                "slot {slot} stored AT near expiry; pre-refreshing before swap"
            );
            if let Err(e) = state
                .accounts
                .refresh_inactive(slot, &state.auth.exchange)
                .await
            {
                tracing::warn!("pre-swap refresh of slot {slot} failed: {e:#}");
            }
        }
    }

    state
        .accounts
        .swap_to(slot)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(target: "switchboard.swap", "swap_to_account(slot={slot}) complete");

    // swap_to commits both CC creds and the global oauthAccount blob for
    // `slot`; reconcile active_slot eagerly so the next list_accounts call
    // (the UI hits this immediately after we return) sees correct is_active
    // flags without waiting on the poll-loop tick.
    *state.active_slot.write() = Some(slot);

    // The statusline daemon's shared snapshot carries no account identity, so
    // anything it wrote before this instant describes the account we just
    // swapped away from. Without this, the new active slot adopts the previous
    // account's numbers and both rows show identical usage for up to one
    // polling interval.
    *state.active_since.write() = Some(Utc::now());

    // Drop per-slot backoff state. The previous backoff was earned by a
    // different token (the prior active slot's live CC blob, or a stale
    // OAuth refresh token) — a swap rotates which token authenticates each
    // slot's usage fetch, so prior 429s no longer apply. Without this, an
    // unlucky run of throttling can leave every slot waiting out a 30-min
    // window with no successful fetch, which strands the popover on the
    // empty LoadingShell because state.snapshot() has nothing to return.
    state.backoff_by_slot.write().clear();

    // Re-seed per-slot schedules so the new active slot polls first
    // (next_poll_at = now), with previously-active and other inactive
    // slots staggered behind it. Without this, the new active would
    // wait out whatever deadline was set when it was inactive.
    {
        let accounts = state.accounts.list().map_err(err_to_string)?;
        let (interval, base_gap) = {
            let s = state.settings.read();
            (
                std::time::Duration::from_secs(s.polling_interval_secs.max(60)),
                std::time::Duration::from_secs(s.stagger_gap_secs.clamp(5, 120)),
            )
        };
        let slot_ids: Vec<u32> = accounts.iter().map(|a| a.slot).collect();
        *state.schedule_by_slot.write() = crate::poll_loop::seed_schedules(
            &slot_ids,
            Some(slot),
            std::time::Instant::now(),
            interval,
            base_gap,
        );
    }

    if let Ok(Some(target)) = state.accounts.get(slot) {
        let prev = state.keychain_guardian.lock().replace(
            crate::auth::keychain_guardian::KeychainGuardian::arm_with_claude_code_creds(
                target.claude_code_oauth_blob.clone(),
                target.oauth_account_blob.clone(),
                target.account_uuid.clone(),
            ),
        );
        if let Some(p) = prev {
            p.cancel();
        }
    }

    let running = process_detection::detect();
    state.force_refresh.notify_one();
    Ok(SwapReport {
        new_active_slot: slot,
        running,
    })
}

#[command]
#[specta::specta]
pub async fn detect_running_claude_code() -> Result<RunningClaudeCode, String> {
    Ok(process_detection::detect())
}

#[command]
#[specta::specta]
pub async fn refresh_account(
    slot: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let active = *state.active_slot.read();
    if Some(slot) == active {
        state.force_refresh.notify_one();
        return Ok(());
    }
    state
        .accounts
        .refresh_inactive(slot, &state.auth.exchange)
        .await
        .map_err(err_to_string)?;
    state.force_refresh.notify_one();
    Ok(())
}

// ---------------------------------------------------------------------------
// Warmup pillar commands (Plan B T16)
// ---------------------------------------------------------------------------

/// Trigger a manual warm-up for a specific account (UI "Warm up now" button).
#[command]
#[specta::specta]
pub async fn warmup_account_now(
    state: State<'_, Arc<AppState>>,
    account_id: String,
) -> Result<crate::warmup::errors::WarmupOutcome, String> {
    ensure_sqlite_account_row(&state, &account_id).map_err(|e| e.to_string())?;
    crate::scheduler_glue::manual_warmup(state.inner(), &account_id)
        .await
        .map_err(|e| format!("{e:#}"))
}

/// Set the per-account schedule preset.
#[command]
#[specta::specta]
pub async fn set_account_schedule(
    state: State<'_, Arc<AppState>>,
    account_id: String,
    schedule: crate::scheduler::Schedule,
) -> Result<(), String> {
    ensure_sqlite_account_row(&state, &account_id).map_err(|e| e.to_string())?;
    let conn = state.db.conn();
    let json = serde_json::to_string(&schedule).map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE accounts SET schedule = ?1 WHERE id = ?2",
        rusqlite::params![json, account_id],
    )
    .map_err(|e| e.to_string())?;
    tracing::info!(
        target: "switchboard.warmup",
        "set_account_schedule({account_id}, {schedule:?})"
    );
    Ok(())
}

/// Toggle warm-up on/off for a specific account.
#[command]
#[specta::specta]
pub async fn set_warmup_enabled(
    state: State<'_, Arc<AppState>>,
    account_id: String,
    enabled: bool,
) -> Result<(), String> {
    ensure_sqlite_account_row(&state, &account_id).map_err(|e| e.to_string())?;
    let conn = state.db.conn();
    conn.execute(
        "UPDATE accounts SET warmup_enabled = ?1 WHERE id = ?2",
        rusqlite::params![enabled as i64, account_id],
    )
    .map_err(|e| e.to_string())?;
    tracing::info!(
        target: "switchboard.warmup",
        "set_warmup_enabled({account_id}, {enabled})"
    );
    Ok(())
}

/// Grant the global warm-up consent (called by WarmupConsentModal on Accept).
#[command]
#[specta::specta]
pub async fn grant_warmup_consent(
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let conn = state.db.conn();
    conn.execute(
        "UPDATE settings SET value = '1' WHERE key = 'warmup_consent_granted'",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Revoke global consent (also disables warm-up on every account).
#[command]
#[specta::specta]
pub async fn revoke_warmup_consent(
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let conn = state.db.conn();
    conn.execute_batch(
        "UPDATE settings SET value = '0' WHERE key = 'warmup_consent_granted'; \
         UPDATE accounts SET warmup_enabled = 0;",
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Read the consent flag.
#[command]
#[specta::specta]
pub async fn get_warmup_consent_granted(
    state: State<'_, Arc<AppState>>,
) -> Result<bool, String> {
    let conn = state.db.conn();
    let v: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'warmup_consent_granted'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(v == "1")
}

/// Register OS-level scheduler (writes plist / schtasks task).
#[command]
#[specta::specta]
pub async fn os_scheduler_register() -> Result<(), String> {
    let bin = std::env::current_exe().map_err(|e| e.to_string())?;
    let s = crate::os_scheduler::for_current_platform()
        .ok_or_else(|| "OS-level scheduling not supported on this platform".to_string())?;
    s.register(&bin).map_err(|e| format!("{e:#}"))
}

/// Unregister OS-level scheduler.
#[command]
#[specta::specta]
pub async fn os_scheduler_unregister() -> Result<(), String> {
    let s = crate::os_scheduler::for_current_platform()
        .ok_or_else(|| "OS-level scheduling not supported on this platform".to_string())?;
    s.unregister().map_err(|e| format!("{e:#}"))
}

/// Check if OS-level scheduler is currently registered.
#[command]
#[specta::specta]
pub async fn os_scheduler_is_registered() -> Result<bool, String> {
    let s = crate::os_scheduler::for_current_platform()
        .ok_or_else(|| "OS-level scheduling not supported on this platform".to_string())?;
    s.is_registered().map_err(|e| format!("{e:#}"))
}

/// Per-account warm-up state returned by `get_warmup_state`.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct WarmupAccountState {
    pub warmup_enabled: bool,
    pub schedule: crate::scheduler::Schedule,
    pub last_warmup_at: Option<i64>,
}

/// Fetch the warm-up state for a specific account. Used by the UI row to
/// initialise the WarmupToggle / ScheduleSelector on mount.
#[command]
#[specta::specta]
pub async fn get_warmup_state(
    state: State<'_, Arc<AppState>>,
    account_id: String,
) -> Result<WarmupAccountState, String> {
    // Make sure a row exists so a brand-new account doesn't bubble a
    // "no rows returned" error up to the UI on mount.
    ensure_sqlite_account_row(&state, &account_id).map_err(|e| e.to_string())?;

    let conn = state.db.conn();
    let (enabled, schedule_json, last_warmup_at): (i64, String, Option<i64>) = conn
        .query_row(
            "SELECT warmup_enabled, schedule, last_warmup_at FROM accounts WHERE id = ?1",
            rusqlite::params![account_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|e| e.to_string())?;
    let schedule: crate::scheduler::Schedule =
        serde_json::from_str(&schedule_json).map_err(|e| e.to_string())?;
    Ok(WarmupAccountState {
        warmup_enabled: enabled != 0,
        schedule,
        last_warmup_at,
    })
}

// ---------------------------------------------------------------------------
// SQLite mirror helpers
//
// `accounts.json` (managed by AccountManager) is the canonical source of
// "which accounts exist." The SQLite `accounts` table is a sidecar that holds
// warm-up state (warmup_enabled, schedule, last_warmup_at) — split from the
// JSON store because the transactional claim in scheduler::claim needs SQL,
// and we deliberately keep credential blobs out of the DB.
//
// These helpers keep the two stores in sync.
// ---------------------------------------------------------------------------

/// Insert (or no-op-update) the SQLite mirror row for a slot. Called after
/// every successful AccountManager add so warm-up queries can find a row.
pub(crate) fn mirror_account_to_sqlite(
    state: &Arc<AppState>,
    slot: u32,
) -> anyhow::Result<()> {
    let acc = state
        .accounts
        .get(slot)?
        .ok_or_else(|| anyhow::anyhow!("slot {slot} not in accounts.json"))?;
    state.db.upsert_account(&crate::store::StoredAccount {
        id: acc.account_uuid,
        email: acc.email,
        display_name: None,
    })?;
    Ok(())
}

/// Ensure a SQLite row exists for `account_uuid`. Used as a defensive guard
/// at the top of every warm-up command so a missing mirror row (e.g. account
/// added before this fix shipped, or mirror failed asynchronously) doesn't
/// cause `set_warmup_enabled` to silently match 0 rows or `load_schedule` to
/// return `QueryReturnedNoRows`.
fn ensure_sqlite_account_row(
    state: &State<'_, Arc<AppState>>,
    account_uuid: &str,
) -> anyhow::Result<()> {
    let accounts = state.accounts.list()?;
    let acc = accounts
        .into_iter()
        .find(|a| a.account_uuid == account_uuid)
        .ok_or_else(|| {
            anyhow::anyhow!("account {account_uuid} not found in accounts.json")
        })?;
    state.db.upsert_account(&crate::store::StoredAccount {
        id: acc.account_uuid,
        email: acc.email,
        display_name: None,
    })?;
    Ok(())
}

/// Reconcile every account in accounts.json into the SQLite mirror at
/// startup. Idempotent — existing rows keep their warm-up state because
/// upsert_account's ON CONFLICT clause only touches email/display_name/
/// last_seen_at, not the warm-up columns.
pub fn reconcile_sqlite_account_mirror(state: &Arc<AppState>) -> anyhow::Result<()> {
    let accounts = state.accounts.list()?;
    for acc in accounts {
        if let Err(e) = state.db.upsert_account(&crate::store::StoredAccount {
            id: acc.account_uuid.clone(),
            email: acc.email.clone(),
            display_name: None,
        }) {
            tracing::warn!(
                "reconcile_sqlite_account_mirror: failed for {}: {e:#}",
                acc.account_uuid
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_resize_keeps_top_edge_glued() {
        // Collapsing 380 → 208 must not move the top edge — a menu-bar
        // popover hangs from the tray; center-fixed math visibly detaches it.
        let (_, y) = compact_target_xy(100.0, 24.0, 360.0, 360.0);
        assert_eq!(y, 24.0);
    }

    #[test]
    fn warmup_suggestion_returns_none_below_the_active_day_floor() {
        // 9 distinct active days, floor is 10 — must not suggest yet.
        let events: Vec<StoredSessionEvent> = (0..9)
            .map(|i| test_event(Utc::now() - Duration::days(i)))
            .collect();
        assert!(compute_warmup_suggestion(&events, 10).is_none());
    }

    #[test]
    fn warmup_suggestion_fires_at_exactly_the_floor() {
        let events: Vec<StoredSessionEvent> = (0..10)
            .map(|i| test_event(Utc::now() - Duration::days(i)))
            .collect();
        assert!(compute_warmup_suggestion(&events, 10).is_some());
    }

    #[test]
    fn warmup_suggestion_takes_only_the_earliest_event_per_day() {
        // Two active days; each day has a late event and an early event —
        // the median must be computed from the early ones only.
        let day0 = Utc::now();
        let day1 = Utc::now() - Duration::days(1);
        let events = vec![
            test_event(day0 + Duration::hours(5)), // late on day0
            test_event(day0),                      // early on day0
            test_event(day1 + Duration::hours(5)), // late on day1
            test_event(day1),                      // early on day1
        ];
        let suggestion = compute_warmup_suggestion(&events, 2).unwrap();
        assert_eq!(suggestion.active_days, 2);

        let expected_minutes: Vec<u32> = [day0, day1]
            .iter()
            .map(|d| {
                let local = d.with_timezone(&chrono::Local);
                use chrono::Timelike;
                local.hour() * 60 + local.minute()
            })
            .collect();
        let mut sorted = expected_minutes.clone();
        sorted.sort_unstable();
        let expected_median = sorted[sorted.len() / 2];
        let got_minutes = suggestion.anchor.hour as u32 * 60 + suggestion.anchor.minute as u32;
        assert_eq!(got_minutes, expected_median);
    }

    #[test]
    fn daily_trends_csv_has_header_and_one_row_per_bucket() {
        let buckets = vec![
            DailyBucket {
                date: "2026-08-01".into(),
                input_tokens: 1000,
                output_tokens: 500,
                cost_usd: 1.5,
                request_count: 3,
            },
            DailyBucket {
                date: "2026-08-02".into(),
                input_tokens: 2000,
                output_tokens: 0,
                cost_usd: 0.25,
                request_count: 1,
            },
        ];

        let csv = daily_trends_to_csv(&buckets);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "date,input_tokens,output_tokens,cost_usd,request_count");
        assert_eq!(lines[1], "2026-08-01,1000,500,1.5,3");
        assert_eq!(lines[2], "2026-08-02,2000,0,0.25,1");
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn daily_trends_csv_is_just_the_header_when_empty() {
        let csv = daily_trends_to_csv(&[]);
        assert_eq!(csv.trim_end(), "date,input_tokens,output_tokens,cost_usd,request_count");
    }

    #[test]
    fn daily_trends_counts_requests_and_sums_cost_per_day() {
        let day0 = Utc::now();
        let day1 = day0 - Duration::days(1);
        let events = vec![
            test_event(day0),
            test_event(day0 + Duration::hours(1)),
            test_event(day1),
        ];
        let buckets = bucket_daily_trends(&events);

        let date0 = day0.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string();
        let date1 = day1.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string();

        let bucket0 = buckets.iter().find(|b| b.date == date0).unwrap();
        assert_eq!(bucket0.request_count, 2);
        assert_eq!(bucket0.input_tokens, 2);

        let bucket1 = buckets.iter().find(|b| b.date == date1).unwrap();
        assert_eq!(bucket1.request_count, 1);
    }

    fn test_event(ts: chrono::DateTime<Utc>) -> StoredSessionEvent {
        StoredSessionEvent {
            ts,
            project: "p".into(),
            model: "m".into(),
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 0.0,
            source_file: "/a.jsonl".into(),
            source_line: 1,
            event_id: format!("evt-{}", ts.timestamp_nanos_opt().unwrap()),
        }
    }

    #[test]
    fn compact_resize_recenters_width() {
        // If the width ever changes, the window recenters horizontally…
        let (x, _) = compact_target_xy(100.0, 24.0, 400.0, 360.0);
        assert_eq!(x, 120.0);
        // …but at equal widths x is untouched (the common case).
        let (x, _) = compact_target_xy(100.0, 24.0, 360.0, 360.0);
        assert_eq!(x, 100.0);
    }
}

// ---------------------------------------------------------------------------
// Custom model providers
// ---------------------------------------------------------------------------

use crate::providers::launcher::{self, LaunchSpec, LaunchSurface, Terminal};
use crate::providers::model::Provider;
use crate::providers::presets::{self, PresetInfo};
use crate::providers::{default_env, DefaultProviderState};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum SetDefaultOutcome {
    Applied,
    /// `settings.json` already carries provider env we do not own. The UI
    /// must confirm before we overwrite hand-written configuration.
    NeedsConfirmation { unmanaged_keys: Vec<String> },
}

fn claude_settings_path() -> Result<PathBuf, String> {
    crate::auth::paths::claude_config_home()
        .map(|d| d.join("settings.json"))
        .ok_or_else(|| "could not resolve the Claude config directory".to_string())
}

#[command]
#[specta::specta]
pub async fn list_providers(state: State<'_, Arc<AppState>>) -> Result<Vec<Provider>, String> {
    state.db.list_providers().map_err(|e| e.to_string())
}

#[command]
#[specta::specta]
pub async fn upsert_provider(
    provider: Provider,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    state.db.upsert_provider(&provider).map_err(|e| e.to_string())
}

#[command]
#[specta::specta]
pub async fn delete_provider(id: String, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    // Deleting the active default would leave settings.json holding orphaned
    // env, so undo the default first. `ON DELETE SET NULL` is only a backstop:
    // a nulled provider_id beside a populated managed_env is an orphaned
    // manifest with no UI path to undo the mutation it describes.
    if let Ok(Some(d)) = state.db.get_default_provider() {
        if d.provider_id == id {
            let path = claude_settings_path()?;
            // What we wrote is the provider's own resolved env — §4.1 requires
            // every edit to an active default to go through clear-then-apply,
            // so the stored manifest always corresponds to the current row.
            let written = state
                .db
                .get_provider(&id)
                .map_err(|e| e.to_string())?
                .map(|p| p.resolved_env())
                .unwrap_or_default();
            default_env::clear(&path, &d.managed_env, &written).map_err(|e| e.to_string())?;
            state.db.clear_default_provider().map_err(|e| e.to_string())?;
        }
    }
    state.db.delete_provider(&id).map_err(|e| e.to_string())
}

#[command]
#[specta::specta]
pub async fn list_provider_presets() -> Result<Vec<PresetInfo>, String> {
    Ok(presets::all_info())
}

#[command]
#[specta::specta]
pub async fn list_available_terminals() -> Result<Vec<Terminal>, String> {
    Ok(launcher::available_terminals())
}

#[command]
#[specta::specta]
/// `surface` is optional so callers written before VS Code tabs existed keep
/// launching into a terminal.
pub async fn launch_provider_session(
    provider_id: String,
    cwd: String,
    terminal: Terminal,
    resume_session_id: Option<String>,
    permission_mode: Option<String>,
    surface: Option<LaunchSurface>,
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    let provider = state
        .db
        .get_provider(&provider_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no provider with id {provider_id}"))?;
    let spec = LaunchSpec {
        provider,
        cwd: PathBuf::from(cwd),
        terminal,
        resume_session_id,
        permission_mode,
        surface: surface.unwrap_or_default(),
    };
    launcher::launch(&spec).map_err(|e| format!("{e:#}"))
}

/// Whether a VS Code tab can be offered at all: both the `code` CLI and the
/// Claude Code extension have to be present. Probed at settings time so an
/// unavailable choice surfaces before the user commits to a session.
#[command]
#[specta::specta]
pub async fn vscode_tab_available() -> Result<bool, String> {
    Ok(launcher::vscode::is_available())
}

/// The shell one-liner equivalent of a launch, for users whose terminal we
/// do not drive. Deliberately returns the script path rather than inlining
/// secrets into a string the user will paste somewhere.
#[command]
#[specta::specta]
pub async fn get_provider_launch_command(
    provider_id: String,
    cwd: String,
    terminal: Terminal,
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    let provider = state
        .db
        .get_provider(&provider_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no provider with id {provider_id}"))?;
    let spec = LaunchSpec {
        provider,
        cwd: PathBuf::from(cwd.clone()),
        terminal,
        resume_session_id: None,
        permission_mode: None,
        // A copyable one-liner is inherently a shell command, so this path is
        // terminal-only regardless of the configured surface.
        surface: LaunchSurface::Terminal,
    };
    let script =
        launcher::write_script(&spec, &launcher::script_dir()).map_err(|e| format!("{e:#}"))?;
    let (program, args) = launcher::build_command(terminal, &script, &PathBuf::from(cwd));
    Ok(format!("{program} {}", args.join(" ")))
}

#[command]
#[specta::specta]
pub async fn get_default_provider(
    state: State<'_, Arc<AppState>>,
) -> Result<Option<DefaultProviderState>, String> {
    state.db.get_default_provider().map_err(|e| e.to_string())
}

#[command]
#[specta::specta]
pub async fn set_default_provider(
    provider_id: String,
    force: bool,
    state: State<'_, Arc<AppState>>,
) -> Result<SetDefaultOutcome, String> {
    let provider = state
        .db
        .get_provider(&provider_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no provider with id {provider_id}"))?;
    let path = claude_settings_path()?;
    let prev = state.db.get_default_provider().map_err(|e| e.to_string())?;

    // ORDER IS LOAD-BEARING (spec §4.1): the foreign-key check must run
    // BEFORE clear(prev), against the *previous* manifest.
    //
    // Clearing first restores the user's original values into the file, which
    // the check would then report as foreign — so every ordinary A→B switch
    // would prompt, and declining would leave the previous default already
    // undone with no record of it.
    if !force {
        let known = prev.as_ref().map(|p| p.managed_env.clone()).unwrap_or_default();
        let foreign =
            default_env::foreign_provider_keys(&path, &known).map_err(|e| e.to_string())?;
        if !foreign.is_empty() {
            return Ok(SetDefaultOutcome::NeedsConfirmation { unmanaged_keys: foreign });
        }
    }

    // Only now undo the previous default, so its keys cannot linger and the
    // new manifest is computed against the user's true prior state (§4.1).
    if let Some(prev) = prev {
        let prev_written = state
            .db
            .get_provider(&prev.provider_id)
            .map_err(|e| e.to_string())?
            .map(|p| p.resolved_env())
            .unwrap_or_default();
        default_env::clear(&path, &prev.managed_env, &prev_written).map_err(|e| e.to_string())?;
    }

    let env = provider.resolved_env();
    let manifest = default_env::apply(&path, &env).map_err(|e| e.to_string())?;
    state
        .db
        .set_default_provider(&provider_id, &manifest, Utc::now().timestamp())
        .map_err(|e| e.to_string())?;
    Ok(SetDefaultOutcome::Applied)
}

/// Returns the keys left untouched because the user edited them by hand while
/// the default was active (§4.2) — the UI reports these rather than silently
/// reverting someone's edit.
#[command]
#[specta::specta]
pub async fn clear_default_provider(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<String>, String> {
    let Some(d) = state.db.get_default_provider().map_err(|e| e.to_string())? else {
        return Ok(Vec::new());
    };
    let path = claude_settings_path()?;
    let written = state
        .db
        .get_provider(&d.provider_id)
        .map_err(|e| e.to_string())?
        .map(|p| p.resolved_env())
        .unwrap_or_default();
    let skipped = default_env::clear(&path, &d.managed_env, &written).map_err(|e| e.to_string())?;
    state.db.clear_default_provider().map_err(|e| e.to_string())?;
    Ok(skipped)
}

#[derive(Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum InstallStatuslineOutcome {
    Applied,
    /// `settings.json` already carries a `statusLine` we do not own. The UI
    /// must confirm before we overwrite hand-written (or another tool's)
    /// configuration.
    NeedsConfirmation { foreign_value: serde_json::Value },
}

#[command]
#[specta::specta]
pub async fn get_statusline_install_state(
    state: State<'_, Arc<AppState>>,
) -> Result<Option<crate::statusline_installer::StatuslineInstallState>, String> {
    Ok(state
        .db
        .get_statusline_install()
        .map_err(|e| e.to_string())?
        .map(|(s, _)| s))
}

#[command]
#[specta::specta]
pub async fn install_statusline(
    force: bool,
    state: State<'_, Arc<AppState>>,
) -> Result<InstallStatuslineOutcome, String> {
    let path = claude_settings_path()?;
    let existing = state.db.get_statusline_install().map_err(|e| e.to_string())?;

    // ORDER IS LOAD-BEARING, same reasoning as set_default_provider (spec
    // §4.1 of the providers feature): the foreign-value check must run
    // BEFORE clearing any previous Switchboard-owned value. Clearing first
    // would restore the pre-Switchboard value into the file, which the
    // check would then misreport as foreign.
    if !force {
        let ours = existing.as_ref().map(|(s, _)| s.installed_command.as_str());
        if let Some(foreign) = crate::statusline_installer::foreign_statusline(&path, ours)
            .map_err(|e| e.to_string())?
        {
            return Ok(InstallStatuslineOutcome::NeedsConfirmation { foreign_value: foreign });
        }
    }

    if let Some((prev_state, prev_prior)) = existing {
        let written = serde_json::json!({ "type": "command", "command": prev_state.installed_command });
        crate::statusline_installer::clear(&path, &prev_prior, &written).map_err(|e| e.to_string())?;
    }

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let command = format!("\"{}\" statusline", exe.display());
    let prior = crate::statusline_installer::apply(&path, &command).map_err(|e| e.to_string())?;
    state
        .db
        .set_statusline_install(&prior, &command, Utc::now().timestamp())
        .map_err(|e| e.to_string())?;
    Ok(InstallStatuslineOutcome::Applied)
}

#[command]
#[specta::specta]
pub async fn uninstall_statusline(state: State<'_, Arc<AppState>>) -> Result<bool, String> {
    let Some((install_state, prior)) = state.db.get_statusline_install().map_err(|e| e.to_string())? else {
        return Ok(true);
    };
    let path = claude_settings_path()?;
    let written = serde_json::json!({ "type": "command", "command": install_state.installed_command });
    let ok = crate::statusline_installer::clear(&path, &prior, &written).map_err(|e| e.to_string())?;
    if ok {
        state.db.clear_statusline_install().map_err(|e| e.to_string())?;
    }
    Ok(ok)
}

// ---------------------------------------------------------------------------
// Session browser
// ---------------------------------------------------------------------------

use crate::sessions::{recap, scan, SessionSummary};

const MAX_SESSIONS: usize = 200;

#[command]
#[specta::specta]
pub async fn list_resumable_sessions(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<SessionSummary>, String> {
    let Some(root) = crate::jsonl_parser::walker::claude_projects_root() else {
        return Ok(Vec::new());
    };
    let files = scan::discover_session_files(&root);

    // Newest mtime is the cache key: any new or appended transcript advances it.
    let newest = files
        .first()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
        .unwrap_or(std::time::UNIX_EPOCH);

    if let Some((cached_at, rows)) = state.sessions_cache.read().as_ref() {
        if *cached_at == newest {
            return Ok(rows.clone());
        }
    }

    // The cap applies to sessions AFTER filtering, not files scanned — a
    // pre-filter cap would let one session's subagent transcripts evict real
    // sessions. Scanning is cheap; the result list is what needs bounding.
    // One grouped scan of session_events for the whole list — a per-session
    // query would be 200 round-trips to show one column.
    let totals = state.db.session_totals().unwrap_or_default();

    let mut rows: Vec<SessionSummary> = Vec::new();
    for f in &files {
        if let Some(mut s) = recap::parse_session(f) {
            // `session_events.source_file` is stored relative to the projects
            // root (P1-7, so the value is not machine-specific), so the
            // lookup key has to be built the same way.
            if let Some((tokens, cost)) = f
                .strip_prefix(&root)
                .ok()
                .and_then(|rel| totals.get(rel.to_string_lossy().as_ref()))
            {
                s.total_tokens = *tokens;
                s.total_cost_usd = *cost;
            }
            rows.push(s);
            if rows.len() >= MAX_SESSIONS {
                break;
            }
        }
    }

    *state.sessions_cache.write() = Some((newest, rows.clone()));
    Ok(rows)
}
