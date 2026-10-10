//! Validator coverage for the TimescaleDB rules (spec section 3.4).

use prax_schema::{SchemaError, validate_schema};

const DS: &str = r#"datasource db { provider = "postgresql" url = env("DATABASE_URL") extensions = [timescaledb] }"#;

fn ts(body: &str) -> String {
    format!("{DS}\n{body}\n")
}

/// Concatenated messages of every sub-error.
fn error_text(schema: &str) -> String {
    match validate_schema(schema) {
        Ok(_) => String::new(),
        Err(SchemaError::ValidationFailed { errors, .. }) => errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        Err(e) => e.to_string(),
    }
}

fn assert_err(schema: &str, fragment: &str) {
    assert!(
        validate_schema(schema).is_err(),
        "expected error containing `{fragment}`, schema validated"
    );
    let text = error_text(schema);
    assert!(text.contains(fragment), "expected `{fragment}` in:\n{text}");
}

fn assert_ok(schema: &str) {
    if let Err(e) = validate_schema(schema) {
        panic!("expected ok, got: {e}\n{}", error_text(schema));
    }
}

const METRIC: &str = r#"
model Metric {
  time     DateTime
  deviceId String
  value    Float

  @@id([deviceId, time])
  @@timescale.hypertable(time, chunkInterval: "1 day", space: hash(deviceId, 4))
  @@timescale.columnstore(segmentBy: [deviceId], orderBy: [desc(time)], after: "7 days")
  @@timescale.retention(dropAfter: "90 days")
}
"#;

fn hourly(extra_args: &str) -> String {
    format!(
        r#"
view HourlyMetric {{
  bucket   DateTime
  deviceId String
  avgValue Float

  @@timescale.continuous(source: Metric, scheduleInterval: "1 hour"{extra_args})
}}

@@sql("HourlyMetric", """
SELECT time_bucket('1 hour', "time") AS bucket, "deviceId", avg("value") AS "avgValue"
FROM "Metric"
GROUP BY 1, 2
""")
"#
    )
}

