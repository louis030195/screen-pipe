// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! Fictional adversarial contract fixtures. These do not measure LLM accuracy.
use super::*;

#[test]
fn recording_preview_request_respects_recorder_contract() {
    let at = DateTime::parse_from_rfc3339("2026-09-05T23:05:31Z").unwrap().with_timezone(&Utc);
    let url = recording_preview_url("http://localhost:3030", at, " Arc & Notes ").unwrap();
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(url.path(), "/frames/preview-samples");
    assert_eq!(query["limit"], "8");
    assert_eq!(query["app_name"], "Arc & Notes");
    let start = DateTime::parse_from_rfc3339(&query["start_time"]).unwrap().with_timezone(&Utc);
    let end = DateTime::parse_from_rfc3339(&query["end_time"]).unwrap().with_timezone(&Utc);
    assert_eq!((at - start).num_seconds(), 120);
    assert_eq!((end - at).num_seconds(), 120);
}

fn fixture() -> (Value, EvidenceCatalog) {
    let observations = [
        (
            "2026-08-01T10:00:00Z",
            "Open the approved draft in the editor",
        ),
        (
            "2026-08-01T10:05:00Z",
            "The preview shows the selected image and text",
        ),
        (
            "2026-08-02T10:00:00Z",
            "Open the approved draft in the editor",
        ),
        (
            "2026-08-02T10:05:00Z",
            "The preview shows the selected image and text",
        ),
    ];
    let snippets: Vec<Value> = observations
        .iter()
        .map(|(timestamp, text)| {
            json!({
                "source":"parsed", "timestamp":timestamp, "app_name":"Editor", "text":text
            })
        })
        .collect();
    let catalog = EvidenceCatalog::from_daily(&[json!({"snippets":snippets})]);
    let stages: Vec<Value> = (0..2)
        .map(|i| {
            json!({
                "name": if i == 0 { "Open approved draft" } else { "Inspect preview" },
                "description":observations[i].1, "confidence":90,
                "procedure":[{"kind":"action","text":observations[i].1,
                    "timestamp":observations[i].0,"app":"Editor","quote":observations[i].1}],
                "openQuestions":["Who approves the draft?"],
                "evidence":[{"timestamp":observations[i].0,"app":"Editor"},
                    {"timestamp":observations[i+2].0,"app":"Editor"}]
            })
        })
        .collect();
    (
        json!({"evidenceVersion":2,"workflows":[{
            "title":"Inspect an approved draft preview", "description":"Inspect text and image before handoff",
            "confidence":90,"captureSequence":[{"timestamp":observations[0].0,"app":"Editor"},{"timestamp":observations[1].0,"app":"Editor"}],"stages":stages,"openQuestions":["What confirms final approval?"],
            "limitations":["No completed handoff was observed."]
        }]}),
        catalog,
    )
}

#[test]
fn depth_preserves_exact_support_and_missing_details() {
    let (candidate, catalog) = fixture();
    let result = normalize_analysis(candidate, 7, &catalog).unwrap();
    let workflow = &result["workflows"][0];
    assert_eq!(workflow["evidenceVersion"], 3);
    assert_eq!(workflow["captureSequence"].as_array().unwrap().len(), 2);
    assert_eq!(
        workflow["stages"][0]["procedure"][0]["quote"],
        "Open the approved draft in the editor"
    );
    assert_eq!(
        workflow["openQuestions"][0],
        "What confirms final approval?"
    );
    assert_eq!(workflow["totalMinutes"], 0);
}

#[test]
fn depth_rejects_fabricated_quotes_wrong_sources_and_missing_middle_steps() {
    for (key, replacement) in [
        ("quote", "The campaign was successfully sent"),
        ("timestamp", "2026-08-03T10:00:00Z"),
        ("app", "UnrelatedApp"),
        ("quote", "Open"),
        ("kind", "imagined"),
    ] {
        let (mut candidate, catalog) = fixture();
        candidate["workflows"][0]["stages"][0]["procedure"][0][key] = json!(replacement);
        let result = normalize_analysis(candidate, 7, &catalog).unwrap();
        assert_eq!(result["workflows"][0]["stages"].as_array().unwrap().len(), 2);
        assert!(result["workflows"][0]["stages"][0]["procedure"].as_array().unwrap().is_empty(), "accepted invalid {key}");
        assert_eq!(result["workflows"][0]["evidenceStatus"], "candidate");
    }
}

#[test]
fn depth_rejects_meeting_metadata_and_audio_as_procedural_proof() {
    for source in ["meeting", "audio"] {
        let evidence = vec![
            json!({"timestamp":"2026-08-01T10:00:00Z", "app":"Meet", "source":source,
            "detail":"Meeting: customer workflow discovery"}),
        ];
        let stage = json!({"procedure":[{"kind":"action", "text":"Prepare and send the proposal",
            "timestamp":"2026-08-01T10:00:00Z","app":"Meet","quote":"Meeting: customer workflow discovery"}]});
        assert!(normalize_procedure(&stage, &evidence).is_empty());
    }
}


