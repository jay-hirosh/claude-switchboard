use crate::app_state::{AppState, BackoffState, BurnRateProjection, CachedUsage, ExtraBurnRate};
use crate::auth::AuthSource;
use crate::auth::accounts::ManagedAccount;
use crate::notifier;
use crate::notifier::rules::Bucket;
use crate::tray;
use crate::usage_api::{FetchOutcome, UsageSnapshot, Utilization};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

static STALE_EMITTED: AtomicBool = AtomicBool::new(false);

/// Default gap between consecutive slot polls in a staggered round when
/// the user hasn't customised `Settings::stagger_gap_secs`. Mirrors the
/// `Settings::default()` value so a non-test caller that bypasses settings
/// still gets the historical behaviour.
pub const DEFAULT_STAGGER_GAP_SECS: u64 = 30;

/// Pull the polling interval + base stagger gap from settings as Durations,
/// clamping each to a safe lower bound that mirrors the validation in
/// `update_settings` (defends against any settings row written before that
/// validation existed).
fn settings_durations(state: &crate::app_state::AppState) -> (Duration, Duration) {
    let s = state.settings.read();
    (
        Duration::from_secs(s.polling_interval_secs.max(60)),
        Duration::from_secs(s.stagger_gap_secs.clamp(5, 120)),
    )
}

/// Compute the per-slot stagger gap. Compresses below `base_gap` when the
/// configured polling interval can't fit (`slots * base_gap`).
pub fn stagger_gap(slot_count: usize, interval: Duration, base_gap: Duration) -> Duration {
    if slot_count == 0 {
        return base_gap;
    }
    let max_total = interval;
    let needed_total = base_gap * (slot_count.saturating_sub(1) as u32);
    if needed_total <= max_total {
        base_gap
    } else {
        max_total / slot_count as u32
    }
}

/// Lay out per-slot poll deadlines so the active slot fires first and
/// inactive slots trail at fixed offsets. Slot-id ordering for inactive
/// slots makes the schedule deterministic regardless of input order.
pub fn seed_schedules(
    slots: &[u32],
    active_slot: Option<u32>,
    now: Instant,
    interval: Duration,
    base_gap: Duration,
) -> HashMap<u32, crate::app_state::ScheduleState> {
    use crate::app_state::ScheduleState;

    let gap = stagger_gap(slots.len(), interval, base_gap);
    let mut ordered: Vec<u32> = match active_slot {
        Some(active) if slots.contains(&active) => {
            let mut v = vec![active];
            let mut rest: Vec<u32> = slots.iter().copied().filter(|&s| s != active).collect();
            rest.sort_unstable();
            v.extend(rest);
            v
        }
        _ => {
            let mut v: Vec<u32> = slots.to_vec();
            v.sort_unstable();
            v
        }
    };
    ordered.dedup();

    ordered
        .into_iter()
        .enumerate()
        .map(|(i, slot)| {
            let next_poll_at = now + gap * (i as u32);
            (slot, ScheduleState { next_poll_at })
        })
        .collect()
}

/// Choose the slot with the earliest already-expired `next_poll_at`,
/// skipping any slot currently in 429 backoff. Returns None when no
/// slot is ready to fetch.
pub fn pick_due_slot(state: &crate::app_state::AppState, now: Instant) -> Option<u32> {
    let schedule = state.schedule_by_slot.read();
    let backoff = state.backoff_by_slot.read();
    schedule
        .iter()
        .filter(|(_slot, sched)| sched.next_poll_at <= now)
        .filter(|(slot, _sched)| backoff.get(slot).is_none_or(|b| now >= b.until))
        .min_by_key(|(_slot, sched)| sched.next_poll_at)
        .map(|(&slot, _)| slot)
}

/// Earliest future deadline across all scheduled slots, used to pick a
/// sleep target when nothing is currently due. Falls back to 60 s out
/// when the schedule is empty (e.g., before any account is added).
pub fn next_wake_time(state: &crate::app_state::AppState, now: Instant) -> Instant {
    let schedule = state.schedule_by_slot.read();
    schedule
        .values()
        .map(|s| s.next_poll_at)
        .min()
        .unwrap_or(now + Duration::from_secs(60))
}

/// Per-slot burn-rate sample buffers. Bundled into one struct (rather than
/// two parallel HashMaps) so threading the pair through `poll_all` →
/// `fetch_and_apply_one` → `apply_fetch_outcome` doesn't explode each
/// signature with an extra parameter.
#[derive(Default)]
pub struct SlotBurnBuffers {
    five_hour: VecDeque<(DateTime<Utc>, f64)>,
    seven_day: VecDeque<(DateTime<Utc>, f64)>,
    extra: VecDeque<(DateTime<Utc>, f64)>,
}

pub fn spawn(handle: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let mut burn_buffers: HashMap<u32, SlotBurnBuffers> = HashMap::new();
        loop {
            let _ = poll_all(&handle, &state, &mut burn_buffers).await;
            let now = Instant::now();
            let wake_at = next_wake_time(&state, now);
            // Cap the sleep at 60 s so we still re-reconcile active_slot
            // periodically even when no slot is due (covers the
            // "live creds change without a swap" path).
            let max_sleep = Duration::from_secs(60);
            let sleep_for = wake_at.saturating_duration_since(now).min(max_sleep);
            tokio::select! {
                _ = tokio::time::sleep(sleep_for) => {}
                _ = state.force_refresh.notified() => {}
            }
        }
    });
}

async fn fetch_and_apply_one(
    handle: &AppHandle,
    state: &AppState,
    burn_buffers: &mut HashMap<u32, SlotBurnBuffers>,
    slot: u32,
    accounts: &[ManagedAccount],
) {
    let acc = match accounts.iter().find(|a| a.slot == slot).cloned() {
        Some(a) => a,
        None => return, // slot disappeared between poll_all's accounts read and now
    };
    // Re-read active_slot here; in the rare race where swap_to_account
    // fires between poll_all's reconciliation and this point, we accept
    // a one-cycle cosmetic flicker on tray/auth_source rather than
    // adding a lock or capturing the value at the call site.
    let active_slot = *state.active_slot.read();

    // Shared-snapshot fast path (active slot only): when an external poller
    // — the user's statusline daemon — already has fresh data for the live
    // account, adopt it instead of spending the account's scarce /usage rate
    // budget on a duplicate call. Valid only for the active slot because the
    // daemon polls the live Claude Code account, which is what active_slot
    // tracks; inactive slots keep their normal (uncontended) fetch path.
    // Freshness window = the polling interval, so a healthy daemon always
    // wins and a dead one hands control back to the HTTP path within a cycle.
    if Some(slot) == active_slot {
        let interval = Duration::from_secs(state.settings.read().polling_interval_secs.max(60));
        let active_since = *state.active_since.read();
        if let Some(snap) = read_shared_snapshot(&shared_usage_file_path(), interval, active_since)
        {
            tracing::debug!(target: "switchboard.poll", "slot {slot}: adopted shared snapshot");
            return self::apply_fetch_outcome(
                handle,
                state,
                burn_buffers,
                slot,
                &acc,
                active_slot,
                FetchOutcome::Ok(snap),
            )
            .await;
        }
    }

    let token_result = state
        .auth
        .token_for_slot(slot, active_slot, &state.accounts)
        .await;
    let outcome = match token_result {
        Ok(tok) => Some(state.usage.fetch(&tok).await),
        Err(e) => {
            tracing::warn!("token_for_slot({slot}) failed: {e}");
            let _ = handle.emit(
                "auth_required_for_slot",
                json!({ "slot": slot, "email": acc.email }),
            );
            // Most common cause is the refresh token being revoked
            // (Anthropic returns invalid_grant). Mark the slot's cache
            // with auth_required so the UI shows "token expired —
            // re-authenticate" without waiting for a manual refresh.
            // Network blips fall under the same label here; the next
            // successful refresh will clear last_error.
            let mut entry = state
                .cached_usage_by_slot
                .write()
                .remove(&slot)
                .unwrap_or_else(|| placeholder_cached(&acc, "auth_required"));
            entry.last_error = Some("auth_required".into());
            state.cached_usage_by_slot.write().insert(slot, entry.clone());
            let _ = handle.emit(
                "usage_updated",
                json!({ "slot": slot, "cached": entry }),
            );
            return;
        }
    };
    let Some(outcome) = outcome else { return };
    apply_fetch_outcome(handle, state, burn_buffers, slot, &acc, active_slot, outcome).await;
}

