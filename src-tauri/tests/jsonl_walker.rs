use claude_switchboard_lib::jsonl_parser::{walker, PricingTable};
use claude_switchboard_lib::store::{Db, StoredAccount};
use std::fs;
use tempfile::tempdir;

fn setup() -> (tempfile::TempDir, Db, PricingTable, std::path::PathBuf) {
    let d = tempdir().unwrap();
    let db_dir = d.path().join("db");
    let projects = d.path().join("projects");
    let proj = projects.join("demo");
    fs::create_dir_all(&proj).unwrap();
    let db = Db::open(&db_dir).unwrap();
    db.upsert_account(&StoredAccount {
        id: "acc".into(),
        email: "e".into(),
        display_name: None,
    })
    .unwrap();
    (d, db, PricingTable::bundled().unwrap(), projects)
}

#[test]
fn ingests_current_schema_file() {
    let (_d, db, p, projects) = setup();
    let f = projects.join("demo").join("session.jsonl");
    fs::copy("tests/fixtures/jsonl/current_schema.jsonl", &f).unwrap();
    let n = walker::ingest_file(&db, &p, &f, &projects).unwrap();
    assert_eq!(n, 3);
}

#[test]
fn idempotent_on_same_file() {
    let (_d, db, p, projects) = setup();
    let f = projects.join("demo").join("session.jsonl");
    fs::copy("tests/fixtures/jsonl/current_schema.jsonl", &f).unwrap();
    let a = walker::ingest_file(&db, &p, &f, &projects).unwrap();
    let b = walker::ingest_file(&db, &p, &f, &projects).unwrap();
    assert_eq!(a, 3);
    assert_eq!(b, 0);
}

#[test]
fn partial_line_at_eof_is_not_consumed() {
    let (_d, db, p, projects) = setup();
    let f = projects.join("demo").join("session.jsonl");
    fs::copy("tests/fixtures/jsonl/partial_line_at_eof.jsonl", &f).unwrap();
    let n = walker::ingest_file(&db, &p, &f, &projects).unwrap();
    assert_eq!(n, 1, "only the first complete line is ingested");

    let mut contents = fs::read_to_string(&f).unwrap();
    contents.push_str(",\"output_tokens\":30}\n");
    fs::write(&f, contents).unwrap();
    let n = walker::ingest_file(&db, &p, &f, &projects).unwrap();
    assert_eq!(n, 1, "completed line ingested on next pass");
}

#[test]
fn truncation_resets_cursor_and_dedupes() {
    let (_d, db, p, projects) = setup();
    let f = projects.join("demo").join("session.jsonl");
    fs::copy("tests/fixtures/jsonl/current_schema.jsonl", &f).unwrap();
    assert_eq!(walker::ingest_file(&db, &p, &f, &projects).unwrap(), 3);

    let first_line =
        include_str!("fixtures/jsonl/current_schema.jsonl").lines().next().unwrap().to_string()
            + "\n";
    fs::write(&f, first_line).unwrap();

    let n = walker::ingest_file(&db, &p, &f, &projects).unwrap();
    assert_eq!(n, 0, "cursor reset + dedup should add no new rows");

    let n2 = walker::ingest_file(&db, &p, &f, &projects).unwrap();
    assert_eq!(n2, 0);

    let count = db
        .events_between(
            chrono::Utc::now() - chrono::Duration::days(3650),
            chrono::Utc::now() + chrono::Duration::days(1),
        )
        .unwrap()
        .len();
    assert_eq!(count, 3);
}

#[test]
fn malformed_lines_are_skipped_not_fatal() {
    let (_d, db, p, projects) = setup();
    let f = projects.join("demo").join("session.jsonl");
    fs::copy("tests/fixtures/jsonl/malformed_lines.jsonl", &f).unwrap();
    let n = walker::ingest_file(&db, &p, &f, &projects).unwrap();
    assert_eq!(n, 3, "only 3 of 5 lines are valid");
}

#[test]
fn archives_raw_lines_alongside_events() {
    let (_d, db, p, projects) = setup();
    let f = projects.join("demo").join("session.jsonl");
    fs::copy("tests/fixtures/jsonl/current_schema.jsonl", &f).unwrap();
    walker::ingest_file(&db, &p, &f, &projects).unwrap();

    let rel = f.strip_prefix(&projects).unwrap().to_string_lossy().into_owned();
    let lines = db.transcript_lines_for_path(&rel).unwrap();
    let expected: Vec<&str> = include_str!("fixtures/jsonl/current_schema.jsonl").lines().collect();
    assert_eq!(
        lines.len(),
        expected.len(),
        "every raw line is archived, not just usage-bearing ones"
    );
    for (row, original) in lines.iter().zip(expected.iter()) {
        assert_eq!(row.raw_line, original.trim());
    }
}

#[test]
fn archive_reflects_new_content_after_truncation() {
    let (_d, db, p, projects) = setup();
    let f = projects.join("demo").join("session.jsonl");
    fs::copy("tests/fixtures/jsonl/current_schema.jsonl", &f).unwrap();
    walker::ingest_file(&db, &p, &f, &projects).unwrap();

    fs::write(&f, "{\"different\":true}\n").unwrap();
    walker::ingest_file(&db, &p, &f, &projects).unwrap();

    let rel = f.strip_prefix(&projects).unwrap().to_string_lossy().into_owned();
    let lines = db.transcript_lines_for_path(&rel).unwrap();
    // Rows at offsets the shorter post-truncation content never reaches
    // (e.g. the original fixture's other lines) legitimately survive as
    // historical content — the archive is never pruned. Only the row at
    // offset 0 was actually revisited, so only it needs to prove REPLACE
    // (not IGNORE) won over the stale content that used to live there.
    let at_offset_zero = lines
        .iter()
        .find(|l| l.line_no == 0)
        .expect("offset 0 must have a row after truncation");
    assert_eq!(
        at_offset_zero.raw_line, "{\"different\":true}",
        "replace must win over the stale row at the same offset"
    );
}

#[test]
fn discover_jsonl_skips_deep_nesting() {
    let (_d, _db, _p, projects) = setup();
    let deep = projects.join("demo").join("nested").join("deeper");
    fs::create_dir_all(&deep).unwrap();
    fs::write(
        deep.join("hidden.jsonl"),
        r#"{"ts":"2026-01-01T00:00:00Z","project":"x","model":"opus"}"#,
    )
    .unwrap();
    fs::write(projects.join("demo").join("session.jsonl"), "").unwrap();
    let files = walker::discover_jsonl_files(&projects).unwrap();
    assert_eq!(files.len(), 1, "only the one-level file is discovered");
}
