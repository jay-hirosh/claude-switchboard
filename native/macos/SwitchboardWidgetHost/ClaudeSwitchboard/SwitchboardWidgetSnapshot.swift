//
//  SwitchboardWidgetSnapshot.swift
//  ClaudeSwitchboard
//

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
    static let appGroupID = "7VVV8Y9MKT.com.claude-switchboard.app"

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
    // seconds whenever they're nonzero, but Foundation's plain `.iso8601`
    // strategy rejects any fractional component. Try both formats.
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
