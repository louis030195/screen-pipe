// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! Internal-admin historical replay. Uses the normal uploader with a separate
//! cursor; no payload spool, retention change, or customer-storage read access.

use super::*;
use crate::enterprise_policy::{FeedbackSyncMode, SyncStreams};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BackfillStream {
    Frames,
    Audio,
    UiEvents,
    Parsed,
    Memories,
    Activities,
    Feedback,
}

#[derive(Debug, Deserialize)]
pub(super) struct BackfillRequest {
    id: String,
    license_id: String,
    device_id: String,
    start_at: chrono::DateTime<chrono::Utc>,
    end_at: chrono::DateTime<chrono::Utc>,
    streams: Vec<BackfillStream>,
}

impl BackfillRequest {
    fn validate(&self, cfg: &EnterpriseSyncConfig) -> Result<(), EnterpriseSyncError> {
        if uuid::Uuid::parse_str(&self.id).is_err()
            || uuid::Uuid::parse_str(&self.license_id).is_err()
            || self.device_id != cfg.device_id
            || self.start_at >= self.end_at
            || self.end_at > chrono::Utc::now()
            || self.streams.is_empty()
        {
            return Err(EnterpriseSyncError::Configuration(
                "invalid backfill target or range".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn restrict_streams(
        &self,
        policy: SyncStreams,
    ) -> Result<SyncStreams, EnterpriseSyncError> {
        let mut result = SyncStreams {
            frames: false,
            audio: false,
            ui_events: false,
            parsed: false,
            memories: false,
            activities: false,
            snapshots: false,
            feedback: FeedbackSyncMode::Off,
            ..policy
        };
        for stream in &self.streams {
            let allowed = match stream {
                BackfillStream::Frames => {
                    result.frames = policy.frames;
                    policy.frames
                }
                BackfillStream::Audio => {
                    result.audio = policy.audio;
                    policy.audio
                }
                BackfillStream::UiEvents => {
                    result.ui_events = policy.ui_events;
                    policy.ui_events
                }
                BackfillStream::Parsed => {
                    result.parsed = policy.parsed;
                    policy.parsed
                }
                BackfillStream::Memories => {
                    result.memories = policy.memories;
                    policy.memories
                }
                BackfillStream::Activities => {
                    result.activities = policy.activities;
                    policy.activities
                }
                BackfillStream::Feedback => {
                    result.feedback = policy.feedback;
                    policy.feedback != FeedbackSyncMode::Off
                }
            };
            if !allowed {
                return Err(EnterpriseSyncError::Configuration(
                    "backfill stream disabled by current policy".into(),
                ));
            }
        }
        Ok(result)
    }

    /// Both bounds use event time, never upload time; end is exclusive so
    /// adjacent repair windows do not replay their shared boundary twice.
    pub(super) fn retain_in_range<T>(
        &self,
        rows: &mut Vec<T>,
        timestamp: impl Fn(&T) -> &str,
    ) -> Result<(), EnterpriseSyncError> {
        let mut invalid = false;
        rows.retain(
            |row| match chrono::DateTime::parse_from_rfc3339(timestamp(row)) {
                Ok(ts) => ts >= self.start_at && ts < self.end_at,
                Err(_) => {
                    invalid = true;
                    false
                }
            },
        );
        if invalid {
            return Err(EnterpriseSyncError::Configuration(
                "invalid local backfill timestamp".into(),
            ));
        }
        Ok(())
    }

    fn validate_cursor(&self, cursor: &Cursor) -> Result<(), EnterpriseSyncError> {
        for stream in &self.streams {
            let value = match stream {
                BackfillStream::Frames => &cursor.last_frame_ts,
                BackfillStream::Audio => &cursor.last_audio_ts,
                BackfillStream::UiEvents => &cursor.last_ui_ts,
                BackfillStream::Parsed => &cursor.last_parsed_ts,
                BackfillStream::Memories => &cursor.last_memory_ts,
                BackfillStream::Feedback => &cursor.last_feedback_ts,
                BackfillStream::Activities => &cursor.boundary.activity_ts,
            };
            let valid = value
                .as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .is_some_and(|ts| ts >= self.start_at && ts < self.end_at);
            if !valid {
                return Err(EnterpriseSyncError::Configuration(
                    "backfill checkpoint outside requested range".into(),
                ));
            }
        }
        Ok(())
    }

    /// The normal Activity reader is exclusive and returns all remaining rows.
    /// Read inclusively for recovery, then page using its separate tie offset.
    pub(super) fn activity_since(&self, cursor: &Cursor) -> Result<String, EnterpriseSyncError> {
        cursor
            .boundary
            .activity_ts
            .as_deref()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .and_then(|value| value.checked_sub_signed(chrono::Duration::nanoseconds(1)))
            .map(|value| value.to_rfc3339())
            .ok_or_else(|| {
                EnterpriseSyncError::Configuration("invalid activity backfill cursor".into())
            })
    }

    pub(super) fn activity_page(
        &self,
        rows: &mut Vec<ActivityRow>,
        cursor: &Cursor,
    ) -> Result<(), EnterpriseSyncError> {
        // Activity event time is end_at, matching the normal sync representation.
        self.retain_in_range(rows, |row| &row.end_at)?;
        // Stored model output can use different offsets/precision. The live
        // reader sorts strings; recovery pages must sort actual event times.
        rows.sort_by(|left, right| {
            chrono::DateTime::parse_from_rfc3339(&left.end_at)
                .unwrap()
                .cmp(&chrono::DateTime::parse_from_rfc3339(&right.end_at).unwrap())
                .then(left.activity_id.cmp(&right.activity_id))
        });
        rows.drain(..rows.len().min(cursor.boundary.activities as usize));
        rows.truncate(PAGE_LIMIT as usize);
        Ok(())
    }

    pub(super) fn advance_activity_cursor(&self, cursor: &mut Cursor, rows: &[ActivityRow]) {
        // Normalize only checkpoint comparisons, preserving the uploaded rows.
        // activity_page already validated every timestamp before upload.
        let timestamps: Vec<_> = rows
            .iter()
            .map(|row| {
                chrono::DateTime::parse_from_rfc3339(&row.end_at)
                    .unwrap()
                    .with_timezone(&chrono::Utc)
                    .to_rfc3339()
            })
            .collect();
        advance_timestamp_boundary(
            &mut cursor.boundary.activity_ts,
            &mut cursor.boundary.activities,
            &timestamps,
            |timestamp| timestamp,
        );
    }

    fn initial_cursor(&self) -> Cursor {
        let start = Some(self.start_at.to_rfc3339());
        Cursor {
            source_id: None,
            last_frame_ts: start.clone(),
            last_audio_ts: start.clone(),
            last_ui_ts: start.clone(),
            last_memory_ts: start.clone(),
            last_feedback_ts: start.clone(),
            last_parsed_ts: start.clone(),
            boundary: CursorBoundary {
                activity_ts: start,
                ..CursorBoundary::default()
            },
        }
    }
}

#[derive(Deserialize)]
struct Pending {
    request: Option<BackfillRequest>,
}

async fn report(
    cfg: &EnterpriseSyncConfig,
    http: &reqwest::Client,
    url: &str,
    request: &BackfillRequest,
    cursor: &Cursor,
    status: &str,
    failed: bool,
) -> Result<(), EnterpriseSyncError> {
    let response = http.post(url).header("X-License-Key", &cfg.license_key).header("X-Device-Id", &cfg.device_id)
        .json(&serde_json::json!({
            "id": request.id, "status": status, "uploaded_records": cursor.boundary.backfill_records.unwrap_or(0), "last_error": if failed { Some("retrying") } else { None },
            "cursors": { "frames": cursor.last_frame_ts, "audio": cursor.last_audio_ts, "ui_events": cursor.last_ui_ts,
                "parsed": cursor.last_parsed_ts, "memories": cursor.last_memory_ts, "feedback": cursor.last_feedback_ts, "activities": cursor.boundary.activity_ts }
        }))
        .send().await.map_err(|e| EnterpriseSyncError::Ingest(e.to_string()))?
        .error_for_status().map_err(|e| EnterpriseSyncError::Ingest(e.to_string()))?;
    let ack: serde_json::Value = response
        .json()
        .await
        .map_err(|e| EnterpriseSyncError::Ingest(e.to_string()))?;
    if ack.get("accepted").and_then(|v| v.as_bool()) != Some(true) {
        return Err(EnterpriseSyncError::Configuration(
            "backfill no longer active".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    include!("backfill_activity_tests.rs");
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wiremock::{
        matchers::{header, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    fn pending() -> serde_json::Value {
        serde_json::json!({"request": {
            "id": "11111111-1111-1111-1111-111111111111", "license_id": "22222222-2222-2222-2222-222222222222",
            "device_id": "dev-1", "start_at": "2026-01-01T00:00:00Z", "end_at": "2026-01-02T00:00:00Z", "streams": ["frames"]
        }})
    }

    fn request() -> BackfillRequest {
        serde_json::from_value(pending()["request"].clone()).unwrap()
    }

    fn cfg(dir: &tempfile::TempDir, server: &MockServer) -> EnterpriseSyncConfig {
        EnterpriseSyncConfig {
            license_key: "test".into(),
            device_id: "dev-1".into(),
            stable_device_id: None,
            device_label: "friendly".into(),
            ingest_url: format!("{}/api/enterprise/ingest", server.uri()),
            cursor_path: dir.path().join(CURSOR_FILENAME),
            upload_mode: EnterpriseUploadMode::HostedIngest,
            log_dirs: vec![],
        }
    }

    struct Local {
        reads: AtomicUsize,
        fail_ui: bool,
    }
    #[async_trait::async_trait]
    impl LocalApiClient for Local {
        async fn upload_source_id(&self) -> Result<String, EnterpriseSyncError> {
            Ok("33333333-3333-3333-3333-333333333333".into())
        }

        async fn fetch_frames_since(
            &self,
            since: Option<&str>,
            offset: u32,
            limit: u32,
        ) -> Result<Vec<FrameRow>, EnterpriseSyncError> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            let ts = "2026-01-01T01:00:00+00:00";
            // 501 rows at one timestamp exercise the durable tie offset.
            let rows = (0..501).map(|id| FrameRow {
                frame_id: id,
                timestamp: ts.into(),
                app_name: None,
                window_name: None,
                browser_url: None,
                text: Some("synthetic retained history".into()),
            });
            Ok(rows
                .filter(|r| {
                    since.is_none_or(|s| {
                        chrono::DateTime::parse_from_rfc3339(&r.timestamp).unwrap()
                            >= chrono::DateTime::parse_from_rfc3339(s).unwrap()
                    })
                })
                .skip(offset as usize)
                .take(limit as usize)
                .collect())
        }
        async fn fetch_audio_since(
            &self,
            _: Option<&str>,
            _: u32,
            _: u32,
        ) -> Result<Vec<AudioRow>, EnterpriseSyncError> {
            panic!("unselected stream read")
        }
        async fn fetch_ui_events_since(
            &self,
            _: Option<&str>,
            _: u32,
            _: u32,
        ) -> Result<Vec<UiEventRow>, EnterpriseSyncError> {
            if self.fail_ui {
                Err(EnterpriseSyncError::Configuration(
                    "local unavailable".into(),
                ))
            } else {
                Ok(vec![])
            }
        }
    }

    #[test]
    fn backfill_bounds_and_policy_fail_closed() {
        let req = request();
        assert!(req.validate_cursor(&Cursor::default()).is_err());
        assert!(req.validate_cursor(&req.initial_cursor()).is_ok());
        let mut values = vec![
            "2025-12-31T23:59:59Z",
            "2026-01-01T00:00:00Z",
            "2026-01-01T23:59:59Z",
            "2026-01-02T00:00:00Z",
        ];
        req.retain_in_range(&mut values, |v| v).unwrap();
        assert_eq!(values, vec!["2026-01-01T00:00:00Z", "2026-01-01T23:59:59Z"]);
        assert!(req.retain_in_range(&mut vec!["invalid"], |v| v).is_err());
        let mut policy = SyncStreams::default();
        policy.frames = false;
        assert!(req.restrict_streams(policy).is_err());
        let policy = req.restrict_streams(SyncStreams::default()).unwrap();
        assert!(!policy.audio && !policy.snapshots && !policy.activities);
    }

    #[tokio::test]
    async fn backfill_resumes_timestamp_ties_after_lost_ack_without_touching_live_cursor() {
        let _guard = crate::enterprise_policy::sync_streams_test_lock();
        let server = MockServer::start().await;
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = cfg(&dir, &server);
        std::fs::write(&cfg.cursor_path, b"live cursor must remain unchanged").unwrap();
        Mock::given(method("GET"))
            .and(path("/api/enterprise/backfill-requests"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pending()))
            .mount(&server)
            .await;
        let reports = Arc::new(AtomicUsize::new(0));
        let calls = reports.clone();
        Mock::given(method("POST"))
            .and(path("/api/enterprise/backfill-requests"))
            .respond_with(move |_: &wiremock::Request| {
                // Lose the first progress ACK after a successful upload.
                if calls.fetch_add(1, Ordering::SeqCst) == 1 {
                    ResponseTemplate::new(503)
                } else {
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({"accepted": true}))
                }
            })
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/enterprise/ingest"))
            .and(header("x-screenpipe-backfill", "1"))
            .respond_with(ResponseTemplate::new(200))
            .expect(2)
            .mount(&server)
            .await;
        let local = Local {
            reads: AtomicUsize::new(0),
            fail_ui: false,
        };
        let (_, shutdown) = tokio::sync::watch::channel(false);
        let http = enterprise_http_client();
        assert!(fulfill_requests(&cfg, &local, &http, &shutdown)
            .await
            .is_err());
        fulfill_requests(&cfg, &local, &http, &shutdown)
            .await
            .unwrap();
        // Simulate another lost completion ACK / process restart; no re-upload.
        fulfill_requests(&cfg, &local, &http, &shutdown)
            .await
            .unwrap();
        let uploads: Vec<_> = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.url.path() == "/api/enterprise/ingest")
            .collect();
        assert_eq!(
            uploads
                .iter()
                .map(|r| String::from_utf8_lossy(&r.body).lines().count())
                .sum::<usize>(),
            501
        );
        assert_eq!(
            std::fs::read(&cfg.cursor_path).unwrap(),
            b"live cursor must remain unchanged"
        );
        let req = request();
        let checkpoint = Cursor::load(&dir.path().join(format!(
            "enterprise_backfill_{}_{}.json",
            req.license_id, req.id
        )));
        assert_eq!(checkpoint.boundary.backfill_records, Some(501));
    }

    #[tokio::test]
    async fn backfill_failed_optional_read_does_not_complete_or_upload() {
        let _guard = crate::enterprise_policy::sync_streams_test_lock();
        let server = MockServer::start().await;
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = cfg(&dir, &server);
        let mut req = request();
        req.streams.push(BackfillStream::UiEvents);
        let mut cursor = req.initial_cursor();
        let local = Local {
            reads: AtomicUsize::new(0),
            fail_ui: true,
        };
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
        assert!(!cfg.cursor_path.exists());
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn backfill_direct_retries_keep_batch_identity_without_advancing_cursor() {
        let _guard = crate::enterprise_policy::sync_streams_test_lock();
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
        let local = Local {
            reads: AtomicUsize::new(0),
            fail_ui: false,
        };
        let req = request();
        for mode in [
            EnterpriseUploadMode::DirectWriteOnly(direct.clone()),
            EnterpriseUploadMode::DirectReadable(direct),
        ] {
            cfg.upload_mode = mode;
            for _ in 0..2 {
                let mut cursor = req.initial_cursor();
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
                assert_eq!(cursor.boundary.backfill_records, None);
                assert_eq!(cursor.boundary.frames, 0);
                assert!(!cfg.cursor_path.exists());
            }
        }
        let requests = server.received_requests().await.unwrap();
        let manifests: Vec<serde_json::Value> = requests
            .iter()
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect();
        assert_eq!(manifests[0]["batch_id"], manifests[1]["batch_id"]);
        assert_eq!(manifests[2]["batch_id"], manifests[3]["batch_id"]);
        assert_eq!(manifests[0]["record_counts"]["frames"], 500);
    }

    #[tokio::test]
    async fn backfill_wrong_device_and_cancelled_request_never_read_local_data() {
        let server = MockServer::start().await;
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = cfg(&dir, &server);
        let mut req = request();
        req.device_id = "other".into();
        assert!(req.validate(&cfg).is_err());
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pending()))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"accepted": false})),
            )
            .mount(&server)
            .await;
        let local = Local {
            reads: AtomicUsize::new(0),
            fail_ui: false,
        };
        let (_, shutdown) = tokio::sync::watch::channel(false);
        assert!(
            fulfill_requests(&cfg, &local, &enterprise_http_client(), &shutdown)
                .await
                .is_err()
        );
        assert_eq!(local.reads.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn backfill_stable_identity_preserves_range_and_rejects_database_switch() {
        let _guard = crate::enterprise_policy::sync_streams_test_lock();
        let server = MockServer::start().await;
        let dir = tempfile::TempDir::new().unwrap();
        let mut cfg = cfg(&dir, &server);
        cfg.device_id = "sp_device_v1_8bf702412437cb166a226682c7505808".into();
        cfg.cursor_path = dir.path().join("recovery.json");
        let source = "33333333-3333-3333-3333-333333333333";
        Mock::given(method("POST"))
            .and(path("/api/enterprise/device-sources"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "version": 1, "device_id": cfg.device_id, "source_id": source
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/enterprise/ingest"))
            .and(header(
                "X-Screenpipe-Stable-Device-Id",
                cfg.device_id.as_str(),
            ))
            .and(header("X-Screenpipe-Backfill", "1"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let mut request = request();
        request.device_id = cfg.device_id.clone();
        let mut cursor = request.initial_cursor();
        let local = Local {
            reads: AtomicUsize::new(0),
            fail_ui: false,
        };
        let page = run_one_sync_inner(
            &cfg,
            &mut cursor,
            &local,
            &enterprise_http_client(),
            false,
            Some(&request),
        )
        .await
        .unwrap();
        assert_eq!(page.frames, 500);
        assert_eq!(cursor.source_id.as_deref(), Some(source));
        assert!(cursor
            .last_frame_ts
            .as_deref()
            .unwrap()
            .starts_with("2026-01-01"));
        cursor.source_id = Some("different-database".into());
        assert!(run_one_sync_inner(
            &cfg,
            &mut cursor,
            &local,
            &enterprise_http_client(),
            false,
            Some(&request)
        )
        .await
        .is_err());
        assert_eq!(local.reads.load(Ordering::SeqCst), 1);
    }
}

pub(super) async fn fulfill_requests(
    cfg: &EnterpriseSyncConfig,
    local: &dyn LocalApiClient,
    http: &reqwest::Client,
    shutdown: &tokio::sync::watch::Receiver<bool>,
) -> Result<(), EnterpriseSyncError> {
    let Some(base) = control_plane_base(&cfg.ingest_url) else {
        return Ok(());
    };
    let url = format!("{base}/api/enterprise/backfill-requests");
    // At most 10 pages after live sync; poll each page so cancellation and
    // server policy changes take effect before the next local read.
    for _ in 0..10 {
        if *shutdown.borrow() {
            break;
        }
        let response = http
            .get(&url)
            .header("X-License-Key", &cfg.license_key)
            .header("X-Device-Id", &cfg.device_id)
            .send()
            .await
            .map_err(|e| EnterpriseSyncError::Ingest(e.to_string()))?;
        // Server-first deployment is optional: older control planes have no
        // request endpoint and normal sync must keep working.
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            break;
        }
        let pending: Pending = response
            .error_for_status()
            .map_err(|e| EnterpriseSyncError::Ingest(e.to_string()))?
            .json()
            .await
            .map_err(|e| EnterpriseSyncError::Ingest(e.to_string()))?;
        let Some(request) = pending.request else {
            break;
        };
        request.validate(cfg)?;
        let mut replay_cfg = cfg.clone();
        replay_cfg.cursor_path = cfg.cursor_path.with_file_name(format!(
            "enterprise_backfill_{}_{}.json",
            request.license_id, request.id
        ));
        // Stable label keeps retries deterministic even after a hostname change.
        replay_cfg.device_label = cfg.device_id.clone();
        let loaded = match std::fs::read(&replay_cfg.cursor_path) {
            Ok(bytes) => serde_json::from_slice::<Cursor>(&bytes).map_err(|_| {
                EnterpriseSyncError::Configuration("backfill checkpoint unreadable".into())
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(request.initial_cursor()),
            Err(e) => Err(e.into()),
        }
        .and_then(|cursor| {
            request.validate_cursor(&cursor)?;
            Ok(cursor)
        });
        let mut cursor = match loaded {
            Ok(cursor) => cursor,
            Err(error) => {
                let _ = report(
                    cfg,
                    http,
                    &url,
                    &request,
                    &request.initial_cursor(),
                    "running",
                    true,
                )
                .await;
                return Err(error);
            }
        };
        report(cfg, http, &url, &request, &cursor, "running", false).await?;
        match run_one_sync_inner(&replay_cfg, &mut cursor, local, http, false, Some(&request)).await
        {
            Ok(page) => {
                let complete = !page.may_have_more() && page.activities < PAGE_LIMIT as usize;
                // Persist even an empty result; lost completion ACKs then retry
                // from the same boundary without re-uploading prior pages.
                cursor.save(&replay_cfg.cursor_path)?;
                report(
                    cfg,
                    http,
                    &url,
                    &request,
                    &cursor,
                    if complete { "completed" } else { "running" },
                    false,
                )
                .await?;
                if complete {
                    break;
                }
            }
            Err(error) => {
                let _ = report(cfg, http, &url, &request, &cursor, "running", true).await;
                return Err(error);
            }
        }
    }
    Ok(())
}
