//! Launches provider-scoped Claude Code sessions with per-process env.
//!
//! Nothing global is mutated, so several providers can run concurrently and
//! the user's own launch scripts keep working.

pub mod script;
