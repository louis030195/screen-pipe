// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! Resolve final citations through the recorder. The model plans retrieval;
//! this boundary verifies source identity and never trusts generated excerpts.
use super::*;

const MAX_SOURCE_READS: usize = 240;

fn references(
    value: &Value,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<(DateTime<Utc>, String)>, String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for workflow in value
        .get("workflows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let entries = workflow
            .get("stages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|stage| {
                stage
                    .get("evidence")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .chain(
                workflow
                    .get("evidence")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten(),
            );
        for entry in entries {
            let Some(at) = entry
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.with_timezone(&Utc))
            else {
                continue;
            };
            let Some(app) = entry
                .get("app")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            if at < start || at >= end || !seen.insert((at, app.to_lowercase())) {
                continue;
            }
            result.push((at, app.to_string()));
            if result.len() > MAX_SOURCE_READS {
                return Err("The map cites too many sources to validate in one run. Your previous map was kept.".into());
            }
        }
    }
    Ok(result)
}

fn source_points(payload: &Value, at: DateTime<Utc>, app: &str) -> Vec<EvidencePoint> {
    payload
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let source = match row.get("type")?.as_str()? {
                "OCR" | "UI" | "Accessibility" => "screen",
                "Parsed" => "parsed",
                "Audio" => "audio",
                _ => return None,
            };
            let content = row.get("content")?;
            let timestamp = DateTime::parse_from_rfc3339(content.get("timestamp")?.as_str()?)
                .ok()?
                .with_timezone(&Utc);
            let actual_app = content
                .get("app_name")
                .and_then(Value::as_str)
                .unwrap_or("Conversation");
            if timestamp != at || !actual_app.eq_ignore_ascii_case(app) {
                return None;
            }
            let detail = content
                .get(if source == "audio" {
                    "transcription"
                } else {
                    "text"
                })?
                .as_str()?
                .trim();
            if detail.is_empty() {
                return None;
            }
            Some(EvidencePoint {
                timestamp,
                app: actual_app.to_string(),
                detail: detail.chars().take(32_000).collect(),
                source: source.to_string(),
                speaker: None,
            })
        })
        .collect()
}

