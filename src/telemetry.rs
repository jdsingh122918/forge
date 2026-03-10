use anyhow::{Context, Result};
use std::io::IsTerminal;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Debug, Clone, Default, clap::ValueEnum)]
pub enum LogFormat {
    Json,
    #[default]
    Compact,
}

pub struct TelemetryConfig {
    pub log_level: String,
    pub log_format: Option<LogFormat>,
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
    let env_filter = EnvFilter::try_new(&config.log_level).unwrap_or_else(|e| {
        eprintln!(
            "[forge] Warning: invalid log level '{}' ({}), defaulting to 'warn'",
            config.log_level, e
        );
        EnvFilter::new("warn")
    });

    let is_tty = std::io::stderr().is_terminal();
    let use_json = match &config.log_format {
        Some(LogFormat::Json) => true,
        Some(_) => false,
        None => !is_tty,
    };

    if config.otlp_endpoint.is_some() {
        eprintln!(
            "[forge] Warning: --otlp-endpoint is not yet implemented; traces will not be exported"
        );
    }

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
                .with_ansi(is_tty),
        )
    } else {
        None
    };

    match tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer)
        .with(json_stderr)
        .with(compact_stderr)
        .try_init()
    {
        Ok(()) => Ok(()),
        Err(e) if e.to_string().contains("already been set") => Ok(()),
        Err(e) => Err(anyhow::anyhow!(
            "Failed to initialize tracing subscriber: {}",
            e
        )),
    }
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
