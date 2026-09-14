// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

struct ActivityPolicy;
impl ActivityPolicy {
    fn enable() -> Self {
        crate::enterprise_policy::set_activity_sync_enabled(true);
        Self
    }
}
impl Drop for ActivityPolicy {
    fn drop(&mut self) {
        crate::enterprise_policy::set_activity_sync_enabled(false);
    }
}

struct Activities {
    rows: Vec<ActivityRow>,
    reads: AtomicUsize,
    fail: bool,
}
fn activity(id: &str, end: &str) -> ActivityRow {
    ActivityRow {
        activity_id: id.into(),
        activity_kind: "work".into(),
        meeting_id: None,
        timestamp: end.into(),
        start_at: "2025-12-31T23:55:00Z".into(),
        end_at: end.into(),
        title: "Synthetic activity".into(),
        summary: "Retained generated summary".into(),
        evidence: vec![],
    }
}
impl Activities {
    fn new() -> Self {
        let mut rows = vec![activity("before", "2025-12-31T23:59:59Z")];
        // More than one page ending exactly at the inclusive start.
        for id in 0..501 {
            rows.push(activity(&format!("tie-{id:04}"), "2026-01-01T00:00:00Z"));
        }
        rows.push(activity("inside", "2026-01-01T01:00:00Z"));
        rows.push(activity("end", "2026-01-02T00:00:00Z"));
        Self {
            rows,
            reads: AtomicUsize::new(0),
            fail: false,
        }
    }
}
#[async_trait::async_trait]
impl LocalApiClient for Activities {
    async fn fetch_frames_since(
        &self,
        _: Option<&str>,
        _: u32,
        _: u32,
    ) -> Result<Vec<FrameRow>, EnterpriseSyncError> {
        Ok(vec![])
    }
    async fn fetch_audio_since(
        &self,
        _: Option<&str>,
        _: u32,
        _: u32,
    ) -> Result<Vec<AudioRow>, EnterpriseSyncError> {
        Ok(vec![])
    }
    async fn fetch_activities_since(
        &self,
        since: Option<&str>,
    ) -> Result<Vec<ActivityRow>, EnterpriseSyncError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(EnterpriseSyncError::LocalApi(
                "activity store unavailable".into(),
            ));
        }
        // Match the real reader's exclusive lower bound and unpaged result.
        let since = chrono::DateTime::parse_from_rfc3339(since.unwrap()).unwrap();
        Ok(self
            .rows
            .iter()
            .filter(|row| chrono::DateTime::parse_from_rfc3339(&row.end_at).unwrap() > since)
            .cloned()
            .collect())
    }
}
fn activity_request() -> BackfillRequest {
    let mut req = request();
    req.streams = vec![BackfillStream::Activities];
    req
}
fn activity_pending() -> serde_json::Value {
    let mut value = pending();
    value["request"]["streams"] = serde_json::json!(["activities"]);
    value
}

#[test]
fn activity_policy_and_checkpoint_bounds_fail_closed() {
    let req = activity_request();
    assert!(req.restrict_streams(SyncStreams::default()).is_err());
    let policy = req
        .restrict_streams(SyncStreams {
            activities: true,
            ..SyncStreams::default()
        })
        .unwrap();
    assert!(policy.activities);
    assert!(!policy.frames && !policy.audio && !policy.snapshots);
    let mut cursor = req.initial_cursor();
    assert!(req.validate_cursor(&cursor).is_ok());
    for ts in [
        None,
        Some("invalid"),
        Some("2025-12-31T23:59:59Z"),
        Some("2026-01-02T00:00:00Z"),
    ] {
        cursor.boundary.activity_ts = ts.map(str::to_string);
        assert!(req.validate_cursor(&cursor).is_err());
    }
    let mut old = serde_json::to_value(req.initial_cursor()).unwrap();
    old["boundary"]
        .as_object_mut()
        .unwrap()
        .remove("activities");
    assert_eq!(
        serde_json::from_value::<Cursor>(old)
            .unwrap()
            .boundary
            .activities,
        0
    );
}

