//
//  ClaudeSwitchboard.swift
//  ClaudeSwitchboard
//

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

struct ClaudeSwitchboardEntryView: View {
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
        .containerBackground(.fill.tertiary, for: .widget)
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

struct ClaudeSwitchboard: Widget {
    let kind: String = "ClaudeSwitchboard"

    var body: some WidgetConfiguration {
        StaticConfiguration(kind: kind, provider: SwitchboardProvider()) { entry in
            ClaudeSwitchboardEntryView(entry: entry)
        }
        .configurationDisplayName("Claude Switchboard")
        .description("5-hour usage for your active account.")
        .supportedFamilies([.systemSmall])
    }
}

#Preview(as: .systemSmall) {
    ClaudeSwitchboard()
} timeline: {
    SwitchboardEntry(date: .now, snapshot: SwitchboardWidgetSnapshot(
        accountLabel: "jay@example.com", tier: "MAX", fiveHourPct: 42,
        fiveHourResetAt: Date().addingTimeInterval(3600 * 3),
        colorBand: "safe", pollIntervalSeconds: 300, writtenAt: .now
    ))
    SwitchboardEntry(date: .now, snapshot: SwitchboardWidgetSnapshot(
        accountLabel: "jay@example.com", tier: "MAX", fiveHourPct: 92,
        fiveHourResetAt: Date().addingTimeInterval(1800),
        colorBand: "danger", pollIntervalSeconds: 300, writtenAt: Date().addingTimeInterval(-1200)
    ))
    SwitchboardEntry(date: .now, snapshot: nil)
}