pub(super) async fn resolve_references(
    endpoint: &RecorderEndpoint,
    value: &Value,
    mut catalog: EvidenceCatalog,
    activity: &[Value],
) -> Result<(EvidenceCatalog, usize), String> {
    let bounds = |key: &str| {
        activity
            .iter()
            .filter_map(|day| day.get(key).and_then(Value::as_str))
            .filter_map(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&Utc))
            .collect::<Vec<_>>()
    };
    let start = bounds("start")
        .into_iter()
        .min()
        .ok_or("Missing capture period")?;
    let end = bounds("end")
        .into_iter()
        .max()
        .ok_or("Missing capture period")?;
    let refs = references(value, start, end)?;
    let reads = refs.len();
    let client = reqwest::Client::new();
    let results = stream::iter(refs)
        .map(|(at, app)| {
            let client = &client;
            async move {
                let mut url = reqwest::Url::parse(&format!("{}/search", endpoint.base_url))
                    .map_err(|e| e.to_string())?;
                url.query_pairs_mut()
                    .append_pair("start_time", &at.to_rfc3339())
                    .append_pair(
                        "end_time",
                        &(at + ChronoDuration::milliseconds(1)).to_rfc3339(),
                    )
                    .append_pair("app_name", &app)
                    .append_pair("content_type", "all")
                    .append_pair("include_frames", "false")
                    .append_pair("limit", "100");
                let response =
                    apply_auth(endpoint, client.get(url).timeout(Duration::from_secs(20)))
                        .send()
                        .await
                        .map_err(|_| "Source verification could not reach the recorder")?
                        .error_for_status()
                        .map_err(|_| "Source verification failed; your previous map was kept")?;
                let payload = response
                    .json::<Value>()
                    .await
                    .map_err(|_| "Invalid recorder source response")?;
                if !payload.get("data").is_some_and(Value::is_array) {
                    return Err("Incomplete recorder source response".to_string());
                }
                if payload
                    .pointer("/pagination/total")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    > 100
                {
                    return Err(
                        "Source verification response was incomplete; your previous map was kept"
                            .to_string(),
                    );
                }
                Ok((at, app.clone(), source_points(&payload, at, &app)))
            }
        })
        .buffered(HISTORY_QUERY_CONCURRENCY)
        .collect::<Vec<Result<_, String>>>()
        .await;
    // New source reads replace sampled text at the same identity. Missing or
    // revoked sources cannot survive merely because they were in the index.
    for result in results {
        let (at, app, points) = result?;
        catalog
            .points
            .retain(|p| p.timestamp != at || !p.app.eq_ignore_ascii_case(&app));
        for point in points {
            catalog.remember_app(&point.app);
            catalog.points.push(point);
        }
    }
    catalog
        .points
        .retain(|p| p.timestamp >= start && p.timestamp < end);
    Ok((catalog, reads))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn recorder_verification_uses_auth_exact_bounds_and_original_text() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let size = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..size]);
            assert!(request
                .to_lowercase()
                .contains("authorization: bearer fixture-key"));
            let path = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let url = reqwest::Url::parse(&format!("http://localhost{path}")).unwrap();
            let params: HashMap<_, _> = url.query_pairs().collect();
            assert_eq!(url.path(), "/search");
            assert_eq!(params["include_frames"], "false");
            let start = DateTime::parse_from_rfc3339(&params["start_time"]).unwrap();
            let end = DateTime::parse_from_rfc3339(&params["end_time"]).unwrap();
            assert_eq!((end - start).num_milliseconds(), 1);
            let body = json!({"data":[{"type":"Parsed","content":{"timestamp":start,"app_name":"Editor","text":"Actual source retrieved from recorder"}}],"pagination":{"total":1}}).to_string();
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let endpoint = RecorderEndpoint {
            source: "screenpipe",
            base_url: format!("http://{address}"),
            api_key: Some("fixture-key".to_string()),
            health: json!({}),
        };
        let activity = vec![json!({"start":"2026-08-01T00:00:00Z","end":"2026-08-03T00:00:00Z"})];
        let value = json!({"workflows":[{"evidence":[{"timestamp":"2026-08-01T10:00:00Z","app":"Editor","detail":"Invented result"}]}]});
        let (catalog, reads) =
            resolve_references(&endpoint, &value, EvidenceCatalog::default(), &activity)
                .await
                .unwrap();
        server.await.unwrap();
        assert_eq!(reads, 1);
        assert_eq!(
            catalog.points[0].detail,
            "Actual source retrieved from recorder"
        );
        // A transport failure must not become an empty successful catalog.
        assert!(resolve_references(&endpoint, &value, catalog, &activity)
            .await
            .is_err());
    }

    #[test]
    fn source_read_budget_fails_explicitly_instead_of_silently_truncating() {
        let start = DateTime::parse_from_rfc3339("2026-08-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let refs: Vec<_> = (0..=MAX_SOURCE_READS)
            .map(|i| json!({"timestamp":start + ChronoDuration::seconds(i as i64),"app":"Editor"}))
            .collect();
        assert!(references(
            &json!({"workflows":[{"evidence":refs}]}),
            start,
            start + ChronoDuration::days(1)
        )
        .is_err());
    }

    #[test]
    fn resolves_only_exact_original_sources_not_nearby_or_generated_claims() {
        let at = DateTime::parse_from_rfc3339("2026-08-01T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let payload = json!({"data":[
            {"type":"Parsed","content":{"timestamp":at,"app_name":"Editor","text":"Original draft was opened"}},
            {"type":"OCR","content":{"timestamp":at + ChronoDuration::seconds(1),"app_name":"Editor","text":"Different moment"}},
            {"type":"OCR","content":{"timestamp":at,"app_name":"Email","text":"Different app"}},
            {"type":"Memory","content":{"timestamp":at,"app_name":"Editor","text":"Generated summary"}}
        ]});
        let points = source_points(&payload, at, "editor");
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].detail, "Original draft was opened");
        assert_eq!(points[0].source, "parsed");
    }
    #[test]
    fn deduplicates_citations_and_excludes_out_of_period_sources() {
        let start = DateTime::parse_from_rfc3339("2026-08-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let value = json!({"workflows":[{"evidence":[
            {"timestamp":start,"app":"Editor"},{"timestamp":start,"app":"editor"},
            {"timestamp":start - ChronoDuration::days(1),"app":"Editor"},
            {"timestamp":"invalid","app":"Editor"}
        ]}]});
        assert_eq!(
            references(&value, start, start + ChronoDuration::days(1))
                .unwrap()
                .len(),
            1
        );
    }
}
