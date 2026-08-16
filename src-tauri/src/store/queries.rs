use super::Db;
use anyhow::Result;
use chrono::{DateTime, Timelike, Utc};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::app_state::Settings;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct StoredAccount {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct StoredSessionEvent {
    #[specta(type = String)]
    pub ts: DateTime<Utc>,
    pub project: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_5m_tokens: u64,
    pub cache_creation_1h_tokens: u64,
    pub cost_usd: f64,
    pub source_file: String,
    pub source_line: i64,
    /// Stable per-API-call key used for dedup. Format: "{requestId}:{message.id}"
    /// when both are present in the JSONL line, else "{source_file}:{source_line}"
    /// as a structural fallback for older / pre-requestId schemas.
    pub event_id: String,
    /// The managed account whose interval this event's `ts` falls inside,
    /// resolved by `events_between`'s subquery against `account_intervals`.
    /// `None` means either the event predates account_intervals tracking, or
    /// it landed in a gap where no managed account was live. Not a real
    /// column on `session_events` — only ever populated by query methods
    /// that explicitly join for it; constructing a `StoredSessionEvent` for
    /// insertion (e.g. in the JSONL walker) should always set this to `None`.
    pub account_uuid: Option<String>,
}

/// A `/compact` boundary inside one session transcript.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct StoredCompaction {
    #[specta(type = String)]
    pub ts: DateTime<Utc>,
    pub source_file: String,
    /// "manual" (`/compact`) or "auto" (context exhausted).
    pub trigger: String,
    /// Context size immediately before the compaction, in tokens.
    pub pre_tokens: u64,
    /// Context size that survived it.
    pub post_tokens: u64,
    pub uuid: String,
}

/// The parent transcript's most recent event — used by the live-session
/// registry to answer "what is this session doing right now." Subagent
/// transcripts are deliberately excluded: they have their own context
/// windows and aren't part of what the parent session's readout shows.
#[derive(Debug, Clone)]
pub struct LatestEventInfo {
    pub project: String,
    pub model: String,
    /// input + cache_read + cache_creation_5m + cache_creation_1h tokens on
    /// this one event — the same context-size formula used elsewhere in
    /// this codebase's peak-context tracking.
    pub context_tokens: u64,
}

/// A finished (or in-progress) 5H/7D window's peak utilization, as tracked
/// incrementally by `record_window_peak`. `(account_id, bucket, resets_at)`
/// is the window's identity.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct WindowPeak {
    pub bucket: String,
    #[specta(type = String)]
    pub resets_at: DateTime<Utc>,
    #[specta(type = String)]
    pub window_start: DateTime<Utc>,
    pub peak_pct: f64,
    #[specta(type = String)]
    pub peak_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ProjectAttribution {
    pub project: String,
    pub cost_usd: f64,
}

/// One managed account's limit-hit history over a report window. `hits`
/// are windows whose peak_pct cleared the caller-supplied danger threshold
/// — a non-hit window still exists in `window_peaks` but never appears
/// here (changing the threshold in Settings retroactively reclassifies
/// history at read time; nothing is filtered at write time).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct AccountLimitHits {
    pub account_id: String,
    pub email: String,
    pub five_hour_hits: u32,
    pub seven_day_hits: u32,
    /// Always length 24; index = local hour (0-23) a hit's peak was observed.
    pub hourly_distribution: Vec<u32>,
    /// Top 5 by summed cost, across every hit window's [window_start, peak_at] span.
    pub top_projects: Vec<ProjectAttribution>,
}

/// Merge overlapping (or touching) `[start, end]` intervals into the
/// minimal non-overlapping, sorted-by-start set. Standard sweep: sort by
/// start, then fold any interval whose start falls at-or-before the running
/// merged interval's end into that interval, extending its end if needed.
fn merge_intervals(
    mut spans: Vec<(DateTime<Utc>, DateTime<Utc>)>,
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    spans.sort_by_key(|&(start, _)| start);
    let mut merged: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::with_capacity(spans.len());
    for (start, end) in spans {
        match merged.last_mut() {
            Some((_, last_end)) if start <= *last_end => {
                if end > *last_end {
                    *last_end = end;
                }
            }
            _ => merged.push((start, end)),
        }
    }
    merged
}

