//! In-memory ring buffer of recent master log records, fed by a tracing Layer.
//!
//! It lets the panel show a live tail of the master's own runtime logs (the
//! `M###` codes from docs/error-codes.md) without journald access. Bounded:
//! oldest records drop off the front. Only this crate's events are captured, so
//! third-party (sqlx/hyper/tonic) chatter stays out.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Utc};
use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

const CAPACITY: usize = 500;

#[derive(Clone, Serialize)]
pub struct LogRecord {
    pub ts: DateTime<Utc>,
    pub level: String,
    pub code: Option<String>,
    pub target: String,
    pub message: String,
    pub fields: String,
}

static RING: OnceLock<Mutex<VecDeque<LogRecord>>> = OnceLock::new();

fn ring() -> &'static Mutex<VecDeque<LogRecord>> {
    RING.get_or_init(|| Mutex::new(VecDeque::with_capacity(CAPACITY)))
}

fn matches_filter(
    record: &LogRecord,
    level: Option<&str>,
    code: Option<&str>,
    query: Option<&str>,
) -> bool {
    if level.is_some_and(|value| record.level != value) {
        return false;
    }
    if code.is_some_and(|value| record.code.as_deref() != Some(value)) {
        return false;
    }
    query.is_none_or(|value| {
        let value = value.to_ascii_lowercase();
        format!(
            "{} {} {} {}",
            record.level,
            record.code.as_deref().unwrap_or(""),
            record.target,
            record.message
        )
        .to_ascii_lowercase()
        .contains(&value)
            || record.fields.to_ascii_lowercase().contains(&value)
    })
}

/// Search the bounded ring newest-first. Filters are applied before the
/// response limit so an operator can search older entries still in memory.
pub fn search(
    limit: usize,
    level: Option<&str>,
    code: Option<&str>,
    query: Option<&str>,
) -> Vec<LogRecord> {
    let guard = ring().lock().unwrap_or_else(|e| e.into_inner());
    guard
        .iter()
        .rev()
        .filter(|record| matches_filter(record, level, code, query))
        .take(limit.min(CAPACITY))
        .cloned()
        .collect()
}

fn push(record: LogRecord) {
    let mut guard = ring().lock().unwrap_or_else(|e| e.into_inner());
    if guard.len() == CAPACITY {
        guard.pop_front();
    }
    guard.push_back(record);
}

/// The tracing Layer that captures this crate's events into the ring.
pub struct CaptureLayer;

#[derive(Clone)]
struct SpanFields(Vec<String>);

pub fn layer() -> CaptureLayer {
    CaptureLayer
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attributes: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut visitor = Collector::default();
        attributes.record(&mut visitor);
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanFields(visitor.extra));
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let meta = event.metadata();
        // keep the buffer focused on honey's own logs, not dependency chatter.
        if !meta.target().starts_with(env!("CARGO_CRATE_NAME")) {
            return;
        }
        let mut visitor = Collector::default();
        event.record(&mut visitor);
        let mut fields = Vec::new();
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                if let Some(values) = span.extensions().get::<SpanFields>() {
                    fields.extend(values.0.iter().cloned());
                }
            }
        }
        fields.extend(visitor.extra);
        push(LogRecord {
            ts: Utc::now(),
            level: meta.level().as_str().to_ascii_lowercase(),
            code: visitor.code,
            target: meta.target().to_string(),
            message: visitor.message,
            fields: fields.join(" "),
        });
    }
}

#[derive(Default)]
struct Collector {
    message: String,
    code: Option<String>,
    extra: Vec<String>,
}

impl Visit for Collector {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message = value.to_string(),
            "code" => self.code = Some(value.to_string()),
            name => self.extra.push(format!("{name}={value}")),
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let value = format!("{value:?}");
        match field.name() {
            "message" => self.message = value,
            "code" => self.code = Some(value.trim_matches('"').to_string()),
            name => self.extra.push(format!("{name}={value}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::prelude::*;

    fn record() -> LogRecord {
        LogRecord {
            ts: Utc::now(),
            level: "warn".into(),
            code: Some("M0406".into()),
            target: "honey_master::registry".into(),
            message: "push failed; inspect correlated logs".into(),
            fields: "request_id=abc123".into(),
        }
    }

    #[test]
    fn log_filters_match_code_message_and_request_id() {
        let item = record();
        assert!(matches_filter(&item, Some("warn"), Some("M0406"), None));
        assert!(matches_filter(&item, None, None, Some("request_id=abc123")));
        assert!(matches_filter(&item, None, None, Some("PUSH FAILED")));
        assert!(!matches_filter(&item, Some("error"), None, None));
        assert!(!matches_filter(&item, None, Some("M0409"), None));
    }

    #[test]
    fn request_span_fields_are_attached_to_captured_events() {
        let subscriber = tracing_subscriber::registry().with(layer());
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("http_test", request_id = "span-search-test-42");
            let _entered = span.enter();
            tracing::warn!(
                target: env!("CARGO_CRATE_NAME"),
                code = "M1999",
                "span capture test"
            );
        });
        let records = search(10, Some("warn"), Some("M1999"), Some("span-search-test-42"));
        assert_eq!(records.len(), 1);
        assert!(records[0].fields.contains("request_id=span-search-test-42"));
    }
}
