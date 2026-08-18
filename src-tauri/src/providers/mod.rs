//! Custom model providers: persistence, presets, per-process launching, and
//! the opt-in `~/.claude/settings.json` default.
//!
//! Claude Code reads `settings.json` `env` exactly once at startup and
//! `Object.assign`s it over the inherited shell environment, so a running
//! session can never adopt a provider change and a global write silently
//! overrides the user's own launch scripts. Launching with per-process env
//! is therefore the default path; `default_env` is opt-in and guarded.

pub mod default_env;
pub mod launcher;
pub mod model;
pub mod ollama;
pub mod presets;
pub mod store;

pub use model::{Provider, ProviderKind};
pub use store::DefaultProviderState;