#[test]
fn depth_skill_receives_selected_procedure_and_uncertainty_not_whole_history() {
    let (candidate, catalog) = fixture();
    let result = normalize_analysis(candidate, 7, &catalog).unwrap();
    let mut workflow = result["workflows"][0].clone();
    workflow["privateHistory"] = json!("Unrelated private material");
    workflow["stages"][0]["procedure"][0]["screenshot"] = json!("private-image");
    let source = workflow_skill_source(&workflow);
    assert_eq!(
        source["stages"][0]["procedure"][0]["quote"],
        "Open the approved draft in the editor"
    );
    assert_eq!(
        source["limitations"][0],
        "No completed handoff was observed."
    );
    assert!(!source.to_string().contains("Unrelated private material"));
    assert!(!source.to_string().contains("private-image"));
    assert!(SKILL_SYSTEM_PROMPT.contains("Draft — not execution-tested"));
}


#[test]
fn depth_never_substitutes_a_different_app_at_the_same_timestamp() {
    let (_, catalog) = fixture();
    let timestamp = DateTime::parse_from_rfc3339("2026-08-01T10:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    assert!(catalog.resolve(timestamp, "Email").is_none());
    assert!(catalog.resolve(timestamp, "editor").is_some());
}

#[test]
fn depth_rejects_individually_supported_steps_without_an_ordered_occurrence() {
    let (candidate, catalog) = fixture();
    let normalized = normalize_analysis(candidate, 7, &catalog).unwrap();
    let mut stages = normalized["workflows"][0]["stages"]
        .as_array()
        .unwrap()
        .clone();
    let sequence = normalized["workflows"][0]["captureSequence"].clone();
    assert_eq!(ordered_capture_sequence(&stages, &sequence).len(), 2);
    assert!(ordered_capture_sequence(&stages, &Value::Null).is_empty());
    stages[1]["evidence"] =
        json!([{"source":"parsed", "app":"Editor", "timestamp":"2026-09-03T10:05:00Z"}]);
    assert!(ordered_capture_sequence(&stages, &sequence).is_empty());
    stages[1]["evidence"] =
        json!([{"source":"parsed", "app":"Editor", "timestamp":"2026-07-31T10:05:00Z"}]);
    assert!(ordered_capture_sequence(&stages, &sequence).is_empty());
}

#[test]
fn depth_missing_sequence_is_a_gap_not_a_deleted_workflow() {
    let (mut candidate, catalog) = fixture();
    // Valid references on two days, but the proposed first step occurs last.
    candidate["workflows"][0]["stages"][0]["evidence"] = json!([{"timestamp":"2026-08-02T10:00:00Z","app":"Editor"}]);
    candidate["workflows"][0]["stages"][1]["evidence"] = json!([{"timestamp":"2026-08-01T10:05:00Z","app":"Editor"}]);
    let result = normalize_analysis(candidate, 7, &catalog).unwrap();
    assert_eq!(result["workflows"].as_array().unwrap().len(), 1);
    assert!(result["workflows"][0]["captureSequence"].as_array().unwrap().is_empty());
    assert!(result["workflows"][0]["limitations"].to_string().contains("No ordered capture example"));
}


#[test]
fn replay_uses_nearest_valid_frame_within_two_minutes() {
    let at = DateTime::parse_from_rfc3339("2026-09-01T10:00:00Z").unwrap().with_timezone(&Utc);
    let frames = json!({"frames":[
        {"frame_id":1,"timestamp":"2026-09-01T10:02:01Z"},
        {"frame_id":2,"timestamp":"invalid"},
        {"frame_id":3,"timestamp":"2026-09-01T10:00:10Z"},
        {"frame_id":4,"timestamp":"2026-09-01T10:00:01Z"}
    ]});
    let (frame, distance) = closest_recording_frame(&frames, at).unwrap();
    assert_eq!(frame["frame_id"], 4);
    assert_eq!(distance, 1);
    assert!(closest_recording_frame(&json!({"frames":[{"timestamp":"2026-09-01T10:02:01Z"}]}), at).is_none());
}

#[test]
fn sequence_requires_agent_selection_and_allows_connected_cross_day_work() {
    let at = "2026-08-01T10:00:00+00:00";
    let later = "2026-08-04T10:00:00+00:00";
    let stages = vec![json!({"evidence":[{"timestamp":at,"app":"Editor","source":"parsed"}]}), json!({"evidence":[{"timestamp":later,"app":"Editor","source":"parsed"}]})];
    assert!(ordered_capture_sequence(&stages, &Value::Null).is_empty());
    let selected = json!([{"timestamp":at,"app":"Editor"},{"timestamp":later,"app":"Editor"}]);
    assert_eq!(ordered_capture_sequence(&stages, &selected).len(), 2);
}

#[test]
fn discovery_index_benchmark_keeps_raw_history_out_of_the_prompt() {
    let start = DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z").unwrap().with_timezone(&Utc);
    let daily: Vec<_> = (0..90).map(|day| json!({
        "start":(start + ChronoDuration::days(day)).to_rfc3339(),
        "end":(start + ChronoDuration::days(day + 1)).to_rfc3339(),
        "apps":[{"name":"Editor"}], "total_frames":100, "total_active_minutes":60,
        "snippets":(0..30).map(|_| json!({"text":"synthetic-capture-content ".repeat(48)})).collect::<Vec<_>>()
    })).collect();
    let eager_bytes = serde_json::to_vec(&daily).unwrap().len();
    let started = std::time::Instant::now();
    let prompt = workflow_analysis_prompt(90, 5400, &daily, &[], None);
    let elapsed = started.elapsed().as_micros();
    assert!(!prompt.contains("synthetic-capture-content"));
    assert!(prompt.len() < eager_bytes / 10);
    println!("workflow-index benchmark: eager_fixture_bytes={eager_bytes}, prompt_bytes={}, preparation_us={elapsed}", prompt.len());
}