#[tokio::test]
async fn activities_retry_and_resume_ties_with_inclusive_start_and_exclusive_end() {
    let _guard = crate::enterprise_policy::sync_streams_test_lock();
    let _policy = ActivityPolicy::enable();
    let server = MockServer::start().await;
    let dir = tempfile::TempDir::new().unwrap();
    let cfg = cfg(&dir, &server);
    std::fs::write(&cfg.cursor_path, b"live cursor unchanged").unwrap();
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(activity_pending()))
        .mount(&server)
        .await;
    let reports = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST"))
        .and(path("/api/enterprise/backfill-requests"))
        .respond_with(move |r: &wiremock::Request| {
            let body: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
            // Lose the first progress ACK after a successful 500-row page.
            if body["uploaded_records"] == 500 && reports.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(503)
            } else {
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"accepted": true}))
            }
        })
        .mount(&server)
        .await;
    let uploads = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST"))
        .and(path("/api/enterprise/ingest"))
        .and(header("x-screenpipe-backfill", "1"))
        .respond_with(move |_: &wiremock::Request| {
            // An upload may have landed even when its response was lost.
            ResponseTemplate::new(if uploads.fetch_add(1, Ordering::SeqCst) == 0 {
                503
            } else {
                200
            })
        })
        .expect(3)
        .mount(&server)
        .await;
    let local = Activities::new();
    let source = serde_json::to_value(&local.rows).unwrap();
    let (_, shutdown) = tokio::sync::watch::channel(false);
    let http = enterprise_http_client();
    assert!(fulfill_requests(&cfg, &local, &http, &shutdown)
        .await
        .is_err());
    assert!(fulfill_requests(&cfg, &local, &http, &shutdown)
        .await
        .is_err());
    fulfill_requests(&cfg, &local, &http, &shutdown)
        .await
        .unwrap();
    // Restart after a lost completion ACK must not replay saved Activities.
    fulfill_requests(&cfg, &local, &http, &shutdown)
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let uploads: Vec<_> = requests
        .iter()
        .filter(|r| r.url.path() == "/api/enterprise/ingest")
        .collect();
    assert_eq!(uploads[0].body, uploads[1].body);
    let rows: Vec<serde_json::Value> = uploads[1..]
        .iter()
        .flat_map(|r| {
            std::str::from_utf8(&r.body)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
        })
        .collect();
    assert_eq!(rows.len(), 502);
    assert!(rows.iter().all(|r| r["kind"] == "activity"));
    let ids: std::collections::HashSet<_> = rows
        .iter()
        .map(|r| r["activity_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 502);
    assert!(!ids.contains("before") && !ids.contains("end"));
    assert!(ids.contains("tie-0000") && ids.contains("tie-0500") && ids.contains("inside"));
    let last: serde_json::Value = serde_json::from_slice(&requests.last().unwrap().body).unwrap();
    assert_eq!(last["status"], "completed");
    assert_eq!(last["uploaded_records"], 502);
    assert_eq!(last["cursors"]["activities"], "2026-01-01T01:00:00+00:00");
    assert_eq!(
        std::fs::read(&cfg.cursor_path).unwrap(),
        b"live cursor unchanged"
    );
    assert_eq!(serde_json::to_value(&local.rows).unwrap(), source);
}

#[tokio::test]
async fn activity_failures_retry_but_live_sync_remains_best_effort() {
    let _guard = crate::enterprise_policy::sync_streams_test_lock();
    let server = MockServer::start().await;
    let dir = tempfile::TempDir::new().unwrap();
    let cfg = cfg(&dir, &server);
    let mut cursor = activity_request().initial_cursor();
    let local = Activities {
        fail: true,
        ..Activities::new()
    };
    let req = activity_request();
    let http = enterprise_http_client();
    assert!(
        run_one_sync_inner(&cfg, &mut cursor, &local, &http, false, Some(&req))
            .await
            .is_err()
    );
    assert_eq!(local.reads.load(Ordering::SeqCst), 0); // Policy disabled.
    let _policy = ActivityPolicy::enable();
    assert!(
        run_one_sync_inner(&cfg, &mut cursor, &local, &http, false, Some(&req))
            .await
            .is_err()
    );
    assert_eq!(cursor.boundary.backfill_records, None);
    assert!(!cfg.cursor_path.exists());
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(
        run_one_sync_inner(&cfg, &mut cursor, &local, &http, false, None)
            .await
            .is_ok()
    );
    assert_eq!(local.reads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn activity_direct_retries_preserve_manifest_and_checkpoint() {
    let _guard = crate::enterprise_policy::sync_streams_test_lock();
    let _policy = ActivityPolicy::enable();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/ticket"))
        .respond_with(ResponseTemplate::new(503))
        .expect(4)
        .mount(&server)
        .await;
    let dir = tempfile::TempDir::new().unwrap();
    let mut cfg = cfg(&dir, &server);
    let direct = enterprise_upload::DirectUploadConfig {
        ticket_url: format!("{}/ticket", server.uri()),
        complete_url: format!("{}/complete", server.uri()),
        pinned_hosts: vec![],
    };
    let local = Activities::new();
    let req = activity_request();
    for mode in [
        EnterpriseUploadMode::DirectWriteOnly(direct.clone()),
        EnterpriseUploadMode::DirectReadable(direct),
    ] {
        cfg.upload_mode = mode;
        let mut cursor = req.initial_cursor();
        let initial = serde_json::to_value(&cursor).unwrap();
        for _ in 0..2 {
            assert!(run_one_sync_inner(
                &cfg,
                &mut cursor,
                &local,
                &enterprise_http_client(),
                false,
                Some(&req)
            )
            .await
            .is_err());
            assert_eq!(serde_json::to_value(&cursor).unwrap(), initial);
            assert!(!cfg.cursor_path.exists());
        }
    }
    let requests = server.received_requests().await.unwrap();
    let manifests: Vec<serde_json::Value> = requests
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    for pair in manifests.chunks(2) {
        assert_eq!(pair[0]["batch_id"], pair[1]["batch_id"]);
        assert_eq!(pair[0]["record_counts"]["activities"], 500);
        assert_eq!(pair[0]["record_counts"]["frames"], 0);
    }
}

#[tokio::test]
async fn activity_adjacent_window_includes_previous_exclusive_end() {
    let _guard = crate::enterprise_policy::sync_streams_test_lock();
    let _policy = ActivityPolicy::enable();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/enterprise/ingest"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::TempDir::new().unwrap();
    let cfg = cfg(&dir, &server);
    let local = Activities::new();
    let mut req = activity_request();
    req.start_at = req.end_at;
    req.end_at += chrono::Duration::days(1);
    let mut cursor = req.initial_cursor();
    let page = run_one_sync_inner(
        &cfg,
        &mut cursor,
        &local,
        &enterprise_http_client(),
        false,
        Some(&req),
    )
    .await
    .unwrap();
    assert_eq!(page.activities, 1);
    let uploads = server.received_requests().await.unwrap();
    let row: serde_json::Value = serde_json::from_slice(&uploads[0].body).unwrap();
    assert_eq!(row["activity_id"], "end");
    assert_eq!(row["timestamp"], row["end_at"]);
}

#[tokio::test]
async fn activity_cancellation_before_next_page_prevents_local_read() {
    let _guard = crate::enterprise_policy::sync_streams_test_lock();
    let _policy = ActivityPolicy::enable();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(activity_pending()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/enterprise/backfill-requests"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"accepted": false})),
        )
        .mount(&server)
        .await;
    let dir = tempfile::TempDir::new().unwrap();
    let cfg = cfg(&dir, &server);
    let local = Activities::new();
    let (_, shutdown) = tokio::sync::watch::channel(false);
    assert!(
        fulfill_requests(&cfg, &local, &enterprise_http_client(), &shutdown)
            .await
            .is_err()
    );
    assert_eq!(local.reads.load(Ordering::SeqCst), 0);
    assert!(!cfg.cursor_path.exists());
}

#[test]
fn activity_pages_order_actual_times_and_resume_equivalent_timestamp_ties() {
    let req = activity_request();
    let mut cursor = req.initial_cursor();
    let mut source: Vec<_> = (0..501)
        .map(|id| {
            activity(
                &format!("tie-{id:04}"),
                if id % 2 == 0 {
                    "2025-12-31T16:00:00-08:00"
                } else {
                    "2026-01-01T00:00:00.000Z"
                },
            )
        })
        .collect();
    source.push(activity("later", "2025-12-31T20:00:00-08:00"));
    source.sort_by(|left, right| {
        left.end_at
            .cmp(&right.end_at)
            .then(left.activity_id.cmp(&right.activity_id))
    });
    let mut first = source.clone();
    req.activity_page(&mut first, &cursor).unwrap();
    assert_eq!(first.len(), 500);
    assert_eq!(first.last().unwrap().activity_id, "tie-0499");
    req.advance_activity_cursor(&mut cursor, &first);
    assert_eq!(cursor.boundary.activities, 500);
    assert_eq!(
        cursor.boundary.activity_ts.as_deref(),
        Some("2026-01-01T00:00:00+00:00")
    );
    let mut second = source;
    req.activity_page(&mut second, &cursor).unwrap();
    assert_eq!(
        second
            .iter()
            .map(|row| row.activity_id.as_str())
            .collect::<Vec<_>>(),
        vec!["tie-0500", "later"]
    );
    req.advance_activity_cursor(&mut cursor, &second);
    assert_eq!(cursor.boundary.activities, 1);
    assert_eq!(
        cursor.boundary.activity_ts.as_deref(),
        Some("2026-01-01T04:00:00+00:00")
    );
    // Wire timestamps retain their original representation.
    assert_eq!(second[1].end_at, "2025-12-31T20:00:00-08:00");
}
