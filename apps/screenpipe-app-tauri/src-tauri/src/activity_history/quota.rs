// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! Persisted backoff for account allowances, not transient or per-turn limits.
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct QuotaPause {
    pub context: String,
    pub code: String,
    pub retry_at: Option<DateTime<Utc>>,
}

impl QuotaPause {
    pub fn from_error(error: &str, context: String, now: DateTime<Utc>) -> Option<Self> {
        let code = [
            "free_chat_limit_exceeded",
            "hosted_ai_allowance_exceeded",
            "trial_cost_limit_exceeded",
        ]
        .into_iter()
        .find(|code| error.contains(code))?;
        // Error adapters can wrap JSON inside escaped strings or SSE frames.
        // Only a whitelisted code and reset timestamp survive into the store.
        let retry_at = reset_time(error).filter(|at| *at > now).or_else(|| {
            // A trial allowance has no scheduled reset. Other responses
            // without a reset hint get one bounded recheck per 24 hours.
            (code != "trial_cost_limit_exceeded").then(|| now + Duration::hours(24))
        });
        Some(Self {
            context,
            code: code.to_string(),
            retry_at,
        })
    }

    pub fn blocks(&self, context: &str, now: DateTime<Utc>) -> bool {
        self.context == context && self.retry_at.is_none_or(|at| now < at)
    }
}

fn reset_time(error: &str) -> Option<DateTime<Utc>> {
    let mut normalized = error.to_string();
    for _ in 0..4 {
        if let Some(start) = normalized.find('{') {
            if let Some(Ok(value)) = serde_json::Deserializer::from_str(&normalized[start..])
                .into_iter::<Value>()
                .next()
            {
                if let Some(at) = reset_in_value(&value, 0) {
                    return Some(at);
                }
            }
        }
        let unescaped = normalized.replace("\\\"", "\"");
        if unescaped == normalized {
            break;
        }
        normalized = unescaped;
    }
    None
}

fn reset_in_value(value: &Value, depth: usize) -> Option<DateTime<Utc>> {
    if depth > 8 {
        return None;
    }
    match value {
        Value::Object(fields) => {
            for key in ["resets_at", "reset_at"] {
                if let Some(at) = fields.get(key) {
                    if let Some(text) = at.as_str() {
                        if let Ok(parsed) = DateTime::parse_from_rfc3339(text) {
                            return Some(parsed.with_timezone(&Utc));
                        }
                    }
                    if let Some(seconds) = at.as_i64() {
                        if let Some(parsed) = DateTime::from_timestamp(seconds, 0) {
                            return Some(parsed);
                        }
                    }
                }
            }
            fields.values().find_map(|v| reset_in_value(v, depth + 1))
        }
        Value::String(text) => serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|v| reset_in_value(&v, depth + 1)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn now() -> DateTime<Utc> {
        "2026-09-15T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn daily_limit_survives_restart_and_blocks_normal_scheduler_ticks() {
        let pause = QuotaPause::from_error(
            r#"429 {"error":"free_chat_limit_exceeded"}"#,
            "preset-a".into(),
            now(),
        )
        .unwrap();
        let restored: QuotaPause =
            serde_json::from_str(&serde_json::to_string(&pause).unwrap()).unwrap();
        assert!(restored.blocks("preset-a", now() + Duration::minutes(15)));
        assert!(restored.blocks("preset-a", now() + Duration::hours(23)));
        assert!(!restored.blocks("preset-a", now() + Duration::hours(24)));
        assert!(!restored.blocks("new-provider-or-entitlement", now()));
    }

    #[test]
    fn honors_reset_in_sse_and_escaped_adapter_payloads() {
        for message in [
            "429 data: {\"error\":{\"code\":\"hosted_ai_allowance_exceeded\"},\"resets_at\":\"2026-09-15T13:00:00Z\"}\n\ndata: [DONE]",
            r#"429 "{\"error\":\"free_chat_limit_exceeded\",\"resets_at\":\"2026-09-15T13:00:00Z\"}""#,
        ] {
            let pause = QuotaPause::from_error(message, "a".into(), now()).unwrap();
            assert_eq!(pause.retry_at, Some(now() + Duration::hours(1)));
        }
    }

    #[test]
    fn trial_waits_for_account_or_provider_change_without_periodic_retries() {
        let pause = QuotaPause::from_error("trial_cost_limit_exceeded", "a".into(), now()).unwrap();
        assert!(pause.blocks("a", now() + Duration::days(90)));
        assert!(!pause.blocks("b", now()));
    }

    #[test]
    fn does_not_pause_transient_or_per_turn_failures() {
        for message in [
            "429 too many requests",
            "free_chat_turn_request_limit_exceeded",
            "priced_request_in_flight",
            "503 overloaded",
            "activity_quality_failed:parse_error=true",
        ] {
            assert!(QuotaPause::from_error(message, "a".into(), now()).is_none());
        }
    }

    #[test]
    fn expired_or_invalid_reset_does_not_create_a_retry_loop() {
        for reset in ["2026-09-14T12:00:00Z", "invalid"] {
            let message =
                format!(r#"{{"error":"free_chat_limit_exceeded","resets_at":"{reset}"}}"#);
            assert_eq!(
                QuotaPause::from_error(&message, "a".into(), now())
                    .unwrap()
                    .retry_at,
                Some(now() + Duration::days(1))
            );
        }
    }
}