/// Apply a fetch outcome for one slot: update the per-slot cache, emit
/// `usage_updated`, and on success also persist + update tray/notifier.
/// Shared by the HTTP path and the shared-snapshot fast path.
#[allow(clippy::too_many_arguments)]
async fn apply_fetch_outcome(
    handle: &AppHandle,
    state: &AppState,
    burn_buffers: &mut HashMap<u32, SlotBurnBuffers>,
    slot: u32,
    acc: &ManagedAccount,
    active_slot: Option<u32>,
    outcome: FetchOutcome,
) {
    match outcome {
        FetchOutcome::Ok(snapshot) => {
            let bufs = burn_buffers.entry(slot).or_default();
            let now = Utc::now();
            let burn_rate = update_burn_rate(
                &mut bufs.five_hour,
                snapshot.five_hour.as_ref(),
                ChronoDuration::hours(5),
                2.0,
                now,
            );
            let seven_day_burn_rate = update_burn_rate(
                &mut bufs.seven_day,
                snapshot.seven_day.as_ref(),
                ChronoDuration::days(7),
                60.0,
                now,
            );
            let extra_burn_rate = update_extra_burn_rate(&mut bufs.extra, &snapshot, now);
            let cached = CachedUsage {
                snapshot: snapshot.clone(),
                account_id: acc.account_uuid.clone(),
                account_email: acc.email.clone(),
                last_error: None,
                burn_rate,
                seven_day_burn_rate,
                extra_burn_rate,
                auth_source: if Some(slot) == active_slot {
                    AuthSource::ClaudeCode
                } else {
                    AuthSource::OAuth
                },
            };
            state.cached_usage_by_slot.write().insert(slot, cached.clone());
            state.backoff_by_slot.write().remove(&slot);
            // Persist the raw snapshot so a cold start can rehydrate
            // last-known-good data (see hydrated_caches). Best-effort: a
            // storage hiccup must never interrupt polling. Note the
            // re-serialize drops forward-compat `unknown` fields — the UI
            // and hydration only rely on the typed buckets.
            match serde_json::to_string(&snapshot) {
                Ok(payload) => {
                    if let Err(e) =
                        state.db.insert_snapshot(&acc.account_uuid, Utc::now(), &payload)
                    {
                        tracing::warn!("persist snapshot for slot {slot} failed: {e}");
                    }
                }
                Err(e) => tracing::warn!("serialize snapshot for slot {slot} failed: {e}"),
            }
            // Track per-window peaks for the limit-hit analytics report
            // (F5). Runs for every slot, not just the active one — the
            // report covers every managed account. Best-effort, same
            // rationale as insert_snapshot above: a storage hiccup must
            // never interrupt polling.
            for (bucket, data) in [
                (Bucket::FiveHour, snapshot.five_hour.as_ref()),
                (Bucket::SevenDay, snapshot.seven_day.as_ref()),
            ] {
                let Some(u) = data else { continue };
                let Some(resets_at) = u.resets_at else { continue };
                if let Err(e) = state.db.record_window_peak(
                    &acc.account_uuid,
                    bucket.label(),
                    resets_at,
                    Utc::now(),
                    u.utilization,
                ) {
                    tracing::warn!("record_window_peak for slot {slot} ({}) failed: {e}", bucket.label());
                }
            }
            let _ = handle.emit(
                "usage_updated",
                json!({ "slot": slot, "cached": cached }),
            );

            if Some(slot) == active_slot {
                if let Err(e) = write_shared_snapshot(&shared_usage_file_path(), &snapshot) {
                    tracing::warn!("write_shared_snapshot failed: {e:#}");
                }
                *state.cached_usage.write() = Some(cached.clone());
                tray::set_level(
                    handle,
                    snapshot.five_hour.as_ref().map(|u| u.utilization),
                    snapshot.seven_day.as_ref().map(|u| u.utilization),
                    snapshot.five_hour.as_ref().and_then(|u| u.resets_at),
                    snapshot.seven_day.as_ref().and_then(|u| u.resets_at),
                    false,
                );
                let (thresholds, payg_threshold) = {
                    let s = state.settings.read();
                    (s.thresholds.clone(), s.payg_threshold)
                };
                #[cfg(target_os = "macos")]
                {
                    let poll_interval_secs = state.settings.read().polling_interval_secs;
                    let ws = crate::widget_snapshot::build(acc, &snapshot, poll_interval_secs, Utc::now());
                    if let Err(e) =
                        crate::widget_snapshot::write(&crate::widget_snapshot::container_dir(), &ws)
                    {
                        tracing::warn!("widget snapshot write failed: {e}");
                    }
                }
                if let Ok(fired) = notifier::evaluate(
                    &state.db,
                    &cached.account_id,
                    &snapshot,
                    &thresholds,
                    payg_threshold,
                    Utc::now(),
                ) {
                    for f in fired {
                        use tauri_plugin_notification::NotificationExt;
                        let _ = handle
                            .notification()
                            .builder()
                            .title(f.title)
                            .body(f.body)
                            .show();
                    }
                }
                STALE_EMITTED.store(false, Ordering::Relaxed);
            }
        }
        FetchOutcome::Unauthorized => {
            let _ = handle.emit(
                "auth_required_for_slot",
                json!({ "slot": slot, "email": acc.email }),
            );
            let mut entry = state
                .cached_usage_by_slot
                .write()
                .remove(&slot)
                .unwrap_or_else(|| placeholder_cached(acc, "auth_required"));
            entry.last_error = Some("auth_required".into());
            state.cached_usage_by_slot.write().insert(slot, entry.clone());
            let _ = handle.emit(
                "usage_updated",
                json!({ "slot": slot, "cached": entry }),
            );
        }
        FetchOutcome::RateLimited(retry_after) => {
            match backoff_for_429(retry_after) {
                Some(delay) => {
                    tracing::warn!(
                        "slot {slot} rate-limited; backing off {:?} (server retry-after={:?})",
                        delay,
                        retry_after,
                    );
                    state
                        .backoff_by_slot
                        .write()
                        .insert(slot, BackoffState { until: Instant::now() + delay });
                }
                None => {
                    // Retry-After: 0 or absent — retry at the next scheduled
                    // poll; the error state clears on the next success.
                    tracing::warn!(
                        "slot {slot} rate-limited; retrying at next scheduled poll (server retry-after={retry_after:?})",
                    );
                }
            }
            let mut entry = state
                .cached_usage_by_slot
                .write()
                .remove(&slot)
                .unwrap_or_else(|| placeholder_cached(acc, "rate-limited (429)"));
            entry.last_error = Some("rate-limited (429)".into());
            state.cached_usage_by_slot.write().insert(slot, entry.clone());
            let _ = handle.emit(
                "usage_updated",
                json!({ "slot": slot, "cached": entry }),
            );
        }
        FetchOutcome::Transient(e) => {
            let mut entry = state
                .cached_usage_by_slot
                .write()
                .remove(&slot)
                .unwrap_or_else(|| placeholder_cached(acc, &e));
            entry.last_error = Some(e);
            state.cached_usage_by_slot.write().insert(slot, entry.clone());
            let _ = handle.emit(
                "usage_updated",
                json!({ "slot": slot, "cached": entry }),
            );
        }
    }
}

