# macOS Desktop Widget — Design Specification

**Date:** 2026-07-31
**Status:** Design approved, pending implementation plan
**Scope:** macOS only, local builds only (not shipped in public CI/releases)

---

## 1. Overview

A macOS 14+ desktop widget (WidgetKit) for Claude Switchboard, showing the active account's 5-hour usage ring — the same number already shown as the tray icon's ring badge. Tapping the widget opens the app's popover. The widget is a passive reader of data the already-running Tauri app has already polled; it does not perform its own networking or OAuth.

### Why this shape

This is the smallest useful slice of "widget": it reuses an existing, already-designed data point (the tray ring) instead of inventing new UI, and defers the more expensive pieces — multiple widget sizes, in-widget interactive actions, and public signed/notarized distribution — to a later pass once the core mechanism (shared-container data flow, embedding a `.appex` in a Tauri-built bundle) is proven to work.

### Non-goals (this pass)

- Medium/large widget sizes, multi-account summary widget
- In-widget interactive actions (App Intents / buttons)
- Public CI signing, notarization, or shipping the widget in GitHub releases
- Push-based (Darwin notification) instant refresh — the widget relies on WidgetKit's own timeline schedule

### Prerequisite

Requires a paid Apple Developer Program membership (for the App Group entitlement and code signing). This is a hard WidgetKit requirement, not a project choice — App Groups do not work with ad-hoc/unsigned builds.

---

## 2. Architecture

```
┌─────────────────────────┐         ┌──────────────────────────────┐
│  Claude Switchboard.app │         │  Widget Extension (.appex)    │
│  (Tauri/Rust, existing) │         │  (new, Swift/WidgetKit)       │
│                          │         │                                │
│  usage_api poller        │         │  TimelineProvider              │
│  ──► store (SQLite)      │         │   reads snapshot.json          │
│  ──► NEW: widget_snapshot│  write  │   ◄── from shared container    │
│       writes JSON  ──────┼────────►│   builds SwiftUI entry view    │
│                          │  App     │   requests next reload          │
│                          │  Group   │   ≈ next poll time              │
│                          │  container                               │
└─────────────────────────┘         └──────────────────────────────┘
         ▲                                        │
         │ claude-switchboard://open              │
         └────────────────────────────────────────┘
                    tap widget → activates app
```

Both processes are sandboxed and cannot see each other's private storage directly. The App Group shared container (`~/Library/Group Containers/<TeamID>.com.claude-switchboard.app/`) is the only channel between them, and it is one-way: Rust writes, the widget reads.

---

## 3. Components

### 3.1 Rust: `widget_snapshot` module (new)

Called from the existing poll cycle after each **successful** poll. Writes a small JSON file to the App Group container, describing the active account only:

```json
{
  "accountLabel": "jay@example.com",
  "tier": "MAX",
  "fiveHourPct": 42,
  "fiveHourResetAt": "2026-07-31T18:00:00Z",
  "colorBand": "safe",
  "pollIntervalSeconds": 300,
  "writtenAt": "2026-07-31T15:32:00Z"
}
```

- `fiveHourPct` and the safe/amber/danger color band are computed by Rust using the same threshold logic as the tray icon — the widget never re-derives thresholds itself.
- No write on a failed poll. A failed poll simply lets the existing snapshot age; this is surfaced to the user via the freshness label (§3.2), not hidden.
- `pollIntervalSeconds` is included so the widget's timeline provider can schedule its next reload without hardcoding or guessing the app's configured polling cadence.

### 3.2 Swift: `SwitchboardWidgetExtension` (new, under `native/macos/`)

- `StaticConfiguration`, single widget family: `.systemSmall`.
- `TimelineProvider`:
  - Reads `snapshot.json` from the shared container.
  - If the file is missing, or `accountLabel` doesn't match any known/active account, renders an explicit empty state ("Open Claude Switchboard") rather than a fabricated 0%.
  - Requests the next timeline reload at `writtenAt + pollIntervalSeconds` (falling back to a default if the snapshot is absent).
