// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::Worker;
use crate::{RedactError, RedactionOutput, Redactor};
use screenpipe_db::storage::{PrivacyPolicy, Projection, StorageMode};
use sqlx::Row;
use std::sync::Arc;

struct CompleteRedactor<'a>(&'a dyn Redactor);

#[async_trait::async_trait]
impl Redactor for CompleteRedactor<'_> {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn version(&self) -> u32 {
        self.0.version()
    }
    async fn redact_batch(&self, texts: &[String]) -> Result<Vec<RedactionOutput>, RedactError> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            results.push(self.0.redact_complete(text).await?);
        }
        Ok(results)
    }
}

impl Worker {
    pub fn with_frame_storage(mut self, db: Arc<screenpipe_db::DatabaseManager>) -> Self {
        if db.storage_mode() == StorageMode::HybridParquetV1 {
            self.frame_storage = Some(db);
        }
        self
    }

    pub async fn process_hybrid_frames(&self, limit: u32) -> Result<u32, anyhow::Error> {
        let Some(db) = &self.frame_storage else {
            return Ok(0);
        };
        let complete = CompleteRedactor(self.redactor.as_ref());
        let columns = self.cfg.columns;
        let required = 1
            | 8
            | if columns.accessibility_text { 2 } else { 0 }
            | if columns.accessibility_tree { 4 } else { 0 }
            | if columns.window_name { 16 } else { 0 }
            | if columns.browser_url { 32 } else { 0 };
        let identity = format!("{}:{columns:?}", self.redactor.policy_identity());
        db.set_frame_privacy_policy(&PrivacyPolicy {
            identity: identity.clone(),
            required_surfaces: required,
        })
        .await?;
        let ids:Vec<i64>=sqlx::query_scalar("SELECT p.frame_id FROM frame_payloads p CROSS JOIN storage_metadata m WHERE (p.completed_surfaces & m.required_surfaces)!=m.required_surfaces AND (p.retry_at IS NULL OR p.retry_at<=strftime('%s','now')) ORDER BY p.frame_id DESC LIMIT ?")
            .bind(limit).fetch_all(&db.pool).await?;
        let mut count = 0;
        for id in ids {
            let token = db.storage_read_token().await?;
            let Some(mut payload) = db.frame_payloads(&[id], Projection::All).await?.remove(&id)
            else {
                continue;
            };
            let row = sqlx::query("SELECT window_name,browser_url FROM frames WHERE id=?")
                .bind(id)
                .fetch_one(&db.pool)
                .await?;
            let mut window: Option<String> = row.try_get("window_name")?;
            let mut url: Option<String> = row.try_get("browser_url")?;
            {
                let _admission = token.admit(&db.pool).await?;
            }
            drop(token);
            let detection = async {
                if let Some(text) = payload.full_text.as_mut().filter(|s| !s.is_empty()) {
                    *text = complete.redact(text).await?.redacted;
                }
                if columns.accessibility_text {
                    if let Some(text) = payload
                        .accessibility_text
                        .as_mut()
                        .filter(|s| !s.is_empty())
                    {
                        *text = complete.redact(text).await?.redacted;
                    }
                }
                if columns.accessibility_tree {
                    if let Some(json) = payload
                        .accessibility_tree_json
                        .as_mut()
                        .filter(|s| !s.is_empty())
                    {
                        if let Some(clean) =
                            crate::tree_json::redact_tree_json_with_redactor_fields(
                                json,
                                &complete,
                                &columns.a11y_json_fields(),
                            )
                            .await?
                        {
                            *json = clean;
                        }
                    }
                }
                if let Some(json) = payload.text_json.as_mut().filter(|s| !s.is_empty()) {
                    if let Some(clean) =
                        crate::ocr_json::redact_ocr_text_json_with_redactor(json, &complete).await?
                    {
                        *json = clean;
                    }
                }
                if columns.window_name {
                    if let Some(text) = window.as_mut().filter(|s| !s.is_empty()) {
                        *text = complete.redact(text).await?.redacted;
                    }
                }
                if columns.browser_url {
                    if let Some(text) = url.as_mut().filter(|s| !s.is_empty()) {
                        *text = complete.redact(text).await?.redacted;
                    }
                }
                Ok::<_, anyhow::Error>(())
            }
            .await;
            match detection {
                Ok(()) => {
                    if db
                        .replace_frame_payload(
                            &payload,
                            &identity,
                            required,
                            window.as_deref(),
                            url.as_deref(),
                        )
                        .await?
                    {
                        count += 1;
                    }
                }
                Err(error) => {
                    // The receipt contains an error class, never captured text
                    // or detector output. Retry stays scoped to this generation.
                    let writer = self.writer.lock().await?;
                    sqlx::query("UPDATE frame_payloads SET attempts=attempts+1,retry_at=strftime('%s','now')+MIN(3600,30*(attempts+1)),last_error='configured surface processing failed' WHERE frame_id=? AND generation=?")
                        .bind(id).bind(payload.generation).execute(writer.pool()).await?;
                    let mut status = self.status.lock().await;
                    status.last_error =
                        Some("frame surface processing failed; retry scheduled".into());
                    tracing::debug!(frame_id=id,error_type=%std::any::type_name_of_val(&error),"frame privacy processing deferred");
                }
            }
        }
        if count > 0 {
            let mut status = self.status.lock().await;
            status.redacted_total += count as u64;
            status.last_redacted_at = Some(chrono::Utc::now());
        }
        Ok(count)
    }
}