async fn poll_all(
    handle: &AppHandle,
    state: &AppState,
    burn_buffers: &mut HashMap<u32, SlotBurnBuffers>,
) -> Result<(), anyhow::Error> {
    // 1. Reconcile active slot from live CC creds.
    let live = state.auth.read_live_claude_code().await.ok().flatten();
    let accounts = state.accounts.list().unwrap_or_default();
    let active_slot = live.as_ref().and_then(|l| {
        accounts
            .iter()
            .find(|a| a.account_uuid == l.account_uuid)
            .map(|a| a.slot)
    });
    let prev_active_slot = std::mem::replace(&mut *state.active_slot.write(), active_slot);

    // Catches the paths that never go through `swap_to_account`: an external
    // `cswap`, or a Claude Code / cowork instance started on another machine.
    // Any shared snapshot older than this moment describes the account that
    // was live before the change.
    if prev_active_slot != active_slot {
        *state.active_since.write() = Some(Utc::now());

        let new_account_uuid = active_slot.and_then(|slot| {
            accounts.iter().find(|a| a.slot == slot).map(|a| a.account_uuid.as_str())
        });
        if let Err(e) = state.db.record_account_transition(new_account_uuid, Utc::now()) {
            tracing::warn!("failed to record account interval: {e:#}");
        }
    }

    // Notify the frontend whenever the active slot transitions. The
    // frontend's `accounts` array only carries `is_active` flags from the
    // last `list_accounts` call; without this event, an out-of-band CC
    // login (or the startup race where `init()` reads `list_accounts`
    // before this loop's first tick) leaves the AccountsPanel showing no
    // active highlight even though the backend knows which slot is live.
    if prev_active_slot != active_slot {
        let entries: Vec<crate::commands::AccountListEntry> = accounts
            .iter()
            .map(|a| crate::commands::entry_for(state, a, active_slot))
            .collect();
        let _ = handle.emit("accounts_changed", &entries);
    }

    // 2. Empty-state + unmanaged-active signals.
    if accounts.is_empty() && live.is_none() {
        let _ = handle.emit("requires_setup", ());
    }
    if let Some(live) = &live {
        if active_slot.is_none() {
            let _ = handle.emit(
                "unmanaged_active_account",
                json!({
                    "email": live.email,
                    "account_uuid": live.account_uuid,
                }),
            );
        }
    }

    // 3. Lazy-seed the schedule on first call (or when slots have been
    //    added/removed without going through swap_to_account, which seeds
    //    explicitly). If schedule_by_slot is empty but we have managed
    //    accounts, seed; if accounts have been removed since last tick,
    //    drop their entries; if accounts have been added, append at the
    //    tail of the round one stagger gap behind the latest deadline.
    {
        let mut sched = state.schedule_by_slot.write();
        let existing_slots: std::collections::HashSet<u32> =
            sched.keys().copied().collect();
        let current_slots: std::collections::HashSet<u32> =
            accounts.iter().map(|a| a.slot).collect();

        // Drop schedule entries for slots that no longer exist.
        sched.retain(|slot, _| current_slots.contains(slot));

        if sched.is_empty() && !accounts.is_empty() {
            let (interval, base_gap) = settings_durations(state);
            let slot_ids: Vec<u32> = accounts.iter().map(|a| a.slot).collect();
            *sched = seed_schedules(
                &slot_ids,
                active_slot,
                Instant::now(),
                interval,
                base_gap,
            );
        } else {
            // Append newly added slots so they trail the existing
            // schedule (one stagger gap behind the latest deadline).
            // This avoids placing a new slot ahead of an existing slot's
            // next fetch, which would break the configured base gap.
            let now = Instant::now();
            let (interval, base_gap) = settings_durations(state);
            let gap = stagger_gap(current_slots.len(), interval, base_gap);
            let latest = sched
                .values()
                .map(|s| s.next_poll_at)
                .max()
                .unwrap_or(now);
            let mut next_at = latest + gap;
            for slot in current_slots.difference(&existing_slots) {
                sched.insert(
                    *slot,
                    crate::app_state::ScheduleState { next_poll_at: next_at },
                );
                next_at += gap;
            }
        }
    }

    // 4. Pick at most one due slot, fetch it, advance its deadline.
    let now = Instant::now();
    if let Some(slot) = pick_due_slot(state, now) {
        fetch_and_apply_one(handle, state, burn_buffers, slot, &accounts).await;
        let interval = Duration::from_secs(
            state.settings.read().polling_interval_secs.max(60),
        );
        if let Some(entry) = state.schedule_by_slot.write().get_mut(&slot) {
            entry.next_poll_at = Instant::now() + interval;
        }
    }

    Ok(())
}

fn clamp_backoff(d: Duration) -> Duration {
    let min = Duration::from_secs(60);
    let max = Duration::from_secs(10 * 60);
    d.clamp(min, max)
}

/// Decide the backoff for a 429 response, or `None` for "no extra backoff —
/// retry at the next normally scheduled poll".
///
/// The Anthropic usage endpoint returns `Retry-After: 0` on most 429s: the
/// server is asking for no delay beyond the caller's own cadence. The
/// per-slot poll schedule already spaces retries by `polling_interval_secs`
/// (advanced unconditionally after each fetch attempt), so honoring the zero
/// keeps the request rate identical to the success path — no hammering.
/// The previous implementation treated 0 as "no guidance" and escalated
/// 2→4→8→10 min, which stretched each transient 429 into a multi-minute
/// "usage unavailable" window for no benefit.
///
/// Explicit positive `Retry-After` values are honored, clamped into
/// [60s, 10min] so a misconfigured server can't lock us out indefinitely
/// or sub-minute-poll us.
fn backoff_for_429(retry_after: Option<Duration>) -> Option<Duration> {
    match retry_after {
        Some(d) if d > Duration::ZERO => Some(clamp_backoff(d)),
        _ => None,
    }
}