- Entry view: ring using the same safe/amber/danger color bands as the tray icon, percentage, reset time, and an "as of Xm ago" relative freshness label — so a stale number is never presented as live. This directly follows the brand principle of "quiet, confident, trustworthy": an honest stale number beats a falsely-fresh one.
- `widgetURL(URL(string: "claude-switchboard://open"))` on the entry view — tapping anywhere on the widget activates the main app.

### 3.3 Main app: custom URL scheme handling

- Register `claude-switchboard://` via the `tauri-plugin-deep-link` plugin's config: the `schemes` array under `plugins.deep-link.desktop` in `src-tauri/tauri.conf.json`. The Tauri CLI uses this config to generate the app bundle's `CFBundleURLTypes` Info.plist entry automatically at build time — no manual Info.plist editing.
- Handle the incoming URL through the plugin's `on_open_url` listener (registered in `lib.rs`'s `setup()`) to bring the popover forward — equivalent to clicking the tray icon. Also check `get_current()` at startup for a launch-time URL that arrived before the listener was registered (the common cold-start case for a widget tap).

### 3.4 Data contract ownership

The snapshot JSON schema is intentionally minimal and Rust-owned. The widget is a dumb renderer of numbers Rust has already computed, including which color band to display. No threshold or business logic is duplicated in Swift — this keeps the store/notifier modules as the single source of truth for usage-state interpretation, consistent with the rest of the app's architecture.

---

## 4. Build, signing, and testing (local-only)

- A new Xcode project lives under `native/macos/`, containing only the widget extension target (no full host-app target needed in Xcode — the Tauri-built `.app` is the host).
- A local (non-CI) post-build script:
  1. Runs `tauri build` to produce `Claude Switchboard.app`.
  2. Builds the widget extension via `xcodebuild`.
  3. Copies the resulting `.appex` into `Claude Switchboard.app/Contents/PlugIns/`.
  4. Adds the App Group entitlement to both the main app and the widget extension.
  5. Codesigns inner-then-outer (extension first, then the outer `.app`) using the developer's own Developer ID Application certificate.
- No notarization step for v1 — Gatekeeper allows a locally signed (but not notarized) app to run on the machine that built/signed it. This is out of scope for other users until the "ship in public CI" non-goal is revisited.
- Testing:
  - Xcode's widget preview/timeline-debug tooling covers the SwiftUI view and `TimelineProvider` logic in isolation, without a full app rebuild per iteration.
  - End-to-end verification (real snapshot file, real reload timing, tap-to-open) requires running the actual signed app and pinning the widget to the desktop.

---

## 5. Error handling & edge cases

| Scenario | Behavior |
|---|---|
| Main app never launched / no snapshot file yet | Widget shows "Open Claude Switchboard" empty state |
| Main app quit, snapshot exists but is old | Widget shows last known values with an honest "as of Xh ago" label; does not claim to be live |
| Active account switched since last snapshot | `accountLabel`/`tier` reflect whichever account was active as of the last successful poll; no attempt to reconcile mid-flight |
| Poll fails (network error, 401, etc.) | No snapshot write; existing snapshot ages naturally, surfaced via the freshness label |
| Widget removed and re-added by the user | `TimelineProvider` re-reads the current snapshot from the shared container on first placement — no special-case handling needed |

---

## 6. Testing plan

- Unit test `widget_snapshot` (Rust): verifies correct JSON shape, correct behavior on poll failure (no write), correct color-band computation reuse.
- Manual verification in Xcode's widget preview canvas for each `TimelineProvider` state (fresh data, stale data, missing snapshot, mismatched account).
- Manual end-to-end pass: build + sign locally, pin widget to desktop, confirm ring matches the tray icon, confirm tap opens the popover, confirm freshness label advances correctly over time and after a poll.
