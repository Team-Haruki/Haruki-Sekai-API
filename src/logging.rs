use std::fmt;

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::registry::LookupSpan;

const COLOR_GREEN: &str = "\x1b[32m";
const COLOR_BLUE: &str = "\x1b[34m";
const COLOR_MAGENTA: &str = "\x1b[35m";
const COLOR_YELLOW: &str = "\x1b[33m";
const COLOR_RED: &str = "\x1b[31m";
const COLOR_RESET: &str = "\x1b[0m";

/// Initialize the global tracing subscriber.
///
/// `format` accepts `"text"` (default, colored single-line) or `"json"` (one
/// JSON object per line, suitable for stdout-based log shippers in K8s).
/// Unknown values fall back to text with a stderr warning.
pub fn init(format: &str) {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("haruki_sekai_api=info,warn"));

    match format {
        "json" => {
            let _ = tracing_subscriber::fmt()
                .json()
                .with_current_span(true)
                .with_span_list(false)
                .with_env_filter(env_filter)
                .try_init();
        }
        "text" => {
            let _ = tracing_subscriber::fmt()
                .event_format(ColoredFormatter)
                .with_env_filter(env_filter)
                .try_init();
        }
        other => {
            eprintln!(
                "warning: unknown log_format '{}', falling back to 'text'",
                other
            );
            let _ = tracing_subscriber::fmt()
                .event_format(ColoredFormatter)
                .with_env_filter(env_filter)
                .try_init();
        }
    }
}

struct ColoredFormatter;

impl<S, N> FormatEvent<S, N> for ColoredFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let metadata = event.metadata();
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let level = level_name(metadata.level());
        let level_color = level_color(metadata.level());
        let component = component_name(metadata.target());
        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);

        let identity_tags = visitor.identity_tags();
        let after_component = if identity_tags.is_empty() { " " } else { "" };
        let after_identity = if identity_tags.is_empty() { "" } else { " " };
        let fields = if visitor.fields.is_empty() {
            String::new()
        } else {
            format!(" {}", visitor.fields.join(" "))
        };
        let message = format!("{}{}", visitor.message.unwrap_or_default(), fields);

        writeln!(
            writer,
            "{}[{}]{}[{}{}{}][{}{}{}]{}{}{}{}",
            COLOR_GREEN,
            now,
            COLOR_RESET,
            level_color,
            level,
            COLOR_RESET,
            COLOR_MAGENTA,
            component,
            COLOR_RESET,
            after_component,
            identity_tags,
            after_identity,
            message
        )
    }
}

#[derive(Default)]
struct EventVisitor {
    message: Option<String>,
    region: Option<String>,
    user_id: Option<String>,
    fields: Vec<String>,
}

impl EventVisitor {
    fn record_value(&mut self, field: &Field, value: String) {
        match field.name() {
            "message" => {
                self.message = Some(value);
            }
            "region" | "server" | "server_region" if !value.trim().is_empty() => {
                self.region = Some(normalize_region(&value));
            }
            "user_id" if !value.trim().is_empty() => {
                self.user_id = Some(trim_debug_quotes(&value).to_string());
            }
            "log_message" => {
                self.fields.push(format!("message={value}"));
            }
            _ => {
                self.fields.push(format!("{}={}", field.name(), value));
            }
        }
    }

    fn identity_tags(&self) -> String {
        let mut tags = String::new();
        if let Some(region) = &self.region {
            tags.push_str(&format!("[{}{}{}]", COLOR_BLUE, region, COLOR_RESET));
        }
        if let Some(user_id) = &self.user_id {
            tags.push_str(&format!("[{}User-{}{}]", COLOR_BLUE, user_id, COLOR_RESET));
        }
        tags
    }
}

impl Visit for EventVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_value(field, value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record_value(field, value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record_value(field, value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record_value(field, value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record_value(field, format!("{value:?}"));
    }
}

fn trim_debug_quotes(value: &str) -> &str {
    value.trim().trim_matches('"')
}

fn normalize_region(value: &str) -> String {
    match trim_debug_quotes(value) {
        "Jp" | "jp" => "JP".to_string(),
        "En" | "en" => "EN".to_string(),
        "Tw" | "tw" => "TW".to_string(),
        "Kr" | "kr" => "KR".to_string(),
        "Cn" | "cn" => "CN".to_string(),
        other => other.to_ascii_uppercase(),
    }
}

fn level_name(level: &Level) -> &'static str {
    match *level {
        Level::TRACE => "TRACE",
        Level::DEBUG => "DEBUG",
        Level::INFO => "INFO",
        Level::WARN => "WARNING",
        Level::ERROR => "ERROR",
    }
}

fn level_color(level: &Level) -> &'static str {
    match *level {
        Level::TRACE => COLOR_MAGENTA,
        Level::DEBUG => COLOR_BLUE,
        Level::INFO => COLOR_GREEN,
        Level::WARN => COLOR_YELLOW,
        Level::ERROR => COLOR_RED,
    }
}

fn component_name(target: &str) -> &str {
    let mut parts = target.split("::");
    match parts.next() {
        Some("haruki_sekai_api") => match parts.next() {
            None => "main",
            Some(component @ ("api" | "client" | "updater")) => parts.next().unwrap_or(component),
            Some("db") => "db",
            Some("ingest_engine") => "ingest",
            Some("crypto") => "crypto",
            Some("models") => "models",
            Some(component) => component,
        },
        Some(component) => component,
        None => "main",
    }
}