fn placeholder_cached(
    acc: &crate::auth::accounts::ManagedAccount,
    err: &str,
) -> CachedUsage {
    CachedUsage {
        snapshot: UsageSnapshot {
            five_hour: None,
            seven_day: None,
            seven_day_sonnet: None,
            seven_day_opus: None,
            extra_usage: None,
            fetched_at: Utc::now(),
            unknown: Default::default(),
        },
        account_id: acc.account_uuid.clone(),
        account_email: acc.email.clone(),
        last_error: Some(err.to_string()),
        burn_rate: None,
        seven_day_burn_rate: None,
        extra_burn_rate: None,
        auth_source: AuthSource::OAuth,
    }
}

/// Location of the shared usage snapshot file. Two writers share this path:
/// an external poller — the user's statusline daemon (`statusline-daemon.sh`),
/// which already polls `/api/oauth/usage` for the live Claude Code account on
/// a 60s cadence — and Switchboard itself (`write_shared_snapshot`), for the
/// active account whenever its usage is refreshed. `SWITCHBOARD_SHARED_USAGE_FILE`
/// overrides for testing.
pub(crate) fn shared_usage_file_path() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("SWITCHBOARD_SHARED_USAGE_FILE") {
        return std::path::PathBuf::from(p);
    }
    crate::auth::paths::claude_config_home()
        .unwrap_or_else(|| std::path::PathBuf::from("/"))
        .join("statusline-usage.json")
}

/// Adopt a shared usage snapshot written by an external poller when it's
/// fresher than `max_age`. Returns `None` when the file is missing, stale,
/// unparsable, or lacks the injected `fetched_at` epoch-seconds marker that
/// identifies the daemon's format (a bare /usage payload has no timestamp,
/// so a file without one can't be freshness-checked and is ignored).
///
/// Why this exists: the /usage endpoint's rate budget is per-account and is
/// shared by every consumer — Claude Code sessions, the statusline daemon,
/// and this app. On a busy account the budget is saturated, so the app's own
/// fetches 429 constantly. Adopting the daemon's fresh snapshot removes this
/// app as a competitor for the active account's budget entirely.
pub fn read_shared_snapshot(
    path: &std::path::Path,
    max_age: Duration,
    active_since: Option<DateTime<Utc>>,
) -> Option<UsageSnapshot> {
    let raw = std::fs::read_to_string(path).ok()?;
    let mut value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let obj = value.as_object_mut()?;
    let fetched_at = obj.remove("fetched_at")?.as_i64()?;
    let fetched_at = DateTime::from_timestamp(fetched_at, 0)?;
    let now = Utc::now();
    // Reject clock-skewed future timestamps (30s grace) and stale data.
    if fetched_at > now + ChronoDuration::seconds(30) {
        return None;
    }
    if now - fetched_at > ChronoDuration::from_std(max_age).ok()? {
        return None;
    }
    // The file carries NO account identity — it describes whichever account
    // was live when the daemon wrote it. A snapshot written before the active
    // account last changed therefore describes the *previous* account, and
    // adopting it would put that account's numbers on the new active slot
    // while the previous account's own slot fetches the same numbers with its
    // own token — two rows showing identical usage until the file ages out.
    //
    // `active_since` is when the live account last changed (swap, external
    // `cswap`, or a Claude Code / cowork instance started elsewhere). `None`
    // means no change has been observed this run, so there is nothing to
    // invalidate against.
    if let Some(since) = active_since {
        if fetched_at < since {
            return None;
        }
    }
    let mut snap: UsageSnapshot = serde_json::from_value(value).ok()?;
    snap.fetched_at = fetched_at;
    Some(snap)
}

/// Write the active account's snapshot to the shared-usage file, in the
/// exact format `read_shared_snapshot` parses. `fetched_at` must be a bare
/// epoch-seconds integer at the top level — `UsageSnapshot`'s own
/// `fetched_at` field serializes as an RFC3339 string, which the reader's
/// `.as_i64()` call would reject, so it's stripped and replaced rather than
/// serialized as-is.
///
/// The replacement value is `snapshot.fetched_at`, NOT `Utc::now()`: this
/// function is also called when the active slot's "fetch" was actually a
/// `read_shared_snapshot` adoption of this very file (the active-slot fast
/// path in `fetch_and_apply_one`), in which case `snapshot.fetched_at` is
/// the ORIGINAL write time, carried forward by the reader. Stamping
/// `Utc::now()` here instead would re-mark that unchanged data as fresh on
/// every adoption, defeating the reader's own freshness/staleness check and
/// making a self-referential read-adopt-rewrite loop that never ages out —
/// which silently starves `force_refresh` of new data when it runs within
/// one polling interval of the last poll.
pub(crate) fn write_shared_snapshot(
    path: &std::path::Path,
    snapshot: &UsageSnapshot,
) -> anyhow::Result<()> {
    let mut value = serde_json::to_value(snapshot)?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "fetched_at".to_string(),
            serde_json::json!(snapshot.fetched_at.timestamp()),
        );
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_string(&value)?)?;
    Ok(())
}

/// Build per-slot cache entries from the most recent persisted API snapshot
/// for each account. Called once at startup so the UI has last-known-good
/// data before the first poll completes — without this, a cold start during
/// a rate-limit storm shows "usage unavailable" (empty placeholder) until a
/// fetch finally succeeds, which can take minutes.
///
/// Accounts without a persisted snapshot, or whose latest payload fails to
/// decode, are skipped (the poll loop will fill them in on its first tick).
pub fn hydrated_caches(
    db: &crate::store::Db,
    accounts: &[ManagedAccount],
) -> HashMap<u32, CachedUsage> {
    let mut out = HashMap::new();
    for acc in accounts {
        let payload = match db.latest_snapshot(&acc.account_uuid) {
            Ok(Some((_fetched_at, payload))) => payload,
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!("hydrate: latest_snapshot({}) failed: {e}", acc.account_uuid);
                continue;
            }
        };
        match serde_json::from_str::<UsageSnapshot>(&payload) {
            Ok(snapshot) => {
                out.insert(
                    acc.slot,
                    CachedUsage {
                        snapshot,
                        account_id: acc.account_uuid.clone(),
                        account_email: acc.email.clone(),
                        last_error: None,
                        burn_rate: None,
                        seven_day_burn_rate: None,
                        extra_burn_rate: None,
                        auth_source: AuthSource::OAuth,
                    },
                );
            }
            Err(e) => {
                tracing::warn!(
                    "hydrate: decode snapshot for {} failed: {e}",
                    acc.account_uuid
                );
            }
        }
    }
    out
}

