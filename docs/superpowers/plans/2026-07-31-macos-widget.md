# macOS Desktop Widget Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a macOS 14+ desktop widget (WidgetKit, `.systemSmall`) showing the active account's 5-hour usage ring, sourced from a JSON snapshot the existing Tauri/Rust app writes to a shared App Group container.

**Architecture:** Rust's poll loop writes a small JSON snapshot to `~/Library/Group Containers/<TeamID>.com.claude-switchboard.app/snapshot.json` after every successful poll of the active account. A new Swift WidgetKit extension (built separately via Xcode, embedded into the Tauri-built `.app` by a local script) reads that file in its `TimelineProvider` and renders it. Tapping the widget opens the main app via a `claude-switchboard://open` custom URL scheme handled by `tauri-plugin-deep-link`.

**Tech Stack:** Rust (existing `claude-switchboard` crate), Swift + WidgetKit + SwiftUI (new, `native/macos/`), `tauri-plugin-deep-link`, `codesign`/`xcodebuild` (local build only).

## Global Constraints

- macOS 14+ only. No Windows work in this plan.
- Local builds only — no changes to `.github/workflows/release.yml` or public CI signing. The unsigned public release pipeline is untouched.
- Requires a paid Apple Developer Program membership for the App Group entitlement and code signing. There is no ad-hoc-signed path that works for WidgetKit.
- Single widget size: `.systemSmall`. No medium/large layouts.
- Tap-to-open only. No in-widget interactive buttons / App Intents.
- No push-based refresh (no Darwin notifications). The widget relies on WidgetKit's own timeline reload schedule and always shows an "as of Xm ago" freshness label rather than presenting a stale number as live.
- Rust is the single source of truth for usage thresholds and color bands (mirrors `tray_icon::shared::arc_color`'s 75/90 thresholds). Swift never re-derives thresholds — it only renders whatever `colorBand` string Rust already computed.
- The App Group identifier is `<TEAM_ID>.com.claude-switchboard.app`. `<TEAM_ID>` is a placeholder for your Apple Developer Team ID (found on the Membership Details page at developer.apple.com) — it must be substituted identically in three places, listed in Task 5, Step 1.

---

### Task 1: Rust `widget_snapshot` module

**Files:**
- Create: `src-tauri/src/widget_snapshot.rs`
- Modify: `src-tauri/src/lib.rs` (register the module)

**Interfaces:**
- Consumes: `crate::auth::accounts::ManagedAccount` (`email: String`, `subscription_type: Option<String>`), `crate::usage_api::UsageSnapshot` (`five_hour: Option<Utilization>`), `crate::usage_api::Utilization` (`utilization: f64`, `resets_at: Option<DateTime<Utc>>`).
- Produces: `pub struct WidgetSnapshot`, `pub fn color_band(pct: f64) -> &'static str`, `pub fn build(acc: &ManagedAccount, snapshot: &UsageSnapshot, poll_interval_seconds: u64, now: DateTime<Utc>) -> WidgetSnapshot`, `pub fn container_dir() -> PathBuf`, `pub fn write(dir: &Path, snapshot: &WidgetSnapshot) -> anyhow::Result<()>` — all consumed by Task 2.

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/widget_snapshot.rs` with just the test module (implementation is `todo!()` so it fails to compile/panic, proving the tests exercise real code once filled in):

```rust
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
    todo!()
}

pub fn build(
    acc: &ManagedAccount,
    snapshot: &UsageSnapshot,
    poll_interval_seconds: u64,
    now: DateTime<Utc>,
) -> WidgetSnapshot {
    todo!()
}

/// Resolves the App Group shared container path:
/// `~/Library/Group Containers/<APP_GROUP_ID>/`.
pub fn container_dir() -> PathBuf {
    todo!()
}