fn insert_compactions_in_tx(tx: &Transaction<'_>, rows: &[StoredCompaction]) -> Result<usize> {
    if rows.is_empty() {
        return Ok(0);
    }
    let mut stmt = tx.prepare(
        "INSERT OR IGNORE INTO session_compactions
         (ts, source_file, trigger_kind, pre_tokens, post_tokens, uuid)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    let mut inserted = 0;
    for c in rows {
        inserted += stmt.execute(params![
            c.ts.timestamp(),
            c.source_file,
            c.trigger,
            c.pre_tokens as i64,
            c.post_tokens as i64,
            c.uuid,
        ])?;
    }
    Ok(inserted)
}

fn insert_events_in_tx(tx: &Transaction<'_>, events: &[StoredSessionEvent]) -> Result<usize> {
    if events.is_empty() {
        return Ok(0);
    }
    let mut stmt = tx.prepare(
        "INSERT OR IGNORE INTO session_events
        (ts, project, model, input_tokens, output_tokens, cache_read_tokens,
         cache_creation_5m_tokens, cache_creation_1h_tokens, cost_usd,
         source_file, source_line, event_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )?;
    let mut inserted = 0;
    for e in events {
        inserted += stmt.execute(params![
            e.ts.timestamp(),
            e.project,
            e.model,
            e.input_tokens as i64,
            e.output_tokens as i64,
            e.cache_read_tokens as i64,
            e.cache_creation_5m_tokens as i64,
            e.cache_creation_1h_tokens as i64,
            e.cost_usd,
            e.source_file,
            e.source_line,
            e.event_id,
        ])?;
    }
    Ok(inserted)
}

impl Db {
    pub fn upsert_account(&self, acc: &StoredAccount) -> Result<()> {
        let now = Utc::now().timestamp();
        self.conn().execute(
            "INSERT INTO accounts (id, email, display_name, last_seen_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET email=excluded.email,
                                            display_name=excluded.display_name,
                                            last_seen_at=excluded.last_seen_at",
            params![acc.id, acc.email, acc.display_name, now],
        )?;
        Ok(())
    }

    /// Remove the SQLite row for `account_uuid`. Idempotent (no error when
    /// the row is already absent). Paired with AccountManager::remove so the
    /// warmup state for a removed account does not linger.
    pub fn delete_account(&self, account_uuid: &str) -> Result<()> {
        self.conn().execute(
            "DELETE FROM accounts WHERE id = ?1",
            params![account_uuid],
        )?;
        Ok(())
    }

    pub fn insert_snapshot(
        &self,
        account_id: &str,
        fetched_at: DateTime<Utc>,
        payload_json: &str,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO api_snapshots (account_id, fetched_at, payload_json) VALUES (?1, ?2, ?3)",
            params![account_id, fetched_at.timestamp(), payload_json],
        )?;
        Ok(())
    }

    pub fn latest_snapshot(
        &self,
        account_id: &str,
    ) -> Result<Option<(DateTime<Utc>, String)>> {
        let conn = self.conn();
        let row = conn
            .query_row(
                "SELECT fetched_at, payload_json FROM api_snapshots
                 WHERE account_id = ?1 ORDER BY fetched_at DESC LIMIT 1",
                params![account_id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?;
        Ok(row.map(|(ts, p)| (DateTime::from_timestamp(ts, 0).unwrap(), p)))
    }

    /// Bound snapshot history to the newest `keep_per_account` rows per
    /// account. Snapshots are written on every successful poll, so without
    /// pruning the table grows ~700 rows/day/account forever. Returns the
    /// number of rows deleted.
    pub fn prune_snapshots(&self, keep_per_account: u32) -> Result<usize> {
        let deleted = self.conn().execute(
            "DELETE FROM api_snapshots
             WHERE id NOT IN (
                 SELECT id FROM (
                     SELECT id,
                            ROW_NUMBER() OVER (PARTITION BY account_id
                                               ORDER BY fetched_at DESC, id DESC) AS rn
                     FROM api_snapshots
                 ) WHERE rn <= ?1
             )",
            params![keep_per_account],
        )?;
        Ok(deleted)
    }

    /// The pricing revision last applied to stored events, or `None` when
    /// historical costs have never been (re)computed under the current table.
    pub fn repriced_version(&self) -> Result<Option<u32>> {
        let v: Option<String> = self
            .conn()
            .query_row(
                "SELECT value FROM settings WHERE key = 'repriced_pricing_version'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.and_then(|s| s.parse().ok()))
    }

    pub fn set_repriced_version(&self, version: u32) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('repriced_pricing_version', ?1)",
            params![version.to_string()],
        )?;
        Ok(())
    }

    /// Recompute `cost_usd` for every stored session event using the current
    /// pricing table. Used as a one-time migration when pricing entries are
    /// added or corrected (e.g. fable-5 events that were costed 0.0 before
    /// the model had an entry, opus-4-8 events costed at legacy rates).
    /// Rows already matching the current table are untouched. Returns the
    /// number of rows updated.
    pub fn reprice_outdated_events(
        &self,
        pricing: &crate::jsonl_parser::pricing::PricingTable,
    ) -> Result<usize> {
        // (id, model, input, output, cache_read, cache_5m, cache_1h, old cost)
        type EventCostRow = (i64, String, i64, i64, i64, i64, i64, f64);

        // Read first, then write: iterating a SELECT while issuing UPDATEs
        // against the same table on one connection is asking for skipped
        // rows, so the scan is materialized up front.
        let rows: Vec<EventCostRow> = {
            let conn = self.conn();
            let mut stmt = conn.prepare(
                "SELECT id, model, input_tokens, output_tokens, cache_read_tokens,
                        cache_creation_5m_tokens, cache_creation_1h_tokens, cost_usd
                 FROM session_events",
            )?;
            let mapped = stmt.query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut updated = 0usize;
        {
            let mut write =
                tx.prepare("UPDATE session_events SET cost_usd = ?2 WHERE id = ?1")?;
            for (id, model, input, output, cr, c5m, c1h, old_cost) in rows {
                let new_cost = pricing.cost_for(
                    &model,
                    input as u64,
                    output as u64,
                    cr as u64,
                    c5m as u64,
                    c1h as u64,
                );
                if (new_cost - old_cost).abs() > 1e-9 {
                    write.execute(params![id, new_cost])?;
                    updated += 1;
                }
            }
        }
        tx.commit()?;
        Ok(updated)
    }

    pub fn insert_events(&self, events: &[StoredSessionEvent]) -> Result<usize> {
        if events.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let inserted = insert_events_in_tx(&tx, events)?;
        tx.commit()?;
        Ok(inserted)
    }

    /// Sum `cost_usd` per project for events in `[from, to]`, aggregated in
    /// SQL rather than materializing full `StoredSessionEvent` rows — used by
    /// `limit_hit_stats` where only the per-project total matters, and a
    /// single hit window's span can cover most of a week.
    pub fn project_costs_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<(String, f64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT project, SUM(cost_usd) FROM session_events
             WHERE ts BETWEEN ?1 AND ?2 GROUP BY project",
        )?;
        let rows = stmt.query_map(params![from.timestamp(), to.timestamp()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn events_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<StoredSessionEvent>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            // The account lookup is a correlated scalar subquery, not a
            // LEFT JOIN. Two reasons, both load-bearing:
            //   * Correctness: a join emits one row per matching interval, so
            //     any overlap in `account_intervals` would silently duplicate
            //     the event and double-count its tokens/cost everywhere
            //     downstream. `LIMIT 1` makes that structurally impossible —
            //     and `ORDER BY started_at DESC` picks the most-recently-opened
            //     matching interval, i.e. the newer swap wins.
            //   * Performance: this is an ORDER BY + LIMIT over a range on
            //     `idx_account_intervals_span(started_at, ended_at)`, so it
            //     costs a bounded index seek per event rather than a scan of
            //     every interval per event (which grew quadratically as both
            //     tables grew with the app's age).
            // Semantics are unchanged for well-formed intervals: same
            // half-open `[started_at, ended_at)` window, NULL when no interval
            // covers the event.
            "SELECT e.ts, e.project, e.model, e.input_tokens, e.output_tokens, e.cache_read_tokens,
                    e.cache_creation_5m_tokens, e.cache_creation_1h_tokens, e.cost_usd,
                    e.source_file, e.source_line, e.event_id,
                    (SELECT ai.account_uuid FROM account_intervals ai
                      WHERE ai.started_at <= e.ts AND (ai.ended_at IS NULL OR e.ts < ai.ended_at)
                      ORDER BY ai.started_at DESC LIMIT 1) AS account_uuid
             FROM session_events e
             WHERE e.ts BETWEEN ?1 AND ?2 ORDER BY e.ts DESC",
        )?;
        let rows = stmt.query_map(params![from.timestamp(), to.timestamp()], |r| {
            Ok(StoredSessionEvent {
                ts: DateTime::from_timestamp(r.get(0)?, 0).unwrap(),
                project: r.get(1)?,
                model: r.get(2)?,
                input_tokens: r.get::<_, i64>(3)? as u64,
                output_tokens: r.get::<_, i64>(4)? as u64,
                cache_read_tokens: r.get::<_, i64>(5)? as u64,
                cache_creation_5m_tokens: r.get::<_, i64>(6)? as u64,
                cache_creation_1h_tokens: r.get::<_, i64>(7)? as u64,
                cost_usd: r.get(8)?,
                source_file: r.get(9)?,
                source_line: r.get(10)?,
                event_id: r.get(11)?,
                account_uuid: r.get(12)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Lifetime `(input+output tokens, cost)` per conversation, keyed by the
    /// parent transcript's path relative to the Claude projects root.
    ///
    /// Subagent transcripts (`<sessionId>/subagents/agent-*.jsonl`) fold onto
    /// their parent, matching how the Cost tab aggregates: a subagent's API
    /// calls are real spend that the parent transcript does not record, and
    /// they are not separately resumable sessions.
    pub fn session_totals(&self) -> Result<std::collections::HashMap<String, (u64, f64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT source_file, SUM(input_tokens + output_tokens), SUM(cost_usd)
             FROM session_events GROUP BY source_file",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, f64>(2)?))
        })?;
        let mut out: std::collections::HashMap<String, (u64, f64)> =
            std::collections::HashMap::new();
        for row in rows {
            let (source_file, tokens, cost) = row?;
            let key = match source_file.find("/subagents/") {
                Some(i) => format!("{}.jsonl", &source_file[..i]),
                None => source_file,
            };
            let slot = out.entry(key).or_insert((0, 0.0));
            slot.0 += tokens.max(0) as u64;
            slot.1 += cost;
        }
        Ok(out)
    }

    /// Distinct accounts whose interval covers at least one event in each
    /// conversation, keyed the same way as `session_totals` (subagent
    /// transcripts folded onto their parent's `source_file`). Used by
    /// `get_repo_breakdown` to badge each repo/project with the account(s)
    /// that worked in it — a conversation almost always maps to exactly one
    /// account, but nothing prevents more if it happened to span a swap.
    ///
    /// A `None` element means "some of this conversation's events are not
    /// attributable to any managed account" — pre-feature history, or a gap
    /// where no managed account was live. It is deliberately kept rather than
    /// filtered out: dropping it made a repo whose sessions are 90%
    /// unattributed render as if it were 100% one account, which is worse
    /// than showing an explicit "Unknown". Every other surface in this
    /// feature renders `None` as a muted Unknown badge; this matches.
    pub fn account_uuids_by_source_file(
        &self,
    ) -> Result<std::collections::HashMap<String, Vec<Option<String>>>> {
        let conn = self.conn();
        // Correlated scalar subquery rather than a LEFT JOIN, for the same
        // correctness (at most one row per event, so overlapping intervals
        // can't duplicate) and performance (bounded index seek on
        // `idx_account_intervals_span` instead of a per-event interval scan)
        // reasons documented on `events_between`.
        let mut stmt = conn.prepare(
            "SELECT DISTINCT e.source_file,
                    (SELECT ai.account_uuid FROM account_intervals ai
                      WHERE ai.started_at <= e.ts AND (ai.ended_at IS NULL OR e.ts < ai.ended_at)
                      ORDER BY ai.started_at DESC LIMIT 1) AS account_uuid
             FROM session_events e",
        )?;
        let rows =
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?;

        let mut out: std::collections::HashMap<String, Vec<Option<String>>> =
            std::collections::HashMap::new();
        for row in rows {
            let (source_file, account_uuid) = row?;
            let key = match source_file.find("/subagents/") {
                Some(i) => format!("{}.jsonl", &source_file[..i]),
                None => source_file,
            };
            let list = out.entry(key).or_default();
            if !list.contains(&account_uuid) {
                list.push(account_uuid);
            }
        }
        Ok(out)
    }

    /// Aggregate tokens/cost for one live session: the parent transcript
    /// plus any subagent transcripts folded under it via the same
    /// `/subagents/` convention `session_totals()` uses for the full-table
    /// historical report. Scoped to a single session (two bound params) vs.
    /// `session_totals()`'s full-table GROUP BY, so it does less per-row
    /// work — but there is no index on `source_file`, so this is still a
    /// full table scan, same O(n) cost class as `session_totals()`. Adding
    /// an index to fix that was deliberately left out of scope for this
    /// feature (no DB schema change).
    pub fn live_session_totals(&self, parent_source_file: &str) -> Result<(u64, f64)> {
        let conn = self.conn();
        let prefix = parent_source_file
            .strip_suffix(".jsonl")
            .unwrap_or(parent_source_file);
        // `source_file` is written using the current platform's native path
        // separator, unnormalized (see jsonl_parser::walker::ingest_file),
        // so the LIKE pattern must match with that same separator rather
        // than a hardcoded '/' — which would silently match nothing on
        // Windows.
        let subagent_pattern = format!("{prefix}{sep}subagents{sep}%", sep = std::path::MAIN_SEPARATOR);
        conn.query_row(
            "SELECT COALESCE(SUM(input_tokens + output_tokens), 0), COALESCE(SUM(cost_usd), 0.0)
             FROM session_events WHERE source_file = ?1 OR source_file LIKE ?2",
            params![parent_source_file, subagent_pattern],
            |r| {
                let tokens: i64 = r.get(0)?;
                let cost: f64 = r.get(1)?;
                Ok((tokens.max(0) as u64, cost))
            },
        )
        .map_err(Into::into)
    }

    /// The parent file's most recent event (by `source_line`), or `None` if
    /// nothing has been ingested for it yet. Subagent files are excluded by
    /// the exact `source_file` match (no LIKE) — see `LatestEventInfo`.
    pub fn latest_event_for_file(&self, source_file: &str) -> Result<Option<LatestEventInfo>> {
        let conn = self.conn();
        conn.query_row(
            "SELECT project, model, input_tokens, cache_read_tokens,
                    cache_creation_5m_tokens, cache_creation_1h_tokens
             FROM session_events WHERE source_file = ?1
             ORDER BY source_line DESC LIMIT 1",
            params![source_file],
            |r| {
                let input: i64 = r.get(2)?;
                let cache_read: i64 = r.get(3)?;
                let cache_5m: i64 = r.get(4)?;
                let cache_1h: i64 = r.get(5)?;
                Ok(LatestEventInfo {
                    project: r.get(0)?,
                    model: r.get(1)?,
                    context_tokens: (input.max(0) + cache_read.max(0) + cache_5m.max(0) + cache_1h.max(0))
                        as u64,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn compactions_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<StoredCompaction>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT ts, source_file, trigger_kind, pre_tokens, post_tokens, uuid
             FROM session_compactions WHERE ts BETWEEN ?1 AND ?2 ORDER BY ts DESC",
        )?;
        let rows = stmt.query_map(params![from.timestamp(), to.timestamp()], |r| {
            Ok(StoredCompaction {
                ts: DateTime::from_timestamp(r.get(0)?, 0).unwrap(),
                source_file: r.get(1)?,
                trigger: r.get(2)?,
                pre_tokens: r.get::<_, i64>(3)? as u64,
                post_tokens: r.get::<_, i64>(4)? as u64,
                uuid: r.get(5)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn prune_events_older_than(&self, cutoff: DateTime<Utc>) -> Result<usize> {
        let rows = self
            .conn()
            .execute("DELETE FROM session_events WHERE ts < ?1", params![cutoff.timestamp()])?;
        Ok(rows)
    }

    pub fn get_cursor(&self, file: &str) -> Result<Option<(i64, i64)>> {
        let conn = self.conn();
        let row = conn
            .query_row(
                "SELECT last_mtime_ns, byte_offset FROM jsonl_cursors WHERE file_path = ?1",
                params![file],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?;
        Ok(row)
    }

    pub fn set_cursor(&self, file: &str, mtime_ns: i64, offset: i64) -> Result<()> {
        self.conn().execute(
            "INSERT INTO jsonl_cursors (file_path, last_mtime_ns, byte_offset) VALUES (?1, ?2, ?3)
             ON CONFLICT(file_path) DO UPDATE SET
               last_mtime_ns = MAX(excluded.last_mtime_ns, jsonl_cursors.last_mtime_ns),
               byte_offset   = MAX(excluded.byte_offset,   jsonl_cursors.byte_offset)",
            params![file, mtime_ns, offset],
        )?;
        Ok(())
    }

    /// Insert events and advance the JSONL cursor in a single SQLite
    /// transaction. This prevents the race where two concurrent callers
    /// (backfill task + watcher) split the two writes across transactions,
    /// letting the slower caller's cursor write regress the faster caller's
    /// progress and causing repeated re-ingestion of the same lines.
    /// Insert events and advance the JSONL cursor in a single SQLite
    /// transaction. This prevents the race where two concurrent callers
    /// (backfill task + watcher) split the two writes across transactions,
    /// letting the slower caller's cursor write regress the faster caller's
    /// progress and causing repeated re-ingestion of the same lines.
    pub fn ingest_atomic(
        &self,
        file: &str,
        events: &[StoredSessionEvent],
        compactions: &[StoredCompaction],
        mtime_ns: i64,
        byte_offset: i64,
    ) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let inserted = insert_events_in_tx(&tx, events)?;
        // Same transaction as the events and the cursor: a compaction that
        // landed while the cursor advanced past it could never be recovered,
        // since the walker would not re-read those bytes.
        insert_compactions_in_tx(&tx, compactions)?;
        tx.execute(
            "INSERT INTO jsonl_cursors (file_path, last_mtime_ns, byte_offset) VALUES (?1, ?2, ?3)
             ON CONFLICT(file_path) DO UPDATE SET
               last_mtime_ns = MAX(excluded.last_mtime_ns, jsonl_cursors.last_mtime_ns),
               byte_offset   = MAX(excluded.byte_offset,   jsonl_cursors.byte_offset)",
            params![file, mtime_ns, byte_offset],
        )?;
        tx.commit()?;
        Ok(inserted)
    }

    pub fn notification_last_fired(
        &self,
        account_id: &str,
        bucket: &str,
        threshold: i64,
    ) -> Result<Option<DateTime<Utc>>> {
        let conn = self.conn();
        let row = conn
            .query_row(
                "SELECT last_fired_at FROM notification_state
                 WHERE account_id = ?1 AND bucket = ?2 AND threshold = ?3",
                params![account_id, bucket, threshold],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        Ok(row.map(|ts| DateTime::from_timestamp(ts, 0).unwrap()))
    }

    pub fn record_notification_fired(
        &self,
        account_id: &str,
        bucket: &str,
        threshold: i64,
        at: DateTime<Utc>,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO notification_state (account_id, bucket, threshold, last_fired_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(account_id, bucket, threshold) DO UPDATE SET last_fired_at=excluded.last_fired_at",
            params![account_id, bucket, threshold, at.timestamp()],
        )?;
        Ok(())
    }

    /// Record one poll's reading for a window, keeping the running peak.
    /// `(account_id, bucket, resets_at)` identifies the window — a new
    /// `resets_at` value creates a fresh row rather than updating the old
    /// one, which is how a window rollover gets detected with no explicit
    /// bookkeeping. Best-effort from the caller's perspective: errors
    /// propagate but callers on the poll-loop path treat them as
    /// log-and-continue (see poll_loop.rs).
    pub fn record_window_peak(
        &self,
        account_id: &str,
        bucket: &str,
        resets_at: DateTime<Utc>,
        observed_at: DateTime<Utc>,
        pct: f64,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO window_peaks (account_id, bucket, resets_at, window_start, peak_pct, peak_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?4)
             ON CONFLICT(account_id, bucket, resets_at) DO UPDATE SET
                 peak_pct = MAX(peak_pct, excluded.peak_pct),
                 peak_at = CASE WHEN excluded.peak_pct > peak_pct THEN excluded.peak_at ELSE peak_at END,
                 window_start = MIN(window_start, excluded.window_start)",
            params![account_id, bucket, resets_at.timestamp(), observed_at.timestamp(), pct],
        )?;
        Ok(())
    }

    /// Closes the currently-open account interval (if its account differs
    /// from `new_active`) and opens a new one for `new_active` (if `Some`
    /// and different from what was already open). No-op if `new_active`
    /// already matches the open interval — both `swap_to_account` and the
    /// poll loop's active-account reconciliation call this, and the poll
    /// loop's later observation of an already-recorded in-app swap must not
    /// create a spurious zero-length interval.
    ///
    /// `at` is captured by the caller *before* this method takes the DB lock,
    /// so two racing callers (an in-app `swap_to_account` and the poll loop's
    /// reconciliation of the same swap) can arrive with out-of-order
    /// timestamps. The recorded timestamp is therefore clamped forward to the
    /// currently-open interval's `started_at`, which keeps intervals
    /// monotonically ordered and non-overlapping (worst case a zero-length
    /// interval) instead of letting a late-but-earlier caller open an interval
    /// that starts before the one it closes.
    pub fn record_account_transition(
        &self,
        new_active: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let open: Option<(String, i64)> = tx
            .query_row(
                "SELECT account_uuid, started_at FROM account_intervals WHERE ended_at IS NULL",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;

        if open.as_ref().map(|(uuid, _)| uuid.as_str()) == new_active {
            return Ok(());
        }

        let ts = match open.as_ref() {
            Some((_, started_at)) => at.timestamp().max(*started_at),
            None => at.timestamp(),
        };

        if open.is_some() {
            tx.execute(
                "UPDATE account_intervals SET ended_at = ?1 WHERE ended_at IS NULL",
                params![ts],
            )?;
        }
        if let Some(uuid) = new_active {
            tx.execute(
                "INSERT INTO account_intervals (account_uuid, started_at, ended_at) VALUES (?1, ?2, NULL)",
                params![uuid, ts],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// All window peaks for one account whose `window_start` falls in
    /// `[from, to]`, oldest first.
    pub fn window_peaks_between(
        &self,
        account_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<WindowPeak>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT bucket, resets_at, window_start, peak_pct, peak_at
             FROM window_peaks
             WHERE account_id = ?1 AND window_start BETWEEN ?2 AND ?3
             ORDER BY peak_at ASC",
        )?;
        let rows = stmt.query_map(params![account_id, from.timestamp(), to.timestamp()], |r| {
            Ok(WindowPeak {
                bucket: r.get(0)?,
                resets_at: DateTime::from_timestamp(r.get(1)?, 0).unwrap(),
                window_start: DateTime::from_timestamp(r.get(2)?, 0).unwrap(),
                peak_pct: r.get(3)?,
                peak_at: DateTime::from_timestamp(r.get(4)?, 0).unwrap(),
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Aggregate one account's limit-hit history: hit counts per bucket,
    /// an hour-of-day distribution of when hits peaked, and the projects
    /// that consumed each hit window beforehand. Read-only, computed
    /// entirely from already-stored data.
    ///
    /// A `five_hour` hit window's `[window_start, peak_at]` span is always
    /// contained within some `seven_day` hit window's span (5 hours falls
    /// within 7 days), so querying each hit window's span independently
    /// would double-count every event in the overlap. Instead, the hit
    /// windows' spans are merged into non-overlapping intervals first, and
    /// cost is aggregated in SQL once per merged interval via
    /// `project_costs_between`.
    pub fn limit_hit_stats(
        &self,
        account_id: &str,
        email: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        danger_threshold: f64,
    ) -> Result<AccountLimitHits> {
        let peaks = self.window_peaks_between(account_id, from, to)?;
        let mut five_hour_hits = 0u32;
        let mut seven_day_hits = 0u32;
        let mut hourly_distribution = vec![0u32; 24];
        let mut hit_spans: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();

        for peak in &peaks {
            if peak.peak_pct < danger_threshold {
                continue;
            }
            match peak.bucket.as_str() {
                "five_hour" => five_hour_hits += 1,
                "seven_day" => seven_day_hits += 1,
                _ => continue,
            }
            let hour = peak.peak_at.with_timezone(&chrono::Local).hour() as usize;
            hourly_distribution[hour] += 1;
            hit_spans.push((peak.window_start, peak.peak_at));
        }

        let mut project_totals: std::collections::HashMap<String, f64> =
            std::collections::HashMap::new();
        for (start, end) in merge_intervals(hit_spans) {
            for (project, cost) in self.project_costs_between(start, end)? {
                *project_totals.entry(project).or_insert(0.0) += cost;
            }
        }

        let mut top_projects: Vec<ProjectAttribution> = project_totals
            .into_iter()
            .map(|(project, cost_usd)| ProjectAttribution { project, cost_usd })
            .collect();
        // total_cmp is panic-free on NaN; the project-name tiebreak keeps
        // ordering deterministic despite project_totals being a HashMap.
        top_projects.sort_by(|a, b| {
            b.cost_usd
                .total_cmp(&a.cost_usd)
                .then_with(|| a.project.cmp(&b.project))
        });
        top_projects.truncate(5);

        Ok(AccountLimitHits {
            account_id: account_id.to_string(),
            email: email.to_string(),
            five_hour_hits,
            seven_day_hits,
            hourly_distribution,
            top_projects,
        })
    }

    /// Persist user settings to the `settings` table as a single JSON blob
    /// keyed on `"settings"`. Subsequent calls overwrite the previous value.
    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        let json = serde_json::to_string(settings)?;
        self.conn().execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('settings', ?1)",
            params![json],
        )?;
        Ok(())
    }

    /// Load user settings from the `settings` table. Returns `None` when no
    /// row has been written yet (first launch), so the caller can fall back to
    /// `Settings::default()`.
    pub fn load_settings(&self) -> Result<Option<Settings>> {
        let conn = self.conn();
        let row = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'settings'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        match row {
            None => Ok(None),
            Some(json) => {
                let settings = serde_json::from_str(&json)?;
                Ok(Some(settings))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Db;
    use tempfile::tempdir;

    fn fresh_db() -> (tempfile::TempDir, Db) {
        let dir = tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        db.upsert_account(&StoredAccount {
            id: "acc1".into(),
            email: "a@example.com".into(),
            display_name: None,
        })
        .unwrap();
        (dir, db)
    }

    #[test]
    fn snapshot_roundtrip() {
        let (_dir, db) = fresh_db();
        let now = Utc::now();
        db.insert_snapshot("acc1", now, r#"{"five_hour":null}"#)
            .unwrap();
        let (ts, payload) = db.latest_snapshot("acc1").unwrap().expect("snapshot");
        assert_eq!(ts.timestamp(), now.timestamp());
        assert!(payload.contains("five_hour"));
    }

    #[test]
    fn prune_snapshots_keeps_latest_n_per_account() {
        let (_dir, db) = fresh_db();
        db.upsert_account(&StoredAccount {
            id: "acc2".into(),
            email: "b@example.com".into(),
            display_name: None,
        })
        .unwrap();
        let base = Utc::now();
        for i in 0..5 {
            db.insert_snapshot(
                "acc1",
                base + chrono::Duration::seconds(i),
                &format!(r#"{{"n":{i}}}"#),
            )
            .unwrap();
        }
        for i in 0..2 {
            db.insert_snapshot(
                "acc2",
                base + chrono::Duration::seconds(i),
                &format!(r#"{{"n":{i}}}"#),
            )
            .unwrap();
        }

        let pruned = db.prune_snapshots(2).unwrap();
        assert_eq!(pruned, 3, "acc1 drops 3 of 5; acc2 already within keep");

        let (_, payload) = db.latest_snapshot("acc1").unwrap().unwrap();
        assert!(payload.contains("\"n\":4"), "newest snapshot survives");
        let remaining: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM api_snapshots WHERE account_id = 'acc2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 2, "accounts within the keep count are untouched");
    }

    fn event(model: &str, input: u64, output: u64, cost: f64, id: &str) -> StoredSessionEvent {
        StoredSessionEvent {
            ts: Utc::now(),
            project: "p".into(),
            model: model.into(),
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: cost,
            source_file: "f.jsonl".into(),
            source_line: 1,
            event_id: id.into(),
            account_uuid: None,
        }
    }

    fn stored_cost(db: &Db, event_id: &str) -> f64 {
        db.conn()
            .query_row(
                "SELECT cost_usd FROM session_events WHERE event_id = ?1",
                params![event_id],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn reprice_recomputes_costs_with_current_table() {
        let (_dir, db) = fresh_db();
        // Written before fable-5 had a pricing entry (costed 0.0) and before
        // opus-4-8 got its own entry (costed at legacy $15/$75 rates).
        db.insert_events(&[
            event("claude-fable-5", 100_000, 1_000, 0.0, "e_fable"),
            event("claude-opus-4-8", 5_000, 0, 0.075, "e_o48"),
            // Relay models previously costed 0.0.
            event("MiniMax-M2.7", 1_000_000, 0, 0.0, "e_mm"),
        ])
        .unwrap();
        let pricing = crate::jsonl_parser::pricing::PricingTable::bundled().unwrap();
        let updated = db.reprice_outdated_events(&pricing).unwrap();
        assert_eq!(updated, 3);
        // fable: 0.1M in × $10 + 0.001M out × $50 = $1.05
        assert!((stored_cost(&db, "e_fable") - 1.05).abs() < 1e-6);
        // opus-4-8: 0.005M in × $5 = $0.025
        assert!((stored_cost(&db, "e_o48") - 0.025).abs() < 1e-6);
        // minimax: 1M in × $0.30 = $0.30
        assert!((stored_cost(&db, "e_mm") - 0.30).abs() < 1e-6);
    }

    #[test]
    fn reprice_leaves_already_correct_rows_untouched() {
        let (_dir, db) = fresh_db();
        // sonnet-4-6 base: 0.1M in × $3 = $0.30 — already correct.
        db.insert_events(&[event("claude-sonnet-4-6", 100_000, 0, 0.30, "e_ok")])
            .unwrap();
        let pricing = crate::jsonl_parser::pricing::PricingTable::bundled().unwrap();
        assert_eq!(db.reprice_outdated_events(&pricing).unwrap(), 0);
    }

    #[test]
    fn reprice_is_idempotent() {
        let (_dir, db) = fresh_db();
        db.insert_events(&[event("claude-fable-5", 100_000, 0, 0.0, "e_f")])
            .unwrap();
        let pricing = crate::jsonl_parser::pricing::PricingTable::bundled().unwrap();
        assert_eq!(db.reprice_outdated_events(&pricing).unwrap(), 1);
        assert_eq!(db.reprice_outdated_events(&pricing).unwrap(), 0);
    }

    #[test]
    fn repriced_version_roundtrip() {
        let (_dir, db) = fresh_db();
        assert_eq!(db.repriced_version().unwrap(), None);
        db.set_repriced_version(2).unwrap();
        assert_eq!(db.repriced_version().unwrap(), Some(2));
    }

    #[test]
    fn events_insert_and_dedupe() {
        let (_dir, db) = fresh_db();
        let e = StoredSessionEvent {
            ts: Utc::now(),
            project: "p".into(),
            model: "sonnet-4-6".into(),
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 0.001,
            source_file: "f.jsonl".into(),
            source_line: 1,
            event_id: "req_1:msg_1".into(),
            account_uuid: None,
        };
        assert_eq!(db.insert_events(std::slice::from_ref(&e)).unwrap(), 1);
        assert_eq!(
            db.insert_events(std::slice::from_ref(&e)).unwrap(),
            0,
            "same event_id is rejected"
        );
    }

    /// The exact regression that v1 missed: Claude Code can write the same
    /// `message.usage` block to multiple offsets in the same file (retries,
    /// partial rewinds). Different (source_file, source_line) but identical
    /// event_id — must dedupe.
    #[test]
    fn dedupe_catches_same_event_at_different_offsets() {
        let (_dir, db) = fresh_db();
        let mk = |line: i64| StoredSessionEvent {
            ts: Utc::now(),
            project: "p".into(),
            model: "opus-4-7".into(),
            input_tokens: 6,
            output_tokens: 332,
            cache_read_tokens: 19099,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 396681,
            cost_usd: 11.95,
            source_file: "/Users/me/.claude/projects/p/abc.jsonl".into(),
            source_line: line,
            event_id: "req_abc:msg_xyz".into(),
            account_uuid: None,
        };
        // Same event written at offsets 100, 1000, 2000 — only the first lands.
        assert_eq!(db.insert_events(&[mk(100), mk(1000), mk(2000)]).unwrap(), 1);
    }

    #[test]
    fn session_totals_fold_subagents_onto_their_parent() {
        let (_dir, db) = fresh_db();
        let mk = |src: &str, id: &str, tokens: u64, cost: f64| StoredSessionEvent {
            ts: Utc::now(),
            project: "p".into(),
            model: "claude-opus-5".into(),
            input_tokens: tokens,
            output_tokens: 0,
            cache_read_tokens: 9_999, // must NOT count toward total_tokens
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: cost,
            source_file: src.into(),
            source_line: 0,
            event_id: id.into(),
            account_uuid: None,
        };
        db.insert_events(&[
            mk("proj/abc.jsonl", "e1", 100, 1.0),
            mk("proj/abc/subagents/agent-aaa.jsonl", "e2", 30, 0.5),
            mk("proj/abc/subagents/agent-bbb.jsonl", "e3", 20, 0.25),
            mk("proj/other.jsonl", "e4", 7, 0.1),
        ])
        .unwrap();

        let totals = db.session_totals().unwrap();
        assert_eq!(
            totals.get("proj/abc.jsonl"),
            Some(&(150, 1.75)),
            "both subagents must roll up into the parent conversation"
        );
        assert_eq!(totals.get("proj/other.jsonl"), Some(&(7, 0.1)));
        assert!(
            !totals.contains_key("proj/abc/subagents/agent-aaa.jsonl"),
            "a subagent is not a session and must not appear as its own key"
        );
    }

    #[test]
    fn cursor_roundtrip() {
        let (_dir, db) = fresh_db();
        assert!(db.get_cursor("f.jsonl").unwrap().is_none());
        db.set_cursor("f.jsonl", 123, 456).unwrap();
        assert_eq!(db.get_cursor("f.jsonl").unwrap(), Some((123, 456)));
    }

    #[test]
    fn notification_state_roundtrip() {
        let (_dir, db) = fresh_db();
        assert!(db
            .notification_last_fired("acc1", "five_hour", 75)
            .unwrap()
            .is_none());
        let now = Utc::now();
        db.record_notification_fired("acc1", "five_hour", 75, now)
            .unwrap();
        let got = db
            .notification_last_fired("acc1", "five_hour", 75)
            .unwrap()
            .unwrap();
        assert_eq!(got.timestamp(), now.timestamp());
    }

    #[test]
    fn settings_roundtrip() {
        let (_dir, db) = fresh_db();

        // No row written yet — should return None.
        assert!(db.load_settings().unwrap().is_none());

        // Write non-default settings.
        let s = Settings {
            polling_interval_secs: 60,
            stagger_gap_secs: 45,
            thresholds: vec![50, 80, 95],
            payg_threshold: 65,
            theme: "dark".into(),
            launch_at_login: true,
            crash_reports: true,
            preferred_auth_source: None,
            terminal: Some(crate::providers::launcher::Terminal::Iterm2),
            notify_session_finished: false,
            notify_context_warning: false,
        };
        db.save_settings(&s).unwrap();

        // Read back and assert every field survived the round-trip.
        let loaded = db.load_settings().unwrap().expect("settings row");
        assert_eq!(loaded.polling_interval_secs, s.polling_interval_secs);
        assert_eq!(loaded.stagger_gap_secs, s.stagger_gap_secs);
        assert_eq!(loaded.thresholds, s.thresholds);
        assert_eq!(loaded.payg_threshold, s.payg_threshold);
        assert_eq!(loaded.theme, s.theme);
        assert_eq!(loaded.launch_at_login, s.launch_at_login);
        assert_eq!(loaded.crash_reports, s.crash_reports);
        assert_eq!(loaded.terminal, s.terminal);
        assert_eq!(loaded.notify_session_finished, s.notify_session_finished);
        assert_eq!(loaded.notify_context_warning, s.notify_context_warning);

        // Overwrite and confirm the latest value wins (UPSERT).
        let s2 = Settings { polling_interval_secs: 120, ..Settings::default() };
        db.save_settings(&s2).unwrap();
        let loaded2 = db.load_settings().unwrap().expect("updated settings row");
        assert_eq!(loaded2.polling_interval_secs, 120);
        assert_eq!(loaded2.theme, Settings::default().theme);
    }

    #[test]
    fn prune_removes_old_events() {
        let (_dir, db) = fresh_db();
        let old = Utc::now() - chrono::Duration::days(200);
        let recent = Utc::now();
        let mk = |ts, line: i64| StoredSessionEvent {
            ts,
            project: "p".into(),
            model: "sonnet-4-6".into(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 0.0,
            source_file: "f.jsonl".into(),
            source_line: line,
            event_id: format!("ev_{line}"),
            account_uuid: None,
        };
        db.insert_events(&[mk(old, 1), mk(recent, 2)]).unwrap();
        let cutoff = Utc::now() - chrono::Duration::days(90);
        assert_eq!(db.prune_events_older_than(cutoff).unwrap(), 1);
    }

    fn mk_event(source_file: &str, source_line: i64, input: u64, cost: f64) -> StoredSessionEvent {
        StoredSessionEvent {
            ts: Utc::now(),
            project: "my-proj".into(),
            model: "claude-opus-5".into(),
            input_tokens: input,
            output_tokens: 5,
            cache_read_tokens: 100,
            cache_creation_5m_tokens: 10,
            cache_creation_1h_tokens: 0,
            cost_usd: cost,
            source_file: source_file.into(),
            source_line,
            event_id: format!("{source_file}:{source_line}"),
            account_uuid: None,
        }
    }

    #[test]
    fn live_session_totals_folds_subagents_onto_parent() {
        let (_dir, db) = fresh_db();
        let parent = "-Users-me-proj/sess1.jsonl";
        db.ingest_atomic(parent, &[mk_event(parent, 0, 100, 0.5), mk_event(parent, 1, 100, 0.5)], &[], 1, 200).unwrap();
        let sub = "-Users-me-proj/sess1/subagents/agent-a.jsonl";
        db.ingest_atomic(sub, &[mk_event(sub, 0, 50, 0.1)], &[], 1, 100).unwrap();
        // A different session must not leak in.
        let other = "-Users-me-proj/sess2.jsonl";
        db.ingest_atomic(other, &[mk_event(other, 0, 999, 9.0)], &[], 1, 100).unwrap();

        let (tokens, cost) = db.live_session_totals(parent).unwrap();
        // input+output per event: (100+5)+(100+5)+(50+5) = 265
        assert_eq!(tokens, 265);
        assert!((cost - 1.1).abs() < 1e-9);
    }

    #[test]
    fn live_session_totals_zero_for_unknown_session() {
        let (_dir, db) = fresh_db();
        let (tokens, cost) = db.live_session_totals("-Users-me-proj/never-seen.jsonl").unwrap();
        assert_eq!(tokens, 0);
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn latest_event_for_file_returns_most_recent_by_source_line() {
        let (_dir, db) = fresh_db();
        let parent = "-Users-me-proj/sess1.jsonl";
        db.ingest_atomic(
            parent,
            &[
                mk_event(parent, 0, 1000, 0.1),
                { let mut e = mk_event(parent, 5, 2000, 0.2); e.model = "claude-sonnet-5".into(); e.cache_read_tokens = 500; e.cache_creation_5m_tokens = 50; e },
            ],
            &[],
            1,
            300,
        ).unwrap();
        let info = db.latest_event_for_file(parent).unwrap().expect("row present");
        assert_eq!(info.project, "my-proj");
        assert_eq!(info.model, "claude-sonnet-5");
        // 2000 input + 500 cache_read + 50 cache_5m + 0 cache_1h
        assert_eq!(info.context_tokens, 2550);
    }

    #[test]
    fn latest_event_for_file_ignores_subagent_files() {
        let (_dir, db) = fresh_db();
        let parent = "-Users-me-proj/sess1.jsonl";
        db.ingest_atomic(parent, &[mk_event(parent, 0, 100, 0.1)], &[], 1, 100).unwrap();
        let sub = "-Users-me-proj/sess1/subagents/agent-a.jsonl";
        db.ingest_atomic(sub, &[mk_event(sub, 99, 50000, 5.0)], &[], 1, 100).unwrap();
        // Querying the PARENT file must not pick up the subagent's huge event.
        let info = db.latest_event_for_file(parent).unwrap().expect("row present");
        assert_eq!(info.context_tokens, 100 + 100 + 10); // input+cache_read+cache_5m from mk_event
    }

    #[test]
    fn latest_event_for_file_none_when_no_rows() {
        let (_dir, db) = fresh_db();
        assert!(db.latest_event_for_file("-Users-me-proj/never.jsonl").unwrap().is_none());
    }

    #[test]
    fn record_window_peak_tracks_the_max_within_one_window() {
        let (_dir, db) = fresh_db();
        let resets_at = Utc::now() + chrono::Duration::hours(3);
        let t1 = Utc::now();
        let t2 = t1 + chrono::Duration::minutes(5);
        let t3 = t1 + chrono::Duration::minutes(10);

        db.record_window_peak("acc1", "five_hour", resets_at, t1, 40.0).unwrap();
        db.record_window_peak("acc1", "five_hour", resets_at, t2, 82.0).unwrap();
        // A later, lower reading must not overwrite the peak.
        db.record_window_peak("acc1", "five_hour", resets_at, t3, 60.0).unwrap();

        let peaks = db
            .window_peaks_between("acc1", t1 - chrono::Duration::minutes(1), t3 + chrono::Duration::minutes(1))
            .unwrap();
        assert_eq!(peaks.len(), 1);
        assert_eq!(peaks[0].peak_pct, 82.0);
        assert_eq!(peaks[0].peak_at.timestamp(), t2.timestamp());
        assert_eq!(peaks[0].window_start.timestamp(), t1.timestamp());
    }

    #[test]
    fn record_window_peak_starts_a_new_row_when_resets_at_changes() {
        let (_dir, db) = fresh_db();
        let window1_resets = Utc::now() + chrono::Duration::hours(1);
        let window2_resets = Utc::now() + chrono::Duration::hours(6);
        let now = Utc::now();

        db.record_window_peak("acc1", "five_hour", window1_resets, now, 95.0).unwrap();
        db.record_window_peak("acc1", "five_hour", window2_resets, now, 10.0).unwrap();

        let peaks = db
            .window_peaks_between("acc1", now - chrono::Duration::minutes(1), now + chrono::Duration::minutes(1))
            .unwrap();
        assert_eq!(peaks.len(), 2, "a new resets_at must create a new row, not overwrite the old one");
    }

    #[test]
    fn window_peaks_between_scopes_to_the_requested_account() {
        let (_dir, db) = fresh_db();
        db.upsert_account(&StoredAccount { id: "acc2".into(), email: "b@example.com".into(), display_name: None }).unwrap();
        let resets_at = Utc::now() + chrono::Duration::hours(1);
        let now = Utc::now();

        db.record_window_peak("acc1", "five_hour", resets_at, now, 91.0).unwrap();
        db.record_window_peak("acc2", "five_hour", resets_at, now, 91.0).unwrap();

        let peaks = db
            .window_peaks_between("acc1", now - chrono::Duration::minutes(1), now + chrono::Duration::minutes(1))
            .unwrap();
        assert_eq!(peaks.len(), 1);
        assert_eq!(peaks[0].bucket, "five_hour");
    }

    #[test]
    fn limit_hit_stats_counts_hits_and_attributes_projects() {
        let (_dir, db) = fresh_db();
        let resets_at = Utc::now() + chrono::Duration::hours(2);
        let window_start = Utc::now() - chrono::Duration::hours(3);
        let peak_at = window_start + chrono::Duration::hours(1);

        // A hit (>= 90 threshold) and a miss (< 90) — only the hit should count.
        // Record window start first (with low reading), then peak; this establishes window_start in DB.
        db.record_window_peak("acc1", "five_hour", resets_at, window_start, 0.0).unwrap();
        db.record_window_peak("acc1", "five_hour", resets_at, peak_at, 95.0).unwrap();
        db.record_window_peak("acc1", "seven_day", resets_at, window_start, 0.0).unwrap();
        db.record_window_peak("acc1", "seven_day", resets_at, peak_at, 50.0).unwrap();

        // One event inside the hit window, one outside it — only the inside one attributes.
        db.insert_events(&[StoredSessionEvent {
            ts: window_start + chrono::Duration::minutes(30),
            project: "switchboard".into(),
            model: "claude-sonnet-4-6".into(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 1.23,
            source_file: "/a.jsonl".into(),
            source_line: 1,
            event_id: "evt-in".into(),
            account_uuid: None,
        }])
        .unwrap();
        db.insert_events(&[StoredSessionEvent {
            ts: peak_at + chrono::Duration::hours(5),
            project: "other-project".into(),
            model: "claude-sonnet-4-6".into(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 9.99,
            source_file: "/b.jsonl".into(),
            source_line: 1,
            event_id: "evt-out".into(),
            account_uuid: None,
        }])
        .unwrap();

        let stats = db
            .limit_hit_stats("acc1", "a@example.com", Utc::now() - chrono::Duration::hours(4), Utc::now() + chrono::Duration::hours(4), 90.0)
            .unwrap();

        assert_eq!(stats.five_hour_hits, 1);
        assert_eq!(stats.seven_day_hits, 0);
        assert_eq!(stats.hourly_distribution.len(), 24);
        assert_eq!(stats.hourly_distribution.iter().sum::<u32>(), 1);
        assert_eq!(stats.top_projects.len(), 1);
        assert_eq!(stats.top_projects[0].project, "switchboard");
        assert_eq!(stats.top_projects[0].cost_usd, 1.23);
    }

    #[test]
    fn limit_hit_stats_returns_zeroed_struct_for_an_account_with_no_history() {
        let (_dir, db) = fresh_db();
        let stats = db
            .limit_hit_stats("acc1", "a@example.com", Utc::now() - chrono::Duration::days(30), Utc::now(), 90.0)
            .unwrap();
        assert_eq!(stats.five_hour_hits, 0);
        assert_eq!(stats.seven_day_hits, 0);
        assert!(stats.top_projects.is_empty());
    }

    #[test]
    fn limit_hit_stats_sums_project_cost_across_multiple_hit_windows() {
        let (_dir, db) = fresh_db();
        // Two separate five_hour windows (different resets_at times).
        let resets_at_1 = Utc::now() + chrono::Duration::hours(2);
        let window_start_1 = Utc::now() - chrono::Duration::hours(6);
        let peak_at_1 = window_start_1 + chrono::Duration::hours(1);

        let resets_at_2 = Utc::now() + chrono::Duration::hours(7);  // Different reset time
        let window_start_2 = Utc::now() - chrono::Duration::hours(1);
        let peak_at_2 = window_start_2 + chrono::Duration::hours(1);

        // First window: record a hit.
        db.record_window_peak("acc1", "five_hour", resets_at_1, window_start_1, 0.0).unwrap();
        db.record_window_peak("acc1", "five_hour", resets_at_1, peak_at_1, 95.0).unwrap();

        // Second window: record another hit.
        db.record_window_peak("acc1", "five_hour", resets_at_2, window_start_2, 0.0).unwrap();
        db.record_window_peak("acc1", "five_hour", resets_at_2, peak_at_2, 92.0).unwrap();

        // Insert same project in both windows — costs should accumulate.
        db.insert_events(&[StoredSessionEvent {
            ts: window_start_1 + chrono::Duration::minutes(30),
            project: "my-app".into(),
            model: "claude-sonnet-4-6".into(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 2.50,
            source_file: "/a.jsonl".into(),
            source_line: 1,
            event_id: "evt-window1".into(),
            account_uuid: None,
        }])
        .unwrap();

        db.insert_events(&[StoredSessionEvent {
            ts: window_start_2 + chrono::Duration::minutes(30),
            project: "my-app".into(),
            model: "claude-sonnet-4-6".into(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 1.75,
            source_file: "/b.jsonl".into(),
            source_line: 1,
            event_id: "evt-window2".into(),
            account_uuid: None,
        }])
        .unwrap();

        let stats = db
            .limit_hit_stats("acc1", "a@example.com", Utc::now() - chrono::Duration::hours(8), Utc::now() + chrono::Duration::hours(8), 90.0)
            .unwrap();

        assert_eq!(stats.five_hour_hits, 2, "should find both hit windows");
        assert_eq!(stats.top_projects.len(), 1, "should have one project (accumulated across windows)");
        assert_eq!(stats.top_projects[0].project, "my-app");
        assert!((stats.top_projects[0].cost_usd - 4.25).abs() < 1e-6, "costs should sum: 2.50 + 1.75 = 4.25");
    }

    /// Regression for the double-counting bug: a five_hour hit window's span
    /// is fully nested inside an enclosing seven_day hit window's span (5h
    /// always falls within 7d). An event sitting in that overlap must be
    /// counted once in top_projects, not once per enclosing hit window.
    #[test]
    fn limit_hit_stats_does_not_double_count_overlapping_windows() {
        let (_dir, db) = fresh_db();

        // seven_day window: wide span that encloses the five_hour window below.
        let seven_day_resets = Utc::now() + chrono::Duration::days(2);
        let seven_day_start = Utc::now() - chrono::Duration::days(5);
        let seven_day_peak_at = Utc::now() - chrono::Duration::hours(1);

        // five_hour window: narrow span, fully inside [seven_day_start, seven_day_peak_at].
        let five_hour_resets = Utc::now() + chrono::Duration::hours(2);
        let five_hour_start = Utc::now() - chrono::Duration::hours(3);
        let five_hour_peak_at = Utc::now() - chrono::Duration::hours(2);

        db.record_window_peak("acc1", "seven_day", seven_day_resets, seven_day_start, 0.0).unwrap();
        db.record_window_peak("acc1", "seven_day", seven_day_resets, seven_day_peak_at, 95.0).unwrap();
        db.record_window_peak("acc1", "five_hour", five_hour_resets, five_hour_start, 0.0).unwrap();
        db.record_window_peak("acc1", "five_hour", five_hour_resets, five_hour_peak_at, 92.0).unwrap();

        // One event, squarely inside BOTH windows' [window_start, peak_at] spans.
        db.insert_events(&[StoredSessionEvent {
            ts: five_hour_start + chrono::Duration::minutes(30),
            project: "shared-proj".into(),
            model: "claude-sonnet-4-6".into(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 5.0,
            source_file: "/shared.jsonl".into(),
            source_line: 1,
            event_id: "evt-shared".into(),
            account_uuid: None,
        }])
        .unwrap();

        let stats = db
            .limit_hit_stats(
                "acc1",
                "a@example.com",
                Utc::now() - chrono::Duration::days(6),
                Utc::now() + chrono::Duration::days(3),
                90.0,
            )
            .unwrap();

        assert_eq!(stats.five_hour_hits, 1);
        assert_eq!(stats.seven_day_hits, 1);
        assert_eq!(stats.top_projects.len(), 1);
        assert_eq!(stats.top_projects[0].project, "shared-proj");
        assert!(
            (stats.top_projects[0].cost_usd - 5.0).abs() < 1e-6,
            "event inside the overlap must be counted once, not once per enclosing hit window (got {})",
            stats.top_projects[0].cost_usd
        );
    }

    #[test]
    fn record_account_transition_opens_first_interval() {
        let (_dir, db) = fresh_db();
        let t0 = Utc::now();
        db.record_account_transition(Some("acc1"), t0).unwrap();

        let conn = db.conn();
        let (account_uuid, ended_at): (String, Option<i64>) = conn
            .query_row(
                "SELECT account_uuid, ended_at FROM account_intervals",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(account_uuid, "acc1");
        assert_eq!(ended_at, None);
    }

    #[test]
    fn record_account_transition_closes_then_opens_on_change() {
        let (_dir, db) = fresh_db();
        db.upsert_account(&StoredAccount { id: "acc2".into(), email: "b@example.com".into(), display_name: None }).unwrap();
        let t0 = Utc::now();
        let t1 = t0 + chrono::Duration::minutes(5);

        db.record_account_transition(Some("acc1"), t0).unwrap();
        db.record_account_transition(Some("acc2"), t1).unwrap();

        let conn = db.conn();
        let mut stmt = conn
            .prepare("SELECT account_uuid, started_at, ended_at FROM account_intervals ORDER BY started_at")
            .unwrap();
        let rows: Vec<(String, i64, Option<i64>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], ("acc1".into(), t0.timestamp(), Some(t1.timestamp())));
        assert_eq!(rows[1].0, "acc2");
        assert_eq!(rows[1].2, None);
    }

    #[test]
    fn record_account_transition_is_a_noop_when_account_unchanged() {
        let (_dir, db) = fresh_db();
        let t0 = Utc::now();
        let t1 = t0 + chrono::Duration::minutes(1);

        db.record_account_transition(Some("acc1"), t0).unwrap();
        db.record_account_transition(Some("acc1"), t1).unwrap();

        let conn = db.conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM account_intervals", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "re-observing the same active account must not create a second interval");
    }

    #[test]
    fn record_account_transition_closes_interval_on_none() {
        let (_dir, db) = fresh_db();
        let t0 = Utc::now();
        let t1 = t0 + chrono::Duration::minutes(1);

        db.record_account_transition(Some("acc1"), t0).unwrap();
        db.record_account_transition(None, t1).unwrap();

        let conn = db.conn();
        let open_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM account_intervals WHERE ended_at IS NULL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(open_count, 0, "no managed account live must leave no open interval");
    }

    #[test]
    fn events_between_attributes_account_via_interval_join() {
        let (_dir, db) = fresh_db();
        db.upsert_account(&StoredAccount { id: "acc2".into(), email: "b@example.com".into(), display_name: None }).unwrap();

        let t_before = Utc::now() - chrono::Duration::hours(3);
        let t_acc1 = Utc::now() - chrono::Duration::hours(2);
        let t_acc2 = Utc::now() - chrono::Duration::hours(1);

        // acc1 active from t_acc1 to t_acc2, then acc2 active from t_acc2 onward.
        db.record_account_transition(Some("acc1"), t_acc1).unwrap();
        db.record_account_transition(Some("acc2"), t_acc2).unwrap();

        let mk = |ts: chrono::DateTime<Utc>, id: &str| StoredSessionEvent {
            ts,
            project: "p".into(),
            model: "m".into(),
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 0.0,
            source_file: "a.jsonl".into(),
            source_line: 0,
            event_id: id.into(),
            account_uuid: None,
        };
        db.insert_events(&[
            mk(t_before, "before"), // predates any interval
            mk(t_acc1 + chrono::Duration::minutes(1), "during-acc1"),
            mk(t_acc2 + chrono::Duration::minutes(1), "during-acc2"),
        ])
        .unwrap();

        let events = db
            .events_between(t_before - chrono::Duration::minutes(1), Utc::now())
            .unwrap();

        let by_id: std::collections::HashMap<&str, &Option<String>> = events
            .iter()
            .map(|e| (e.event_id.as_str(), &e.account_uuid))
            .collect();

        assert_eq!(by_id.get("before"), Some(&&None), "event before any interval must be unattributed");
        assert_eq!(by_id.get("during-acc1"), Some(&&Some("acc1".to_string())));
        assert_eq!(by_id.get("during-acc2"), Some(&&Some("acc2".to_string())));
    }

    /// Pins the half-open `[started_at, ended_at)` contract at the exact
    /// boundary instants, which is where an off-by-one in the attribution
    /// subquery would show up: an event at a boundary must belong to the
    /// interval that *starts* there, never to the one that just ended.
    #[test]
    fn events_between_attributes_boundary_timestamps_half_open() {
        let (_dir, db) = fresh_db();
        db.upsert_account(&StoredAccount { id: "acc2".into(), email: "b@example.com".into(), display_name: None }).unwrap();

        let t_acc1 = Utc::now() - chrono::Duration::hours(2);
        let t_acc2 = Utc::now() - chrono::Duration::hours(1);

        // acc1 spans [t_acc1, t_acc2); acc2 spans [t_acc2, ∞).
        db.record_account_transition(Some("acc1"), t_acc1).unwrap();
        db.record_account_transition(Some("acc2"), t_acc2).unwrap();

        let mk = |ts: chrono::DateTime<Utc>, id: &str| StoredSessionEvent {
            ts,
            project: "p".into(),
            model: "m".into(),
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 0.0,
            source_file: "a.jsonl".into(),
            source_line: 0,
            event_id: id.into(),
            account_uuid: None,
        };
        db.insert_events(&[
            // Exactly at acc1's started_at.
            mk(t_acc1, "at-start-acc1"),
            // Exactly at acc1's ended_at, which is also acc2's started_at.
            mk(t_acc2, "at-boundary"),
        ])
        .unwrap();

        let events = db
            .events_between(t_acc1 - chrono::Duration::minutes(1), Utc::now())
            .unwrap();
        let by_id: std::collections::HashMap<&str, &Option<String>> = events
            .iter()
            .map(|e| (e.event_id.as_str(), &e.account_uuid))
            .collect();

        assert_eq!(
            by_id.get("at-start-acc1"),
            Some(&&Some("acc1".to_string())),
            "an event exactly at started_at belongs to that interval (inclusive lower bound)"
        );
        assert_eq!(
            by_id.get("at-boundary"),
            Some(&&Some("acc2".to_string())),
            "an event exactly at ended_at belongs to the NEXT interval, not the one that just closed (exclusive upper bound)"
        );
        // And the boundary event must appear exactly once — a join form would
        // emit one row per matching interval if intervals ever overlapped.
        assert_eq!(
            events.iter().filter(|e| e.event_id == "at-boundary").count(),
            1,
            "each event must yield exactly one row regardless of interval layout"
        );
    }

    #[test]
    fn account_uuids_by_source_file_folds_subagents_onto_parent_and_dedupes() {
        let (_dir, db) = fresh_db();
        db.upsert_account(&StoredAccount { id: "acc2".into(), email: "b@example.com".into(), display_name: None }).unwrap();

        let t1 = Utc::now() - chrono::Duration::hours(2);
        let t2 = Utc::now() - chrono::Duration::hours(1);
        db.record_account_transition(Some("acc1"), t1).unwrap();
        db.record_account_transition(Some("acc2"), t2).unwrap();

        let mk = |ts: chrono::DateTime<Utc>, src: &str, id: &str| StoredSessionEvent {
            ts,
            project: "p".into(),
            model: "m".into(),
            input_tokens: 1,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 0.0,
            source_file: src.into(),
            source_line: 0,
            event_id: id.into(),
            account_uuid: None,
        };
        db.insert_events(&[
            mk(t1 + chrono::Duration::minutes(1), "proj/abc.jsonl", "e1"),
            mk(t1 + chrono::Duration::minutes(2), "proj/abc/subagents/agent-x.jsonl", "e2"),
            mk(t2 + chrono::Duration::minutes(1), "proj/abc.jsonl", "e3"), // same conversation, resumed under acc2
            mk(t2 + chrono::Duration::minutes(2), "proj/other.jsonl", "e4"),
        ])
        .unwrap();

        let map = db.account_uuids_by_source_file().unwrap();

        let mut abc = map.get("proj/abc.jsonl").cloned().unwrap_or_default();
        abc.sort();
        assert_eq!(
            abc,
            vec![Some("acc1".to_string()), Some("acc2".to_string())],
            "subagent folds onto parent; both accounts recorded, no duplicates"
        );
        assert_eq!(map.get("proj/other.jsonl"), Some(&vec![Some("acc2".to_string())]));
        assert!(!map.contains_key("proj/abc/subagents/agent-x.jsonl"));
    }

    /// Events no interval covers must survive as a `None` entry rather than
    /// being filtered out — the Repo tab renders that as an "Unknown" badge,
    /// and dropping it made a mostly-unattributed repo look 100% attributed
    /// to whichever account happened to touch it once.
    #[test]
    fn account_uuids_by_source_file_keeps_unattributed_events_as_none() {
        let (_dir, db) = fresh_db();

        let t_pre = Utc::now() - chrono::Duration::hours(3);
        let t_acc1 = Utc::now() - chrono::Duration::hours(1);
        db.record_account_transition(Some("acc1"), t_acc1).unwrap();

        let mk = |ts: chrono::DateTime<Utc>, src: &str, id: &str| StoredSessionEvent {
            ts,
            project: "p".into(),
            model: "m".into(),
            input_tokens: 1,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 0.0,
            source_file: src.into(),
            source_line: 0,
            event_id: id.into(),
            account_uuid: None,
        };
        db.insert_events(&[
            // Entirely pre-feature conversation: nothing attributable at all.
            mk(t_pre, "proj/old.jsonl", "e1"),
            // Conversation that straddles the start of interval tracking.
            mk(t_pre + chrono::Duration::minutes(1), "proj/mixed.jsonl", "e2"),
            mk(t_acc1 + chrono::Duration::minutes(1), "proj/mixed.jsonl", "e3"),
        ])
        .unwrap();

        let map = db.account_uuids_by_source_file().unwrap();

        assert_eq!(
            map.get("proj/old.jsonl"),
            Some(&vec![None]),
            "a conversation with no attributable events must still appear, as Unknown"
        );

        let mut mixed = map.get("proj/mixed.jsonl").cloned().unwrap_or_default();
        mixed.sort();
        assert_eq!(
            mixed,
            vec![None, Some("acc1".to_string())],
            "a partly-attributed conversation keeps both the account and the Unknown bucket"
        );
    }

    /// A caller that reaches the DB *after* another caller but with an
    /// *earlier* timestamp (both capture `at` before taking the lock) must not
    /// be able to open an interval that starts before the one it closes.
    #[test]
    fn record_account_transition_clamps_out_of_order_timestamps() {
        let (_dir, db) = fresh_db();
        db.upsert_account(&StoredAccount { id: "acc2".into(), email: "b@example.com".into(), display_name: None }).unwrap();

        let t_late = Utc::now();
        let t_early = t_late - chrono::Duration::minutes(5);

        db.record_account_transition(Some("acc1"), t_late).unwrap();
        // Racing caller arrives second but carries the earlier timestamp.
        db.record_account_transition(Some("acc2"), t_early).unwrap();

        let conn = db.conn();
        let mut stmt = conn
            .prepare("SELECT account_uuid, started_at, ended_at FROM account_intervals ORDER BY id")
            .unwrap();
        let rows: Vec<(String, i64, Option<i64>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "acc1");
        assert_eq!(
            rows[0].2,
            Some(t_late.timestamp()),
            "the earlier timestamp is clamped forward, so acc1's interval never ends before it began"
        );
        assert_eq!(rows[1].0, "acc2");
        assert_eq!(
            rows[1].1,
            t_late.timestamp(),
            "acc2's interval starts where acc1's ended — no overlap"
        );
        assert!(rows[1].1 >= rows[0].1, "intervals stay monotonically ordered by started_at");
    }
}
