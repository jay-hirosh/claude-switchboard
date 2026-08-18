//! Hour-of-day usage pattern and warm-up scheduling recommendations.
//!
//! Bucketing mirrors `commands::bucket_daily_trends` (same local-timezone
//! keying), just split further into hour-of-day / day-of-week cells. The
//! warm-up recommendation simulates the actual `Every5h` scheduler behavior
//! (see `scheduler::catchup`) rather than assuming five 5-hour windows tile
//! 24 hours cleanly — they don't (25h of coverage doesn't fit in a day), so
//! one scheduled fire is suppressed by the previous window every cycle.

use crate::scheduler::presets::HhMm;
use crate::store::StoredSessionEvent;
use chrono::{Datelike, Timelike};
use serde::{Deserialize, Serialize};

/// Below this many distinct active days, a recommendation would be anchored
/// on noise — same rationale as `commands::compute_warmup_suggestion`'s
/// floor, just lower since the lookback here can be as short as 7 days.
const MIN_ACTIVE_DAYS_FOR_PLAN: u32 = 5;

/// Steady-state simulation length: three 24h laps. The first lap absorbs
/// whatever startup transient comes from beginning in a "no window active"
/// state; windows top out at 5h, so two laps already reach steady state —
/// the third is a buffer we sample from (`STEADY_LAP_START..STEADY_LAP_END`).
const SIM_HOURS: usize = 24 * 3;
const STEADY_LAP_START: usize = 24;
const STEADY_LAP_END: usize = 48;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct HourCell {
    /// 0 = Monday .. 6 = Sunday, matching `HeatmapTab`'s Monday-first grid.
    pub weekday: u8,
    pub hour: u8,
    pub tokens: u64,
    pub cost_usd: f64,
    pub request_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct HourTotal {
    pub hour: u8,
    pub tokens: u64,
    pub cost_usd: f64,
    pub request_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct PlannedWindow {
    pub start: HhMm,
    pub end: HhMm,
    /// Share (0..1) of the day's total tokens this window carries.
    pub share: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct WarmupPlan {
    pub anchor: HhMm,
    pub recommended_peak_share: f64,
    /// Peak window share with no warm-up at all — windows started purely by
    /// the user's own first request in each gap. The honest baseline.
    pub baseline_peak_share: f64,
    pub windows: Vec<PlannedWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct DailyPatternReport {
    /// Always exactly 168 entries (7 * 24), zeros included, ordered
    /// weekday-major then hour-minor — no gap-filling needed client-side.
    pub cells: Vec<HourCell>,
    /// Always exactly 24 entries.
    pub hourly_totals: Vec<HourTotal>,
    pub active_days: u32,
    pub lookback_days: u32,
    /// `None` below `MIN_ACTIVE_DAYS_FOR_PLAN` — too small a sample to
    /// anchor a recommendation on.
    pub plan: Option<WarmupPlan>,
}

/// Groups events into 168 (weekday, hour) cells and 24 hour-of-day totals,
/// all in local time — same `chrono::Local` keying as
/// `commands::bucket_daily_trends`, load-bearing for agreement with the
/// rest of the app (unlike `HeatmapTab`'s UTC-keyed heatmap).
fn bucket_hourly_pattern(events: &[StoredSessionEvent]) -> (Vec<HourCell>, Vec<HourTotal>, u32) {
    use std::collections::{BTreeSet, HashMap};

    let mut by_cell: HashMap<(u8, u8), (u64, f64, u64)> = HashMap::new();
    let mut active_days: BTreeSet<String> = BTreeSet::new();

    for e in events {
        let local = e.ts.with_timezone(&chrono::Local);
        let weekday = local.weekday().num_days_from_monday() as u8;
        let hour = local.hour() as u8;
        active_days.insert(local.format("%Y-%m-%d").to_string());

        let entry = by_cell.entry((weekday, hour)).or_insert((0, 0.0, 0));
        entry.0 += e.input_tokens + e.output_tokens;
        entry.1 += e.cost_usd;
        entry.2 += 1;
    }

    let mut hourly_totals: Vec<HourTotal> = (0..24u8)
        .map(|hour| HourTotal {
            hour,
            tokens: 0,
            cost_usd: 0.0,
            request_count: 0,
        })
        .collect();

    let mut cells = Vec::with_capacity(168);
    for weekday in 0u8..7 {
        for hour in 0u8..24 {
            let (tokens, cost_usd, request_count) =
                by_cell.get(&(weekday, hour)).copied().unwrap_or((0, 0.0, 0));
            cells.push(HourCell {
                weekday,
                hour,
                tokens,
                cost_usd,
                request_count,
            });
            let total = &mut hourly_totals[hour as usize];
            total.tokens += tokens;
            total.cost_usd += cost_usd;
            total.request_count += request_count;
        }
    }

    (cells, hourly_totals, active_days.len() as u32)
}

struct SimWindow {
    start_slot: usize,
    load: f64,
}

/// Simulates `SIM_HOURS` of hour-of-day weights against a set of daily fire
/// hours, replaying the real scheduler rule: a fire (or organic activity in
/// a weight>0 hour) starts a 5-hour window only if none is already active
/// (`warmup::is_five_hour_window_active`). Returns the windows whose start
/// falls in the steady-state third-lap slice.
fn simulate(weights: &[f64; 24], fires: &[u8]) -> Vec<SimWindow> {
    let mut windows: Vec<SimWindow> = Vec::new();
    let mut window_end: Option<usize> = None;

    for i in 0..SIM_HOURS {
        let hour = (i % 24) as u8;
        let w = weights[hour as usize];
        let active = window_end.is_some_and(|end| i < end);

        if !active {
            if fires.contains(&hour) || w > 0.0 {
                windows.push(SimWindow {
                    start_slot: i,
                    load: 0.0,
                });
                window_end = Some(i + 5);
            } else {
                window_end = None;
            }
        }

        if let Some(end) = window_end {
            if i < end {
                windows
                    .last_mut()
                    .expect("window_end implies a pushed window")
                    .load += w;
            }
        }
    }

    windows
        .into_iter()
        .filter(|w| w.start_slot >= STEADY_LAP_START && w.start_slot < STEADY_LAP_END)
        .collect()
}

fn peak_share(windows: &[SimWindow], total: f64) -> f64 {
    if total <= 0.0 {
        return 0.0;
    }
    windows.iter().map(|w| w.load).fold(0.0, f64::max) / total
}

/// True if `(share_a, count_a)` beats `(share_b, count_b)` — lower share
/// wins; a tie prefers fewer windows, and a full tie keeps whichever was
/// seen first (the caller iterates anchors ascending, so that's the
/// earliest anchor — deterministic without an extra tie-break field).
fn better(share_a: f64, count_a: usize, share_b: f64, count_b: usize) -> bool {
    match share_a.total_cmp(&share_b) {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Equal => count_a < count_b,
        std::cmp::Ordering::Greater => false,
    }
}

/// Recommends the hourly `Every5h` anchor that minimizes the busiest
/// window's share of the day, scored against `hourly_totals`' token
/// weights. `None` below `MIN_ACTIVE_DAYS_FOR_PLAN` active days or when
/// there's no usage to weight against.
fn compute_warmup_plan(hourly_totals: &[HourTotal], active_days: u32) -> Option<WarmupPlan> {
    if active_days < MIN_ACTIVE_DAYS_FOR_PLAN {
        return None;
    }

    let mut weights = [0.0f64; 24];
    for t in hourly_totals {
        weights[t.hour as usize] = t.tokens as f64;
    }
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return None;
    }

    let baseline_peak_share = peak_share(&simulate(&weights, &[]), total);

    let mut best: Option<(u8, f64, Vec<SimWindow>)> = None;
    for anchor in 0u8..24 {
        let fires: Vec<u8> = (0u8..5).map(|k| (anchor + 5 * k) % 24).collect();
        let windows = simulate(&weights, &fires);
        let share = peak_share(&windows, total);
        let count = windows.len();

        let replace = match &best {
            None => true,
            Some((_, best_share, best_windows)) => {
                better(share, count, *best_share, best_windows.len())
            }
        };
        if replace {
            best = Some((anchor, share, windows));
        }
    }

    let (anchor_hour, recommended_peak_share, windows) = best?;
    let planned_windows = windows
        .into_iter()
        .map(|w| PlannedWindow {
            start: HhMm::new((w.start_slot % 24) as u8, 0),
            end: HhMm::new(((w.start_slot + 5) % 24) as u8, 0),
            share: w.load / total,
        })
        .collect();

    Some(WarmupPlan {
        anchor: HhMm::new(anchor_hour, 0),
        recommended_peak_share,
        baseline_peak_share,
        windows: planned_windows,
    })
}

/// UTC instant for local midnight of the day containing `now`. Used by
/// `commands::get_today_pattern` to scope a query to "today" in the user's
/// timezone rather than a rolling 24h window.
///
/// A DST transition can make local midnight ambiguous (two instants share
/// that wall-clock time) or nonexistent (spring-forward skips over it) —
/// `.single()` returns `None` for both, and we fall back to `now` itself
/// rather than failing the query. That just narrows "today" to whatever's
/// left of it, which is harmless on the one day a year it matters.
pub fn local_midnight_utc(now: chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;

    let local_now = now.with_timezone(&chrono::Local);
    let naive_midnight = local_now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("00:00:00 is always a valid time");
    chrono::Local
        .from_local_datetime(&naive_midnight)
        .single()
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or(now)
}

/// Entry point for `commands::get_daily_pattern`.
pub fn compute_daily_pattern(
    events: &[StoredSessionEvent],
    lookback_days: u32,
) -> DailyPatternReport {
    let (cells, hourly_totals, active_days) = bucket_hourly_pattern(events);
    let plan = compute_warmup_plan(&hourly_totals, active_days);
    DailyPatternReport {
        cells,
        hourly_totals,
        active_days,
        lookback_days,
        plan,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Utc};

    fn event_at(
        local_hour: u32,
        local_minute: u32,
        day_offset: i64,
        tokens: u64,
    ) -> StoredSessionEvent {
        // Anchor on a fixed Monday so weekday math is hand-verifiable
        // regardless of the machine's local timezone offset from UTC —
        // creating via Local and round-tripping through UTC preserves the
        // same local wall-clock time either way.
        let base = chrono::Local
            .with_ymd_and_hms(2026, 8, 3, local_hour, local_minute, 0) // 2026-08-03 is a Monday
            .unwrap();
        let ts: DateTime<Utc> = (base + chrono::Duration::days(day_offset)).with_timezone(&Utc);
        StoredSessionEvent {
            ts,
            project: "test".into(),
            model: "claude-sonnet-4-5".into(),
            input_tokens: tokens,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 0.0,
            source_file: "test.jsonl".into(),
            source_line: 1,
            event_id: format!("evt-{local_hour}-{day_offset}"),
            account_uuid: None,
        }
    }

    #[test]
    fn buckets_events_by_local_weekday_and_hour() {
        let events = vec![
            event_at(9, 15, 0, 100), // Monday 09:xx
            event_at(9, 45, 0, 50),  // Monday 09:xx again — same cell
            event_at(14, 0, 1, 200), // Tuesday 14:xx
        ];
        let (cells, totals, active_days) = bucket_hourly_pattern(&events);

        assert_eq!(cells.len(), 168);
        assert_eq!(active_days, 2);

        let mon_9 = cells
            .iter()
            .find(|c| c.weekday == 0 && c.hour == 9)
            .unwrap();
        assert_eq!(mon_9.tokens, 150);
        assert_eq!(mon_9.request_count, 2);

        let tue_14 = cells
            .iter()
            .find(|c| c.weekday == 1 && c.hour == 14)
            .unwrap();
        assert_eq!(tue_14.tokens, 200);

        let total_9 = totals.iter().find(|t| t.hour == 9).unwrap();
        assert_eq!(total_9.tokens, 150);
    }

    #[test]
    fn simulate_with_no_fires_starts_a_window_on_first_activity() {
        let mut weights = [0.0f64; 24];
        for w in weights.iter_mut().take(17).skip(9) {
            *w = 10.0;
        }
        let windows = simulate(&weights, &[]);
        // Steady state: a window starts at 09:00 every day since that's the
        // first weighted hour after the prior day's last window ends.
        assert!(windows.iter().any(|w| w.start_slot % 24 == 9));
    }

    #[test]
    fn every5h_schedule_suppresses_one_fire_and_leaves_a_gap() {
        // Five 5h windows need 25h/day — one fire is always swallowed by the
        // spillover of the previous day's last window.
        let weights = [0.0f64; 24]; // no organic activity, fires are the only trigger
        let fires = [0u8, 5, 10, 15, 20];
        let windows = simulate(&weights, &fires);

        let starts: Vec<usize> = windows.iter().map(|w| w.start_slot % 24).collect();
        assert_eq!(starts, vec![5, 10, 15, 20]); // hour 0's fire is suppressed
        assert_eq!(windows.len(), 4);

        // Hours 1-4 are covered by no window at all.
        let covered: std::collections::HashSet<usize> = windows
            .iter()
            .flat_map(|w| w.start_slot..w.start_slot + 5)
            .map(|slot| slot % 24)
            .collect();
        for gap_hour in 1..5 {
            assert!(
                !covered.contains(&gap_hour),
                "hour {gap_hour} should be uncovered"
            );
        }
    }

    #[test]
    fn plan_is_none_below_the_active_day_floor() {
        let totals: Vec<HourTotal> = (0..24u8)
            .map(|hour| HourTotal {
                hour,
                tokens: if hour == 9 { 100 } else { 0 },
                cost_usd: 0.0,
                request_count: 1,
            })
            .collect();
        assert!(compute_warmup_plan(&totals, MIN_ACTIVE_DAYS_FOR_PLAN - 1).is_none());
        assert!(compute_warmup_plan(&totals, MIN_ACTIVE_DAYS_FOR_PLAN).is_some());
    }

    #[test]
    fn plan_never_recommends_a_worse_peak_than_the_baseline() {
        let mut totals: Vec<HourTotal> = (0..24u8)
            .map(|hour| HourTotal {
                hour,
                tokens: 0,
                cost_usd: 0.0,
                request_count: 0,
            })
            .collect();
        // A sharp midday burst — the case warm-up is meant to help with.
        for hour in 11..13 {
            totals[hour as usize].tokens = 5000;
        }
        let plan = compute_warmup_plan(&totals, MIN_ACTIVE_DAYS_FOR_PLAN).unwrap();
        assert!(plan.recommended_peak_share <= plan.baseline_peak_share + 1e-9);
        // A 2-hour burst split across two windows can't beat a 50/50 split.
        assert!(plan.recommended_peak_share <= 0.5 + 1e-9);
    }

    #[test]
    fn local_midnight_utc_is_start_of_todays_local_calendar_day() {
        let local_now = chrono::Local
            .with_ymd_and_hms(2026, 8, 18, 15, 30, 0)
            .single()
            .unwrap();
        let now_utc = local_now.with_timezone(&Utc);

        let midnight_utc = local_midnight_utc(now_utc);
        let midnight_local = midnight_utc.with_timezone(&chrono::Local);

        assert_eq!(midnight_local.date_naive(), local_now.date_naive());
        assert_eq!((midnight_local.hour(), midnight_local.minute()), (0, 0));
        assert!(midnight_utc <= now_utc);
    }

    #[test]
    fn flat_usage_ties_every_anchor_and_keeps_the_earliest() {
        let totals: Vec<HourTotal> = (0..24u8)
            .map(|hour| HourTotal {
                hour,
                tokens: 100,
                cost_usd: 0.0,
                request_count: 1,
            })
            .collect();
        let plan = compute_warmup_plan(&totals, MIN_ACTIVE_DAYS_FOR_PLAN).unwrap();
        // Constant weight makes every hour self-triggering, so the fire
        // schedule never actually matters — every anchor ties exactly, and
        // the earliest-seen (anchor 0) wins.
        assert_eq!(plan.anchor.hour, 0);
        assert_eq!(plan.anchor.minute, 0);
    }
}