/// Atomically writes the snapshot as `snapshot.json` in `dir`, creating
/// `dir` if needed. Writes to a temp file then renames, so the widget
/// extension never observes a partially written file.
pub fn write(dir: &Path, snapshot: &WidgetSnapshot) -> anyhow::Result<()> {
    todo!()
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
```

Register the module in `src-tauri/src/lib.rs` — add as the last line of the `mod`/`pub mod` block (after `pub mod warmup;`):

```rust
#[cfg(target_os = "macos")]
mod widget_snapshot;
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test widget_snapshot`
Expected: compile failure (`todo!()` isn't reachable at type-check time for `container_dir`/`write`'s bodies, but `color_band`/`build` will panic with "not yet implemented" once called) — confirms the tests actually exercise the new functions.

- [ ] **Step 3: Implement**

Replace each `todo!()` body:

```rust
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

pub fn container_dir() -> PathBuf {
    let home = directories::BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("Library/Group Containers").join(APP_GROUP_ID)
}

pub fn write(dir: &Path, snapshot: &WidgetSnapshot) -> anyhow::Result<()> {
    fs::create_dir_all(dir)?;
    let tmp_path = dir.join("snapshot.json.tmp");
    let final_path = dir.join("snapshot.json");
    fs::write(&tmp_path, serde_json::to_vec_pretty(snapshot)?)?;
    fs::rename(&tmp_path, &final_path)?;
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test widget_snapshot`
Expected: all 6 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/widget_snapshot.rs src-tauri/src/lib.rs
git commit -m "feat(widget): add widget_snapshot module for macOS App Group data sharing"
```

---

### Task 2: Wire snapshot writes into the poll loop

**Files:**
- Modify: `src-tauri/src/poll_loop.rs:266-276` (the `if Some(slot) == active_slot` block inside `apply_fetch_outcome`)

**Interfaces:**
- Consumes: `crate::widget_snapshot::{build, write, container_dir}` from Task 1; `state.settings.read().polling_interval_secs` (existing field, see `settings_durations` a few lines above in the same file).
- Produces: nothing new consumed by later tasks — this task's only effect is that `snapshot.json` gets written on every successful active-account poll.

- [ ] **Step 1: Read the current active-slot success branch**

Confirm the exact current text at `src-tauri/src/poll_loop.rs:266-276`:

```rust
            if Some(slot) == active_slot {
                *state.cached_usage.write() = Some(cached.clone());
                tray::set_level(
                    handle,
                    snapshot.five_hour.as_ref().map(|u| u.utilization),
                    snapshot.seven_day.as_ref().map(|u| u.utilization),
                    snapshot.five_hour.as_ref().and_then(|u| u.resets_at),
                    snapshot.seven_day.as_ref().and_then(|u| u.resets_at),
                    false,
                );
```

- [ ] **Step 2: Add the widget snapshot write immediately after `tray::set_level(...)`**

```rust
            if Some(slot) == active_slot {
                *state.cached_usage.write() = Some(cached.clone());
                tray::set_level(
                    handle,
                    snapshot.five_hour.as_ref().map(|u| u.utilization),
                    snapshot.seven_day.as_ref().map(|u| u.utilization),
                    snapshot.five_hour.as_ref().and_then(|u| u.resets_at),
                    snapshot.seven_day.as_ref().and_then(|u| u.resets_at),
                    false,
                );
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
```

This only runs in the `FetchOutcome::Ok` branch for the active slot — a failed poll (`Unauthorized`/`RateLimited`/`Transient`, handled in the other match arms) never reaches this code, so the widget snapshot naturally ages instead of being overwritten with bad data, matching the spec's error-handling table.

- [ ] **Step 3: Run the full existing test suite to confirm no regressions**

Run: `cd src-tauri && cargo test`
Expected: all existing tests still PASS (this change adds no new branching logic of its own — the logic it calls is already covered by Task 1's tests — so no new test is added here, only a regression check).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/poll_loop.rs
git commit -m "feat(widget): write widget snapshot after each active-account poll"
```

---

### Task 3: Custom URL scheme + deep-link handling in the main app

**Files:**
- Modify: `src-tauri/Cargo.toml` (add dependency)
- Modify: `src-tauri/tauri.conf.json` (register scheme)
- Modify: `src-tauri/src/lib.rs` (plugin init + open-url handler)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: the running app responds to `claude-switchboard://open` by showing the popover — this is what the widget's `widgetURL` (Task 4) targets.

- [ ] **Step 1: Add the deep-link plugin dependency**

In `src-tauri/Cargo.toml`, add alongside the other `tauri-plugin-*` entries (after `tauri-plugin-dialog`):

```toml
tauri-plugin-deep-link = "2"
```

- [ ] **Step 2: Register the scheme in `tauri.conf.json`**

In `src-tauri/tauri.conf.json`, add a `deep-link` entry to the existing `plugins` object (alongside `updater`):

```json
  "plugins": {
    "deep-link": {
      "desktop": {
        "schemes": ["claude-switchboard"]
      }
    },
    "updater": {
```

- [ ] **Step 3: Initialize the plugin**

In `src-tauri/src/lib.rs`, add to the plugin chain (after `.plugin(tauri_plugin_dialog::init())`):

```rust
        .plugin(tauri_plugin_deep_link::init())
```

- [ ] **Step 4: Handle the open-url event**

In the same `.setup(|app| { ... })` closure, after the `if let Some(tray) = app.tray_by_id("main") { ... } else { ... }` block closes (i.e. right before the "First-run UX" comment), add:

```rust
            // Widget tap-to-open: `claude-switchboard://open` (any path)
            // brings the popover forward, same as a tray-icon click.
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let app_handle = app.handle().clone();
                app.deep_link().on_open_url(move |_event| {
                    if let Some(w) = app_handle.get_webview_window("popover") {
                        crate::move_to_tray_center(&w);
                        let _ = w.show();
                        let _ = w.set_focus();
                        use tauri::Emitter;
                        let _ = app_handle.emit("popover_shown", ());
                    }
                });
            }
```

No changes to `src-tauri/capabilities/default.json` are needed — the frontend never calls a deep-link command directly, so no new permission grant is required.

- [ ] **Step 5: Build and manually verify**

Run: `pnpm tauri dev` (from repo root, wait for the app to launch), then in a separate terminal:

```bash
open "claude-switchboard://open"
```

Expected: the popover window comes forward and gains focus, identical to clicking the tray icon. (This is an OS-level URL-dispatch integration — there is no meaningful Rust unit test for "the OS routed this URL to us"; the `open` command above is the real verification.)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/tauri.conf.json src-tauri/src/lib.rs
git commit -m "feat(widget): handle claude-switchboard:// deep link to open the popover"
```

---

### Task 4: WidgetKit extension (Swift)

**Files:**
- Create: `native/macos/SwitchboardWidget/SwitchboardWidget.xcodeproj/` (Xcode-generated project)
- Create: `native/macos/SwitchboardWidget/SwitchboardWidgetExtension/SwitchboardWidgetSnapshot.swift`
- Create: `native/macos/SwitchboardWidget/SwitchboardWidgetExtension/SwitchboardWidget.swift`
- Create: `native/macos/SwitchboardWidget/SwitchboardWidgetExtension/SwitchboardWidgetBundle.swift`
- Create: `native/macos/SwitchboardWidget/SwitchboardWidgetExtension/Info.plist` (Xcode-generated, default contents are fine)
- Create: `native/macos/SwitchboardWidget/SwitchboardWidgetExtension/SwitchboardWidgetExtension.entitlements`

**Interfaces:**
- Consumes: the JSON file written by Task 1/2 at `~/Library/Group Containers/<TEAM_ID>.com.claude-switchboard.app/snapshot.json`, with keys `accountLabel`, `tier`, `fiveHourPct`, `fiveHourResetAt`, `colorBand`, `pollIntervalSeconds`, `writtenAt` (camelCase, ISO-8601 dates) — exactly what `WidgetSnapshot`'s `#[serde(rename_all = "camelCase")]` produces.
- Produces: the compiled `SwitchboardWidgetExtension.appex`, embedded by Task 5's script. The `claude-switchboard://open` URL from Task 3 is the tap target.

This task is Xcode-GUI-driven for project scaffolding (hand-authoring a `.xcodeproj`'s `project.pbxproj` is impractical and error-prone) — the actual logic lives in the Swift source below, which is fully specified.

- [ ] **Step 1: Create the Xcode project**

1. Open Xcode → File → New → Project → macOS → Widget Extension.
2. Product Name: `SwitchboardWidget`. Team: your Apple Developer team. Bundle Identifier: `com.claude-switchboard.app.widget`.
3. Uncheck "Include Configuration Intent" (this widget has no user-configurable options).
4. Save at `native/macos/SwitchboardWidget/` in the repo.
5. In the target's "Signing & Capabilities" tab, click "+ Capability" → "App Groups" → add `<TEAM_ID>.com.claude-switchboard.app` (same placeholder as Task 1 — substitute your real Team ID). This creates `SwitchboardWidgetExtension.entitlements` automatically; verify it matches:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.application-groups</key>
    <array>
        <string>&lt;TEAM_ID&gt;.com.claude-switchboard.app</string>
    </array>
</dict>
</plist>
```

(with `<TEAM_ID>` replaced by your real Team ID, not the literal placeholder text).

- [ ] **Step 2: Write the snapshot model**

Create `SwitchboardWidgetSnapshot.swift`:

```swift
import Foundation

struct SwitchboardWidgetSnapshot: Decodable {
    let accountLabel: String
    let tier: String
    let fiveHourPct: Double
    let fiveHourResetAt: Date?
    let colorBand: String
    let pollIntervalSeconds: Int
    let writtenAt: Date

    /// Must match `APP_GROUP_ID` in src-tauri/src/widget_snapshot.rs exactly.
    static let appGroupID = "<TEAM_ID>.com.claude-switchboard.app"

    static func load() -> SwitchboardWidgetSnapshot? {
        guard let containerURL = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: appGroupID
        ) else { return nil }
        let fileURL = containerURL.appendingPathComponent("snapshot.json")
        guard let data = try? Data(contentsOf: fileURL) else { return nil }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .custom(decodeFlexibleISO8601)
        return try? decoder.decode(SwitchboardWidgetSnapshot.self, from: data)
    }

    // Rust's chrono serializes `DateTime<Utc>` as RFC 3339 with fractional
    // seconds whenever they're nonzero (e.g. "...T15:32:00.123456789Z"),
    // but Foundation's plain `.iso8601` decoding strategy rejects any
    // fractional component. Try both formats rather than coupling to
    // chrono's exact output.
    private static func decodeFlexibleISO8601(_ decoder: Decoder) throws -> Date {
        let container = try decoder.singleValueContainer()
        let string = try container.decode(String.self)
        let withFraction = ISO8601DateFormatter()
        withFraction.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = withFraction.date(from: string) { return date }
        let withoutFraction = ISO8601DateFormatter()
        withoutFraction.formatOptions = [.withInternetDateTime]
        if let date = withoutFraction.date(from: string) { return date }
        throw DecodingError.dataCorruptedError(
            in: container, debugDescription: "Invalid ISO-8601 date: \(string)"
        )
    }
}
```

- [ ] **Step 3: Write the timeline provider, entry view, and widget**

Create `SwitchboardWidget.swift`:

```swift
import WidgetKit
import SwiftUI

struct SwitchboardEntry: TimelineEntry {
    let date: Date
    let snapshot: SwitchboardWidgetSnapshot?
}

struct SwitchboardProvider: TimelineProvider {
    func placeholder(in context: Context) -> SwitchboardEntry {
        SwitchboardEntry(date: Date(), snapshot: nil)
    }

    func getSnapshot(in context: Context, completion: @escaping (SwitchboardEntry) -> Void) {
        completion(SwitchboardEntry(date: Date(), snapshot: SwitchboardWidgetSnapshot.load()))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<SwitchboardEntry>) -> Void) {
        let snapshot = SwitchboardWidgetSnapshot.load()
        let entry = SwitchboardEntry(date: Date(), snapshot: snapshot)

        let nextReload: Date
        if let snapshot {
            nextReload = snapshot.writtenAt.addingTimeInterval(Double(snapshot.pollIntervalSeconds))
        } else {
            // No snapshot yet (app never launched) — retry in 5 minutes.
            nextReload = Date().addingTimeInterval(300)
        }
        completion(Timeline(entries: [entry], policy: .after(nextReload)))
    }
}

struct SwitchboardWidgetEntryView: View {
    var entry: SwitchboardProvider.Entry

    var body: some View {
        Group {
            if let snapshot = entry.snapshot {
                VStack(spacing: 4) {
                    ZStack {
                        Circle()
                            .stroke(Color.gray.opacity(0.25), lineWidth: 6)
                        Circle()
                            .trim(from: 0, to: min(snapshot.fiveHourPct / 100, 1.0))
                            .stroke(color(for: snapshot.colorBand), style: StrokeStyle(lineWidth: 6, lineCap: .round))
                            .rotationEffect(.degrees(-90))
                        Text("\(Int(snapshot.fiveHourPct))%")
                            .font(.system(.title3, design: .monospaced))
                            .bold()
                    }
                    .padding(8)

                    Text(resetLabel(snapshot.fiveHourResetAt))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    Text(freshnessLabel(snapshot.writtenAt))
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
            } else {
                VStack(spacing: 4) {
                    Image(systemName: "questionmark.circle")
                        .font(.title2)
                    Text("Open Claude Switchboard")
                        .font(.caption2)
                        .multilineTextAlignment(.center)
                }
            }
        }
        .widgetURL(URL(string: "claude-switchboard://open"))
    }

    // Matches tray_icon::shared::{accent, warn, danger} in the Rust
    // crate exactly (hex 0xD97757 / 0xE89149 / 0xD85A45).
    private func color(for band: String) -> Color {
        switch band {
        case "danger": return Color(red: 0.847, green: 0.353, blue: 0.271)
        case "warn": return Color(red: 0.910, green: 0.569, blue: 0.286)
        default: return Color(red: 0.851, green: 0.467, blue: 0.341)
        }
    }

    private func resetLabel(_ date: Date?) -> String {
        guard let date else { return "—" }
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .abbreviated
        return "resets " + formatter.localizedString(for: date, relativeTo: Date())
    }

    private func freshnessLabel(_ writtenAt: Date) -> String {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .abbreviated
        return "as of " + formatter.localizedString(for: writtenAt, relativeTo: Date())
    }
}

struct SwitchboardWidget: Widget {
    let kind: String = "SwitchboardWidget"

    var body: some WidgetConfiguration {
        StaticConfiguration(kind: kind, provider: SwitchboardProvider()) { entry in
            SwitchboardWidgetEntryView(entry: entry)
        }
        .configurationDisplayName("Claude Switchboard")
        .description("5-hour usage for your active account.")
        .supportedFamilies([.systemSmall])
    }
}
```

Create `SwitchboardWidgetBundle.swift` (replace Xcode's generated `@main` bundle file with this):

```swift
import WidgetKit
import SwiftUI

@main
struct SwitchboardWidgetBundle: WidgetBundle {
    var body: some Widget {
        SwitchboardWidget()
    }
}
```

- [ ] **Step 4: Verify in Xcode's widget preview**

In `SwitchboardWidget.swift`, add a preview provider for manual verification (Xcode's canvas, not `xcodebuild test` — WidgetKit has no meaningful headless unit-test story for `TimelineProvider`/SwiftUI rendering):

```swift
#Preview(as: .systemSmall) {
    SwitchboardWidget()
} timeline: {
    SwitchboardEntry(date: .now, snapshot: SwitchboardWidgetSnapshot(
        accountLabel: "jay@example.com",
        tier: "MAX",
        fiveHourPct: 42,
        fiveHourResetAt: Date().addingTimeInterval(3600 * 3),
        colorBand: "safe",
        pollIntervalSeconds: 300,
        writtenAt: .now
    ))
    SwitchboardEntry(date: .now, snapshot: SwitchboardWidgetSnapshot(
        accountLabel: "jay@example.com",
        tier: "MAX",
        fiveHourPct: 92,
        fiveHourResetAt: Date().addingTimeInterval(1800),
        colorBand: "danger",
        pollIntervalSeconds: 300,
        writtenAt: Date().addingTimeInterval(-1200)
    ))
    SwitchboardEntry(date: .now, snapshot: nil)
}
```

Open Xcode's canvas (Editor → Canvas) for this file. Expected: three preview states render — a safe/teal-ish ring at 42%, a danger/coral ring at 92% with a visibly older "as of 20m ago" label, and the "Open Claude Switchboard" empty state. Confirm all three read correctly at actual widget size before moving on.

- [ ] **Step 5: Commit**

```bash
git add native/macos/
git commit -m "feat(widget): add SwitchboardWidget WidgetKit extension"
```

---

### Task 5: Entitlements + local build/sign/embed script

**Files:**
- Create: `src-tauri/entitlements.plist`
- Create: `scripts/build-widget-local.sh`

**Interfaces:**
- Consumes: the Tauri build output (`src-tauri/target/release/bundle/macos/Claude Switchboard.app`), the widget extension project from Task 4, `APPLE_SIGNING_IDENTITY` env var (your local `Developer ID Application: ...` codesign identity — find it via `security find-identity -v -p codesigning`).
- Produces: a locally signed `Claude Switchboard.app` with the widget extension embedded, runnable on the machine that built it.

- [ ] **Step 1: Substitute your Team ID everywhere**

Replace the `<TEAM_ID>` placeholder with your real Apple Developer Team ID in all three places it appears:
1. `src-tauri/src/widget_snapshot.rs` — the `APP_GROUP_ID` constant.
2. `native/macos/SwitchboardWidget/SwitchboardWidgetExtension/SwitchboardWidgetSnapshot.swift` — the `appGroupID` static constant.
3. `native/macos/SwitchboardWidget/SwitchboardWidgetExtension/SwitchboardWidgetExtension.entitlements` — the `application-groups` array entry (already substituted if you entered the real ID directly in Task 4 Step 1's Xcode capability UI — double check it, since Xcode's UI accepts the ID as typed).

All three must match byte-for-byte or the widget will silently fail to read the shared container (`FileManager.containerURL` returns `nil` on a mismatch).

- [ ] **Step 2: Create the main app's entitlements file**

Create `src-tauri/entitlements.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.application-groups</key>
    <array>
        <string>YOUR_TEAM_ID.com.claude-switchboard.app</string>
    </array>
</dict>
</plist>
```

Replace `YOUR_TEAM_ID` with your real Team ID (same value as Step 1).

- [ ] **Step 3: Write the build/sign/embed script**

Create `scripts/build-widget-local.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

# Builds Claude Switchboard, builds the WidgetKit extension, embeds it,
# and re-signs the whole bundle with your local Developer ID identity.
# Local/personal use only — see
# docs/superpowers/specs/2026-07-31-macos-widget-design.md §4.
#
# Usage:
#   APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)" \
#     ./scripts/build-widget-local.sh
#
# Find your identity with: security find-identity -v -p codesigning

: "${APPLE_SIGNING_IDENTITY:?Set APPLE_SIGNING_IDENTITY to your codesign identity}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_PATH="$REPO_ROOT/src-tauri/target/release/bundle/macos/Claude Switchboard.app"
WIDGET_PROJECT="$REPO_ROOT/native/macos/SwitchboardWidget/SwitchboardWidget.xcodeproj"
WIDGET_SCHEME="SwitchboardWidgetExtension"
WIDGET_BUILD_DIR="$REPO_ROOT/native/macos/build"
WIDGET_ENTITLEMENTS="$REPO_ROOT/native/macos/SwitchboardWidget/SwitchboardWidgetExtension/SwitchboardWidgetExtension.entitlements"
APP_ENTITLEMENTS="$REPO_ROOT/src-tauri/entitlements.plist"

echo "== 1. Building Claude Switchboard (tauri build) =="
(cd "$REPO_ROOT" && pnpm tauri build --bundles app)

echo "== 2. Building the widget extension =="
xcodebuild -project "$WIDGET_PROJECT" -scheme "$WIDGET_SCHEME" \
  -configuration Release -derivedDataPath "$WIDGET_BUILD_DIR" build

APPEX_SRC="$WIDGET_BUILD_DIR/Build/Products/Release/${WIDGET_SCHEME}.appex"
PLUGINS_DIR="$APP_PATH/Contents/PlugIns"

echo "== 3. Embedding the extension =="
mkdir -p "$PLUGINS_DIR"
rm -rf "$PLUGINS_DIR/${WIDGET_SCHEME}.appex"
cp -R "$APPEX_SRC" "$PLUGINS_DIR/"

echo "== 4. Codesigning (extension first, then the app) =="
codesign --force --options runtime \
  --entitlements "$WIDGET_ENTITLEMENTS" \
  --sign "$APPLE_SIGNING_IDENTITY" \
  "$PLUGINS_DIR/${WIDGET_SCHEME}.appex"

codesign --force --options runtime \
  --entitlements "$APP_ENTITLEMENTS" \
  --sign "$APPLE_SIGNING_IDENTITY" \
  "$APP_PATH"

echo "== 5. Verifying =="
codesign --verify --strict --verbose=2 "$APP_PATH"

echo "Done: $APP_PATH"
echo "Launch it once, then add the widget: right-click the desktop > Edit Widgets > search 'Claude Switchboard'."
```

Make it executable:

```bash
chmod +x scripts/build-widget-local.sh
```

- [ ] **Step 4: Run it end-to-end**

Run: `APPLE_SIGNING_IDENTITY="Developer ID Application: <Your Name> (<TEAM_ID>)" ./scripts/build-widget-local.sh`
Expected: script completes through "== 5. Verifying ==" with `codesign --verify` printing no errors and exiting 0, ending with the "Done:" line.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/entitlements.plist scripts/build-widget-local.sh
git commit -m "feat(widget): add local build/sign/embed script for the widget extension"
```

---

### Task 6: End-to-end manual verification

**Files:** none (verification only).

**Interfaces:**
- Consumes: everything from Tasks 1-5.
- Produces: confidence the feature works as designed before considering it done.

- [ ] **Step 1: Launch the signed app and confirm it runs**

Run: `open "src-tauri/target/release/bundle/macos/Claude Switchboard.app"`
Expected: app launches normally (tray icon appears, popover works as before) — confirms codesigning with the new entitlements didn't break the existing app.

- [ ] **Step 2: Add the widget to the desktop**

Right-click the desktop → Edit Widgets → search "Claude Switchboard" → drag the small widget onto the desktop.
Expected: within a few seconds, the widget shows a ring matching the current tray icon's percentage, the correct reset time, and an "as of Xm ago" label that starts near 0m.

- [ ] **Step 3: Confirm the ring matches the tray icon**

Compare the widget's percentage and color against the menu-bar tray icon's ring at the same moment.
Expected: identical percentage and color band (safe/warn/danger).

- [ ] **Step 4: Confirm tap-to-open**

Click the widget.
Expected: Claude Switchboard's popover opens and gains focus (same as clicking the tray icon).

- [ ] **Step 5: Confirm freshness label advances**

Wait past one poll interval (or temporarily lower `polling_interval_secs` in Settings for faster iteration) without touching the app otherwise.
Expected: the "as of Xm ago" label increases, then resets to "as of 0m ago" (or similar) after the next successful poll — confirming the timeline reload schedule is working and the label isn't hardcoded.

- [ ] **Step 6: Confirm the empty state**

Quit Claude Switchboard entirely, delete `~/Library/Group Containers/<TEAM_ID>.com.claude-switchboard.app/snapshot.json`, remove and re-add the widget.
Expected: "Open Claude Switchboard" empty state renders instead of a stale or fabricated percentage.

No commit for this task — if any step fails, fix the relevant earlier task and re-run.