/// Linear utilization projection, shared by the 5H and 7D buckets — only the
/// window length and minimum sample span differ between them. A longer
/// window needs a proportionally higher span floor: a 2-minute floor tuned
/// for 5H's 300-minute horizon would let 7D's ~33x-longer horizon
/// extrapolate wildly from a couple of samples taken moments apart (the same
/// amplification-per-sample-gap reasoning `update_extra_burn_rate` already
/// applies for PAYG's 30-day horizon).
fn update_burn_rate(
    buf: &mut VecDeque<(DateTime<Utc>, f64)>,
    bucket: Option<&Utilization>,
    window: ChronoDuration,
    min_span_minutes: f64,
    now: DateTime<Utc>,
) -> Option<BurnRateProjection> {
    let bucket = bucket?;
    let resets_at = bucket.resets_at?;
    let window_start = resets_at - window;
    while let Some(&(ts, _)) = buf.front() {
        if ts < window_start {
            buf.pop_front();
        } else {
            break;
        }
    }
    buf.push_back((now, bucket.utilization));
    if buf.len() < 2 {
        return None;
    }
    let &(t0, u0) = buf.front()?;
    let &(t1, u1) = buf.back()?;
    let span_minutes = (t1 - t0).num_seconds() as f64 / 60.0;
    if span_minutes < min_span_minutes {
        return None;
    }
    let slope = (u1 - u0) / span_minutes;
    let mins_until_reset = ((resets_at - now).num_seconds() as f64 / 60.0).max(0.0);
    Some(BurnRateProjection {
        utilization_per_min: slope,
        projected_at_reset: u1 + slope * mins_until_reset,
    })
}

