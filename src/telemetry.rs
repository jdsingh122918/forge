use anyhow::{Context, Result};
use std::io::IsTerminal;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub struct TelemetryConfig {
    pub log_level: String,
    pub log_format: Option<String>,
    pub log_dir: std::path::PathBuf,
    pub otlp_endpoint: Option<String>,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            log_level: "warn".to_string(),
            log_format: None,
            log_dir: std::path::PathBuf::from(".forge/logs"),
            otlp_endpoint: None,
        }
    }
}

pub fn init_telemetry(config: &TelemetryConfig) -> Result<()> {
    let env_filter =
        EnvFilter::try_new(&config.log_level).unwrap_or_else(|_| EnvFilter::new("warn"));

    let is_tty = std::io::stderr().is_terminal();
    let use_json = match config.log_format.as_deref() {
        Some("json") => true,
        Some("pretty") | Some("compact") => false,
        _ => !is_tty,
    };

    // File layer — always-on JSON appender
    std::fs::create_dir_all(&config.log_dir).context("Failed to create log directory")?;
    let file_appender = tracing_appender::rolling::daily(&config.log_dir, "forge.jsonl");
    let file_layer = fmt::layer()
        .json()
        .with_writer(file_appender)
        .with_target(true)
        .with_span_events(fmt::format::FmtSpan::CLOSE);

    // Build stderr layers using Option<Layer> to avoid type branching
    let json_stderr = if use_json {
        Some(
            fmt::layer()
                .json()
                .with_writer(std::io::stderr)
                .with_target(true),
        )
    } else {
        None
    };

    let compact_stderr = if !use_json {
        Some(
            fmt::layer()
                .compact()
                .with_writer(std::io::stderr)
                .with_target(false)
                .with_ansi(true),
        )
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer)
        .with(json_stderr)
        .with(compact_stderr)
        .try_init()
        .ok(); // ok() because tests may call this multiple times

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracing_imports_available() {
        use tracing::info;
        info!("test");
    }

    #[test]
    fn test_default_config() {
        let config = TelemetryConfig::default();
        assert_eq!(config.log_level, "warn");
        assert!(config.log_format.is_none());
        assert!(config.otlp_endpoint.is_none());
    }

    #[test]
    fn test_init_telemetry_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let config = TelemetryConfig {
            log_dir: dir.path().to_path_buf(),
            ..Default::default()
        };
        init_telemetry(&config).unwrap();
    }
}
