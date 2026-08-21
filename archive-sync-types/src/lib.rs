use serde::{Deserialize, Serialize};

/// Wire format for one archived transcript line. Deliberately has no
/// `device_id` field — the server stamps that from the authenticated
/// caller, never trusting a client-supplied value for its own identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncTranscriptLine {
    pub project_slug: String,
    pub session_id: String,
    pub jsonl_path: String,
    pub line_no: i64,
    pub raw_line: String,
    pub ingested_at: i64,
}

/// Wire format for one archived file snapshot. Same no-device_id rule as
/// SyncTranscriptLine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncFileSnapshot {
    pub source_path: String,
    pub kind: String,
    pub content: String,
    pub content_hash: String,
    pub captured_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushRequest {
    pub transcript_lines: Vec<SyncTranscriptLine>,
    pub file_snapshots: Vec<SyncFileSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushResponse {
    pub transcript_lines_accepted: usize,
    pub file_snapshots_accepted: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PullQuery {
    pub since_transcript_seq: i64,
    pub since_snapshot_seq: i64,
    pub limit: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PullResponse {
    pub transcript_lines: Vec<SyncTranscriptLine>,
    pub file_snapshots: Vec<SyncFileSnapshot>,
    pub transcript_seq_high_water: i64,
    pub snapshot_seq_high_water: i64,
}

/// `device_id` is client-generated (a UUID formatted as a string) —
/// phase 1's local archive already assigns one per install, independent
/// of whether sync is ever enabled. The server validates and stores it,
/// never mints its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateAccountRequest {
    pub device_id: String,
    pub device_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateAccountResponse {
    pub user_id: String,
    pub device_id: String,
    pub api_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairCodeResponse {
    pub pairing_code: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JoinRequest {
    pub pairing_code: String,
    pub device_id: String,
    pub device_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JoinResponse {
    pub user_id: String,
    pub device_id: String,
    pub api_key: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_dto_round_trips_through_json() {
        let line = SyncTranscriptLine {
            project_slug: "p".into(),
            session_id: "s".into(),
            jsonl_path: "p/s.jsonl".into(),
            line_no: 0,
            raw_line: "{}".into(),
            ingested_at: 0,
        };
        let json = serde_json::to_string(&line).unwrap();
        let back: SyncTranscriptLine = serde_json::from_str(&json).unwrap();
        assert_eq!(line, back);

        let snap = SyncFileSnapshot {
            source_path: "/x".into(),
            kind: "settings".into(),
            content: "{}".into(),
            content_hash: "h".into(),
            captured_at: 0,
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: SyncFileSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snap, back);

        let push = PushRequest { transcript_lines: vec![line], file_snapshots: vec![snap] };
        let json = serde_json::to_string(&push).unwrap();
        let back: PushRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(push, back);

        let create_req = CreateAccountRequest {
            device_id: "d1".into(),
            device_name: "MacBook".into(),
        };
        let json = serde_json::to_string(&create_req).unwrap();
        let back: CreateAccountRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(create_req, back);

        let join_req = JoinRequest {
            pairing_code: "ABCD1234".into(),
            device_id: "d2".into(),
            device_name: "Desktop".into(),
        };
        let json = serde_json::to_string(&join_req).unwrap();
        let back: JoinRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(join_req, back);
    }
}