fn update_extra_burn_rate(
    buf: &mut VecDeque<(DateTime<Utc>, f64)>,
    snapshot: &UsageSnapshot,
    now: DateTime<Utc>,
) -> Option<ExtraBurnRate> {
    let extra = snapshot.extra_usage.as_ref()?;
    if !extra.is_enabled {
        return None;
    }
    // PAYG windows run ~30 days; a rolling day of samples gives a stable
    // slope without unbounded growth.
    let cutoff = now - ChronoDuration::hours(24);
    while let Some(&(ts, _)) = buf.front() {
        if ts < cutoff {
            buf.pop_front();
        } else {
            break;
        }
    }
    let used = extra.used_credits_cents as f64;
    if buf.back().is_some_and(|&(_, prev)| used < prev) {
        // used_credits_cents dropped — a reset or credit top-up happened.
        // Old samples straddle the discontinuity and would produce a wild
        // slope; start the projection fresh from this point.
        buf.clear();
    }
    buf.push_back((now, used));
    if buf.len() < 2 {
        return None;
    }
    let &(t0, u0) = buf.front()?;
    let &(t1, u1) = buf.back()?;
    let span_minutes = (t1 - t0).num_seconds() as f64 / 60.0;
    // PAYG projects across up to a 30-day horizon (43,200 min) vs. the 5h
    // sibling's 300 min — ~144x more amplification per minute of sample
    // gap — so this floor is raised to 30 minutes to blunt cold-start
    // extrapolation from a single short polling gap.
    if span_minutes < 30.0 {
        return None;
    }
    let cents_per_min = (u1 - u0) / span_minutes;
    let projected_cents_at_reset = extra.resets_at.map(|reset| {
        let mins_until_reset = ((reset - now).num_seconds() as f64 / 60.0).max(0.0);
        u1 + cents_per_min * mins_until_reset
    });
    Some(ExtraBurnRate { cents_per_min, projected_cents_at_reset })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn d(secs: u64) -> Duration {
        Duration::from_secs(secs)
    }

    #[test]
    fn stagger_gap_returns_base_when_interval_fits() {
        // 3 slots, 300 s interval, 30 s base → needs 60 s for stagger, fits.
        let gap = stagger_gap(3, d(300), d(30));
        assert_eq!(gap, d(30));
    }

    #[test]
    fn stagger_gap_compresses_when_interval_too_short() {
        // 4 slots, 60 s interval, 30 s base → 90 s needed > 60 s → 60/4 = 15 s.
        let gap = stagger_gap(4, d(60), d(30));
        assert_eq!(gap, d(15));
    }

    #[test]
    fn stagger_gap_honors_custom_base() {
        // 3 slots, 300 s interval, 60 s base → 2*60 = 120 ≤ 300, gap stays 60.
        let gap = stagger_gap(3, d(300), d(60));
        assert_eq!(gap, d(60));
    }

    #[test]
    fn stagger_gap_zero_slots_returns_base() {
        assert_eq!(stagger_gap(0, d(60), d(30)), d(30));
        assert_eq!(stagger_gap(0, d(60), d(15)), d(15));
    }

    #[test]
    fn seed_schedules_active_first_then_inactive_in_slot_id_order() {
        let now = Instant::now();
        let sched = seed_schedules(&[3, 1, 2], Some(2), now, d(300), d(30));

        assert_eq!(sched[&2].next_poll_at, now);
        assert_eq!(sched[&1].next_poll_at, now + d(30));
        assert_eq!(sched[&3].next_poll_at, now + d(60));
    }

    #[test]
    fn seed_schedules_no_active_slot_orders_by_slot_id() {
        let now = Instant::now();
        let sched = seed_schedules(&[3, 1, 2], None, now, d(300), d(30));

        assert_eq!(sched[&1].next_poll_at, now);
        assert_eq!(sched[&2].next_poll_at, now + d(30));
        assert_eq!(sched[&3].next_poll_at, now + d(60));
    }

    #[test]
    fn seed_schedules_active_not_in_slots_ignored() {
        let now = Instant::now();
        let sched = seed_schedules(&[1, 2], Some(99), now, d(300), d(30));

        assert_eq!(sched[&1].next_poll_at, now);
        assert_eq!(sched[&2].next_poll_at, now + d(30));
        assert!(!sched.contains_key(&99));
    }

    #[test]
    fn seed_schedules_empty_slots_returns_empty_map() {
        let now = Instant::now();
        let sched = seed_schedules(&[], Some(1), now, d(300), d(30));
        assert!(sched.is_empty());
    }

    #[test]
    fn seed_schedules_custom_base_gap_propagates_to_offsets() {
        // 3 slots, 300 s interval, 60 s base → offsets at 0, 60, 120.
        let now = Instant::now();
        let sched = seed_schedules(&[1, 2, 3], None, now, d(300), d(60));
        assert_eq!(sched[&1].next_poll_at, now);
        assert_eq!(sched[&2].next_poll_at, now + d(60));
        assert_eq!(sched[&3].next_poll_at, now + d(120));
    }

    #[test]
    fn backoff_for_429_zero_retry_after_means_no_backoff() {
        // The usage endpoint returns `Retry-After: 0` on most 429s — the
        // server is asking for no delay beyond our normal poll cadence (the
        // per-slot schedule already spaces retries by the polling interval),
        // so no backoff entry should be created.
        assert_eq!(backoff_for_429(Some(Duration::ZERO)), None);
    }

    #[test]
    fn backoff_for_429_missing_retry_after_means_no_backoff() {
        assert_eq!(backoff_for_429(None), None);
    }

    #[test]
    fn backoff_for_429_positive_retry_after_is_honored_and_clamped() {
        // Explicit server guidance wins, clamped into [60s, 10min].
        assert_eq!(backoff_for_429(Some(d(120))), Some(d(120)));
        assert_eq!(backoff_for_429(Some(d(5))), Some(d(60)));
        assert_eq!(backoff_for_429(Some(d(3600))), Some(d(600)));
    }

    // `now` is threaded in (rather than the fixture calling `Utc::now()`
    // internally) so `resets_at` stays anchored to the synthetic timeline a
    // test is driving, matching how a real snapshot's reset date holds
    // steady across polls within one billing period. Sampling `Utc::now()`
    // here instead would desync `resets_at` from the `now` passed
    // separately to `update_extra_burn_rate`, skewing the projection by
    // however far apart the two clocks happen to be.
    fn snap_with_extra(used_cents: u64, now: DateTime<Utc>, reset_in_hours: Option<i64>) -> UsageSnapshot {
        UsageSnapshot {
            five_hour: None,
            seven_day: None,
            seven_day_sonnet: None,
            seven_day_opus: None,
            extra_usage: Some(crate::usage_api::ExtraUsage {
                is_enabled: true,
                monthly_limit_cents: 10_000,
                used_credits_cents: used_cents,
                utilization: Some(used_cents as f64 / 100.0),
                resets_at: reset_in_hours.map(|h| now + chrono::Duration::hours(h)),
            }),
            fetched_at: now,
            unknown: Default::default(),
        }
    }

    fn util(utilization: f64, resets_at: Option<DateTime<Utc>>) -> Utilization {
        Utilization { utilization, resets_at }
    }

    #[test]
    fn update_burn_rate_returns_none_without_a_bucket() {
        let mut buf = VecDeque::new();
        let now = Utc::now();
        assert!(update_burn_rate(&mut buf, None, ChronoDuration::hours(5), 2.0, now).is_none());
    }

    #[test]
    fn update_burn_rate_returns_none_without_a_reset_date() {
        let mut buf = VecDeque::new();
        let now = Utc::now();
        let bucket = util(20.0, None);
        assert!(update_burn_rate(&mut buf, Some(&bucket), ChronoDuration::hours(5), 2.0, now).is_none());
    }

    #[test]
    fn update_burn_rate_projects_at_5h_scale_params() {
        // Mirrors the pre-refactor 5H behavior: 2-minute floor, 5-hour window.
        let mut buf = VecDeque::new();
        let t0 = Utc::now();
        let resets_at = t0 + chrono::Duration::hours(4);
        update_burn_rate(&mut buf, Some(&util(20.0, Some(resets_at))), ChronoDuration::hours(5), 2.0, t0);

        let t1 = t0 + chrono::Duration::minutes(10);
        let r = update_burn_rate(&mut buf, Some(&util(25.0, Some(resets_at))), ChronoDuration::hours(5), 2.0, t1)
            .expect("two samples 10min apart, above the 2min floor, project");
        // slope = 5 util-points / 10 min = 0.5/min
        assert!((r.utilization_per_min - 0.5).abs() < 0.01);
    }

    #[test]
    fn update_burn_rate_seven_day_scale_needs_sixty_minutes_of_span() {
        // 7D's ~33x-longer horizon than 5H needs a proportionally higher
        // floor — 10 minutes of span is well under the 60-minute 7D floor.
        let mut buf = VecDeque::new();
        let t0 = Utc::now();
        let resets_at = t0 + chrono::Duration::days(3);
        update_burn_rate(&mut buf, Some(&util(20.0, Some(resets_at))), ChronoDuration::days(7), 60.0, t0);

        let t1 = t0 + chrono::Duration::minutes(10);
        assert!(
            update_burn_rate(&mut buf, Some(&util(25.0, Some(resets_at))), ChronoDuration::days(7), 60.0, t1)
                .is_none()
        );
    }

    #[test]
    fn update_burn_rate_seven_day_scale_projects_past_the_sixty_minute_floor() {
        let mut buf = VecDeque::new();
        let t0 = Utc::now();
        let resets_at = t0 + chrono::Duration::days(3);
        update_burn_rate(&mut buf, Some(&util(20.0, Some(resets_at))), ChronoDuration::days(7), 60.0, t0);

        let t1 = t0 + chrono::Duration::minutes(120);
        let r = update_burn_rate(&mut buf, Some(&util(32.0, Some(resets_at))), ChronoDuration::days(7), 60.0, t1)
            .expect("two samples 120min apart, above the 60min floor, project");
        // slope = 12 util-points / 120 min = 0.1/min
        assert!((r.utilization_per_min - 0.1).abs() < 0.01);
    }

    #[test]
    fn update_burn_rate_prunes_samples_older_than_the_window() {
        let mut buf = VecDeque::new();
        let t0 = Utc::now();
        // First reset window.
        let resets_at_1 = t0 + chrono::Duration::hours(1);
        update_burn_rate(&mut buf, Some(&util(90.0, Some(resets_at_1))), ChronoDuration::hours(5), 2.0, t0);
        assert_eq!(buf.len(), 1);

        // A new window rolls over (resets_at moves forward by 5h) — the old
        // sample, now older than `new_resets_at - 5h`, must be pruned so a
        // fresh pair is required before projecting again.
        let t1 = t0 + chrono::Duration::minutes(70);
        let resets_at_2 = resets_at_1 + chrono::Duration::hours(5);
        assert!(
            update_burn_rate(&mut buf, Some(&util(10.0, Some(resets_at_2))), ChronoDuration::hours(5), 2.0, t1)
                .is_none(),
            "stale pre-rollover sample should have been pruned, leaving only this one"
        );
        assert_eq!(buf.len(), 1);
    }

    #[test]
    fn extra_burn_rate_projects_dollars_at_reset() {
        let mut buf = VecDeque::new();
        let t0 = Utc::now();
        assert!(
            update_extra_burn_rate(&mut buf, &snap_with_extra(1000, t0, Some(24)), t0).is_none()
        );
        // 40 minutes later (above the 30min floor), 400 more cents spent → 10 cents/min.
        let t1 = t0 + chrono::Duration::minutes(40);
        let r = update_extra_burn_rate(&mut buf, &snap_with_extra(1400, t1, Some(24)), t1)
            .expect("two samples 40min apart project");
        assert!((r.cents_per_min - 10.0).abs() < 0.01);
        let projected = r.projected_cents_at_reset.expect("reset known");
        // 1400 + 10c/min * 24h(1440min) = 15800
        assert!((projected - 15_800.0).abs() < 60.0, "got {projected}");
    }

    #[test]
    fn extra_burn_rate_none_without_reset_date_still_reports_slope() {
        let mut buf = VecDeque::new();
        let t0 = Utc::now();
        update_extra_burn_rate(&mut buf, &snap_with_extra(1000, t0, None), t0);
        // 40 minutes later (above the 30min floor), 400 more cents spent → 10 cents/min.
        let t1 = t0 + chrono::Duration::minutes(40);
        let r = update_extra_burn_rate(&mut buf, &snap_with_extra(1400, t1, None), t1)
            .expect("slope computable without reset");
        assert!(r.projected_cents_at_reset.is_none());
        assert!((r.cents_per_min - 10.0).abs() < 0.01);
    }

    #[test]
    fn extra_burn_rate_needs_thirty_minutes_of_span() {
        // PAYG projects across up to a 30-day horizon rather than the 5h
        // sibling's 5h horizon (~144x more amplification per minute of
        // sample gap), so its floor is raised to 30 minutes. 10 minutes of
        // span is well under that and must not yet produce a projection.
        let mut buf = VecDeque::new();
        let t0 = Utc::now();
        update_extra_burn_rate(&mut buf, &snap_with_extra(1000, t0, Some(24)), t0);
        let t1 = t0 + chrono::Duration::minutes(10);
        assert!(
            update_extra_burn_rate(&mut buf, &snap_with_extra(1010, t1, Some(24)), t1).is_none()
        );
    }

    #[test]
    fn extra_burn_rate_clears_buffer_on_credit_drop() {
        // A reset or mid-cycle top-up makes used_credits_cents drop. Old
        // samples from before the drop must not survive into the new
        // window — otherwise the slope goes sharply negative and gets
        // extrapolated across the ~30-day horizon. Assert the buffer
        // restarts clean: the sample right after the drop doesn't pair
        // with anything, so it takes two MORE samples before a projection
        // reappears.
        let mut buf = VecDeque::new();
        let t0 = Utc::now();
        let t1 = t0 + chrono::Duration::minutes(40);
        update_extra_burn_rate(&mut buf, &snap_with_extra(1000, t0, Some(24)), t0);
        let r1 = update_extra_burn_rate(&mut buf, &snap_with_extra(1400, t1, Some(24)), t1)
            .expect("two samples 40min apart project");
        assert!(r1.cents_per_min > 0.0);

        // Credits used drops (reset/top-up) — this sample alone can't pair
        // with anything since the buffer was just cleared.
        let t2 = t1 + chrono::Duration::minutes(40);
        assert!(
            update_extra_burn_rate(&mut buf, &snap_with_extra(0, t2, Some(24)), t2).is_none(),
            "buffer should have cleared on the downward discontinuity"
        );

        // One more sample after the drop still isn't enough — it only
        // pairs with the drop sample, and that pair alone should NOT still
        // contain the old pre-drop points. Confirm a normal projection
        // resumes with fresh, non-negative slope.
        let t3 = t2 + chrono::Duration::minutes(40);
        let r3 = update_extra_burn_rate(&mut buf, &snap_with_extra(400, t3, Some(24)), t3)
            .expect("fresh pair after the drop projects again");
        assert!(
            r3.cents_per_min > 0.0,
            "slope should reflect only post-drop samples, got {}",
            r3.cents_per_min
        );
    }

    mod hydrate {
        use super::*;
        use crate::auth::accounts::{AddSource, ManagedAccount};
        use crate::store::Db;
        use tempfile::tempdir;

        fn acc(slot: u32, uuid: &str) -> ManagedAccount {
            ManagedAccount {
                slot,
                email: format!("slot{slot}@example.com"),
                account_uuid: uuid.to_string(),
                organization_uuid: None,
                organization_name: None,
                subscription_type: None,
                source: AddSource::OAuth,
                claude_code_oauth_blob: serde_json::json!({}),
                oauth_account_blob: serde_json::json!({}),
                token_expires_at: Utc::now(),
                added_at: Utc::now(),
                last_seen_active: None,
            }
        }

        const PAYLOAD: &str = r#"{
            "five_hour": { "utilization": 42.5, "resets_at": "2026-04-24T18:00:00Z" },
            "seven_day": { "utilization": 63.1, "resets_at": "2026-04-30T09:00:00Z" }
        }"#;

        fn db_with_account(uuid: &str) -> (tempfile::TempDir, Db) {
            let dir = tempdir().unwrap();
            let db = Db::open(dir.path()).unwrap();
            // api_snapshots.account_id is FK'd to accounts(id) — mirror the
            // account first, as mirror_account_to_sqlite does in production.
            db.upsert_account(&crate::store::StoredAccount {
                id: uuid.to_string(),
                email: "a@example.com".into(),
                display_name: None,
            })
            .unwrap();
            (dir, db)
        }

        #[test]
        fn returns_persisted_snapshot_for_account_with_last_error_cleared() {
            let (_dir, db) = db_with_account("uuid-2");
            db.insert_snapshot("uuid-2", Utc::now(), PAYLOAD).unwrap();

            let caches = hydrated_caches(&db, &[acc(2, "uuid-2")]);
            let entry = caches.get(&2).expect("slot 2 hydrated");
            assert_eq!(
                entry.snapshot.five_hour.as_ref().unwrap().utilization,
                42.5
            );
            assert_eq!(entry.last_error, None);
            assert_eq!(entry.account_email, "slot2@example.com");
        }

        #[test]
        fn skips_accounts_without_snapshots() {
            let dir = tempdir().unwrap();
            let db = Db::open(dir.path()).unwrap();
            let caches = hydrated_caches(&db, &[acc(2, "uuid-2")]);
            assert!(caches.is_empty());
        }

        #[test]
        fn skips_corrupt_payloads_instead_of_failing() {
            let (_dir, db) = db_with_account("uuid-2");
            db.insert_snapshot("uuid-2", Utc::now(), "not json").unwrap();
            let caches = hydrated_caches(&db, &[acc(2, "uuid-2")]);
            assert!(caches.is_empty());
        }
    }

    mod shared_snapshot {
        use super::*;
        use tempfile::tempdir;

        fn write(dir: &tempfile::TempDir, body: &str) -> std::path::PathBuf {
            let p = dir.path().join("statusline-usage.json");
            std::fs::write(&p, body).unwrap();
            p
        }

        fn payload(fetched_at: i64) -> String {
            format!(
                r#"{{"five_hour": {{"utilization": 42.5, "resets_at": "2026-04-24T18:00:00Z"}}, "seven_day": null, "fetched_at": {fetched_at}}}"#
            )
        }

        #[test]
        fn adopts_fresh_valid_snapshot() {
            let dir = tempdir().unwrap();
            let now = Utc::now().timestamp();
            let p = write(&dir, &payload(now));
            let snap = read_shared_snapshot(&p, Duration::from_secs(120), None)
                .expect("fresh snapshot adopted");
            assert_eq!(snap.five_hour.unwrap().utilization, 42.5);
            assert_eq!(snap.fetched_at.timestamp(), now);
        }

        #[test]
        fn tolerates_unknown_vendor_fields() {
            // The live file carries extra buckets and codename fields the
            // typed snapshot doesn't know — forward-compat must hold.
            let dir = tempdir().unwrap();
            let now = Utc::now().timestamp();
            let body = format!(
                r#"{{"five_hour": {{"utilization": 2.0, "resets_at": null}}, "seven_day": {{"utilization": 26.0, "resets_at": "2026-07-25T02:59:59Z"}}, "seven_day_cowork": null, "tangelo": null, "limits": [], "fetched_at": {now}}}"#
            );
            let p = write(&dir, &body);
            assert!(read_shared_snapshot(&p, Duration::from_secs(120), None).is_some());
        }

        #[test]
        fn rejects_stale_snapshot() {
            let dir = tempdir().unwrap();
            let old = (Utc::now() - ChronoDuration::minutes(10)).timestamp();
            let p = write(&dir, &payload(old));
            assert!(read_shared_snapshot(&p, Duration::from_secs(120), None).is_none());
        }

        #[test]
        fn rejects_future_timestamp() {
            let dir = tempdir().unwrap();
            let future = (Utc::now() + ChronoDuration::minutes(5)).timestamp();
            let p = write(&dir, &payload(future));
            assert!(read_shared_snapshot(&p, Duration::from_secs(120), None).is_none());
        }

        #[test]
        fn missing_file_returns_none() {
            let dir = tempdir().unwrap();
            let p = dir.path().join("does-not-exist.json");
            assert!(read_shared_snapshot(&p, Duration::from_secs(120), None).is_none());
        }

        #[test]
        fn corrupt_json_returns_none() {
            let dir = tempdir().unwrap();
            let p = write(&dir, "not json at all");
            assert!(read_shared_snapshot(&p, Duration::from_secs(120), None).is_none());
        }

        #[test]
        fn missing_fetched_at_returns_none() {
            // Without the daemon's injected epoch marker we can't judge
            // freshness — treat as a foreign file and ignore it.
            let dir = tempdir().unwrap();
            let p = write(&dir, r#"{"five_hour": {"utilization": 42.5, "resets_at": null}}"#);
            assert!(read_shared_snapshot(&p, Duration::from_secs(120), None).is_none());
        }

        /// The file carries NO account identity — no email, no account_uuid,
        /// nothing. It describes whichever account was live when the daemon
        /// wrote it. So a snapshot written BEFORE the active account changed
        /// describes the *previous* account, and adopting it puts the old
        /// account's numbers on the new active slot — while the old account's
        /// own slot fetches the same numbers with its own token. Both rows
        /// then show identical usage until the file ages out.
        #[test]
        fn rejects_a_snapshot_written_before_the_active_account_changed() {
            let dir = tempdir().unwrap();
            let now = Utc::now();
            // Daemon wrote this 60s ago, while account A was live.
            let written = (now - ChronoDuration::seconds(60)).timestamp();
            let p = write(&dir, &payload(written));

            // Still comfortably inside the freshness window on its own.
            assert!(
                read_shared_snapshot(&p, Duration::from_secs(300), None).is_some(),
                "precondition: the snapshot is fresh enough by age alone"
            );

            // The user swapped to account B 30s ago — after the file was
            // written. The snapshot cannot describe B.
            let swapped_at = now - ChronoDuration::seconds(30);
            assert!(
                read_shared_snapshot(&p, Duration::from_secs(300), Some(swapped_at)).is_none(),
                "a snapshot predating the swap describes the previous account"
            );
        }

        #[test]
        fn accepts_a_snapshot_written_after_the_active_account_changed() {
            let dir = tempdir().unwrap();
            let now = Utc::now();
            let swapped_at = now - ChronoDuration::seconds(60);
            // Daemon re-polled after the swap, so this describes the new account.
            let written = (now - ChronoDuration::seconds(10)).timestamp();
            let p = write(&dir, &payload(written));
            assert!(
                read_shared_snapshot(&p, Duration::from_secs(300), Some(swapped_at)).is_some(),
                "a snapshot written after the swap is valid for the new account"
            );
        }

        /// No recorded change (fresh boot, never swapped) must not disable the
        /// fast path — that would silently spend every account's scarce /usage
        /// budget on duplicate calls.
        #[test]
        fn no_recorded_change_still_adopts() {
            let dir = tempdir().unwrap();
            let p = write(&dir, &payload(Utc::now().timestamp()));
            assert!(read_shared_snapshot(&p, Duration::from_secs(120), None).is_some());
        }

        #[test]
        fn write_shared_snapshot_round_trips_through_read_shared_snapshot() {
            let dir = tempdir().unwrap();
            let p = dir.path().join("statusline-usage.json");
            let snap: UsageSnapshot = serde_json::from_str(
                r#"{"five_hour": {"utilization": 55.0, "resets_at": "2026-04-24T18:00:00Z"}, "seven_day": null}"#,
            )
            .unwrap();

            write_shared_snapshot(&p, &snap).unwrap();

            let read_back = read_shared_snapshot(&p, Duration::from_secs(120), None)
                .expect("just-written snapshot must be readable back");
            assert_eq!(read_back.five_hour.unwrap().utilization, 55.0);
        }

        #[test]
        fn write_shared_snapshot_stamps_fetched_at_as_an_epoch_integer() {
            let dir = tempdir().unwrap();
            let p = dir.path().join("statusline-usage.json");
            let snap: UsageSnapshot = serde_json::from_str(r#"{"five_hour": null, "seven_day": null}"#).unwrap();

            write_shared_snapshot(&p, &snap).unwrap();

            let raw = std::fs::read_to_string(&p).unwrap();
            let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
            assert!(
                v["fetched_at"].is_i64() || v["fetched_at"].is_u64(),
                "fetched_at must be a bare epoch-seconds integer, got: {:?}",
                v["fetched_at"]
            );
        }

        /// The active-slot fast path in `fetch_and_apply_one` can adopt this
        /// very file via `read_shared_snapshot` instead of doing a real HTTP
        /// fetch — in which case the resulting `snapshot.fetched_at` is the
        /// file's ORIGINAL write time, carried forward by the reader, not
        /// "now". If the write stamped `Utc::now()` instead of preserving
        /// that timestamp, every adoption would re-mark the same unchanged
        /// data as brand-new, so the file would never age out — and
        /// `force_refresh` (which bypasses normal poll spacing) would
        /// silently keep re-serving stale data forever. The write must
        /// preserve `snapshot.fetched_at` unchanged.
        #[test]
        fn write_shared_snapshot_preserves_an_already_stale_fetched_at() {
            let dir = tempdir().unwrap();
            let p = dir.path().join("statusline-usage.json");
            let mut snap: UsageSnapshot =
                serde_json::from_str(r#"{"five_hour": null, "seven_day": null}"#).unwrap();
            let old = Utc::now() - ChronoDuration::minutes(2);
            snap.fetched_at = old;

            write_shared_snapshot(&p, &snap).unwrap();

            let raw = std::fs::read_to_string(&p).unwrap();
            let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
            assert_eq!(
                v["fetched_at"].as_i64().unwrap(),
                old.timestamp(),
                "write must not silently refresh fetched_at to Utc::now()"
            );
        }
    }
}