#[test]
fn spec_example_validates() {
    assert_ok(&ts(&format!(
        "{METRIC}{}",
        hourly(r#", startOffset: "3 days", endOffset: "1 hour""#)
    )));
}

#[test]
fn rule1_typo_rejected() {
    let s = ts(r#"model M { time DateTime @@id([time]) @@timescale.hypertabel(time) }"#);
    assert_err(&s, "hypertabel");
}

#[test]
fn rule1_bare_timescale_rejected() {
    let s = ts(r#"model M { time DateTime @@id([time]) @@timescale }"#);
    assert_err(&s, "unknown attribute `@@timescale`");
}

#[test]
fn rule2_requires_postgres() {
    let s = r#"datasource db { provider = "mysql" url = env("DATABASE_URL") extensions = [timescaledb] }
model M { time DateTime @@id([time]) @@timescale.hypertable(time) }"#;
    assert_err(s, "postgresql");
}

#[test]
fn rule2_requires_extension() {
    let s = r#"datasource db { provider = "postgresql" url = env("DATABASE_URL") }
model M { time DateTime @@id([time]) @@timescale.hypertable(time) }"#;
    assert_err(s, "timescaledb");
}

#[test]
fn rule3_time_field_missing() {
    let s = ts(r#"model M { id Int @id @@timescale.hypertable(when) }"#);
    assert_err(&s, "time field `when` does not exist");
}

#[test]
fn rule3_time_field_wrong_type() {
    let s = ts(r#"model M { time String @@id([time]) @@timescale.hypertable(time) }"#);
    assert_err(&s, "must be DateTime");
}

#[test]
fn rule3_time_field_optional() {
    let s = ts(r#"model M { id Int @id time DateTime? @@timescale.hypertable(time) }"#);
    assert_err(&s, "must be required, not optional");
}

#[test]
fn rule3_date_allowed() {
    let s = ts(r#"model M { day Date @@id([day]) @@timescale.hypertable(day) }"#);
    assert_ok(&s);
}

#[test]
fn rule4_space_is_time_field() {
    let s = ts(
        r#"model M { time DateTime @@id([time]) @@timescale.hypertable(time, space: hash(time, 4)) }"#,
    );
    assert_err(&s, "must differ from the time field");
}

#[test]
fn rule4_space_field_missing() {
    let s = ts(
        r#"model M { time DateTime @@id([time]) @@timescale.hypertable(time, space: hash(nope, 4)) }"#,
    );
    assert_err(&s, "space field `nope` does not exist");
}

#[test]
fn rule5_id_without_time() {
    let s = ts(
        r#"model M { time DateTime deviceId String @@id([deviceId]) @@timescale.hypertable(time) }"#,
    );
    assert_err(
        &s,
        "`@@id` (deviceId) must include hypertable dimension field `time`",
    );
}

#[test]
fn rule5_field_level_id_without_time() {
    let s = ts(r#"model M { id Int @id time DateTime @@timescale.hypertable(time) }"#);
    assert_err(
        &s,
        "`@id` (id) must include hypertable dimension field `time`",
    );
}

#[test]
fn rule5_unique_without_space() {
    let s = ts(
        r#"model M { time DateTime deviceId String @@id([time, deviceId]) @@unique([time]) @@timescale.hypertable(time, space: hash(deviceId, 4)) }"#,
    );
    assert_err(
        &s,
        "`@@unique` (time) must include hypertable dimension field `deviceId`",
    );
}

/// Rule 6 was dropped (spec §8 item 4): TimescaleDB accepts foreign keys
/// that reference a hypertable.
#[test]
fn fk_to_hypertable_is_allowed() {
    let s = ts(&format!(
        r#"{METRIC}
model Alert {{
  id           Int      @id
  metricTime   DateTime
  metricDevice String
  metric       Metric   @relation(fields: [metricTime, metricDevice], references: [time, deviceId])
}}"#
    ));
    assert_ok(&s);
}

#[test]
fn rule8_unknown_segment_field() {
    let s = ts(
        r#"model M { time DateTime @@id([time]) @@timescale.hypertable(time) @@timescale.columnstore(segmentBy: [ghost]) }"#,
    );
    assert_err(&s, "segmentBy field `ghost` does not exist");
}

#[test]
fn rule8_field_in_both_lists() {
    let s = ts(
        r#"model M { time DateTime d String @@id([time]) @@timescale.hypertable(time) @@timescale.columnstore(segmentBy: [d], orderBy: [d]) }"#,
    );
    assert_err(&s, "appears in both segmentBy and orderBy");
}

#[test]
fn rule10_continuous_without_sql() {
    let s = ts(&format!(
        r#"{METRIC}
view V {{ bucket DateTime @@timescale.continuous(source: Metric, scheduleInterval: "1 hour", startOffset: "3 days") }}"#
    ));
    assert_err(&s, "continuous aggregate requires SQL");
}

#[test]
fn rule11_source_not_hypertable() {
    let s = ts(r#"
model Plain { id Int @id }
view V { bucket DateTime @@timescale.continuous(source: Plain, scheduleInterval: "1 hour") }
@@sql("V", """
SELECT 1
""")
"#);
    assert_err(
        &s,
        "source `Plain` must be a hypertable model or a continuous aggregate view",
    );
}

#[test]
fn rule11_source_cycle() {
    let s = ts(r#"
view A { bucket DateTime @@timescale.continuous(source: B, scheduleInterval: "1 hour") }
view B { bucket DateTime @@timescale.continuous(source: A, scheduleInterval: "1 hour") }
@@sql("A", """
SELECT 1
""")
@@sql("B", """
SELECT 1
""")
"#);
    assert_err(&s, "source graph has a cycle");
}

#[test]
fn rule12_missing_start_offset_with_retention() {
    assert_err(
        &ts(&format!("{METRIC}{}", hourly(""))),
        "`startOffset` is required",
    );
}

#[test]
fn rule12_start_offset_too_long() {
    assert_err(
        &ts(&format!(
            "{METRIC}{}",
            hourly(r#", startOffset: "100 days""#)
        )),
        "`startOffset` must be shorter than `dropAfter`",
    );
}

#[test]
fn rule12_month_vs_days_conservative() {
    let metric = METRIC.replace("90 days", "30 days");
    assert_err(
        &ts(&format!(
            "{metric}{}",
            hourly(r#", startOffset: "1 month""#)
        )),
        "`startOffset` must be shorter than `dropAfter`",
    );
}

#[test]
fn rule12_ok() {
    assert_ok(&ts(&format!(
        "{METRIC}{}",
        hourly(r#", startOffset: "3 days""#)
    )));
}
