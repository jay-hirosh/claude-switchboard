//! Read-only browser over Claude Code transcripts: what each session was
//! about, and enough identity to resume it on the right provider.
//!
//! Deliberately divergent from `jsonl_parser`: ingestion *wants* subagent
//! transcripts (their API calls are real spend), while the browser must never
//! list them (they are not resumable sessions).

pub mod scan;
