use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouncilConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_chairman_model")]
    pub chairman_model: String,
    #[serde(default = "default_chairman_reasoning_effort")]
    pub chairman_reasoning_effort: String,
    #[serde(default = "default_chairman_retry_budget")]
    pub chairman_retry_budget: u32,
    #[serde(default = "default_anonymize_reviews")]
    pub anonymize_reviews: bool,
    #[serde(default)]
    pub workers: HashMap<String, WorkerConfig>,
}

fn default_chairman_model() -> String {
    "gpt-5.4".to_string()
}

fn default_chairman_reasoning_effort() -> String {
    "xhigh".to_string()
}

fn default_chairman_retry_budget() -> u32 {
    3
}

fn default_anonymize_reviews() -> bool {
    true
}

impl CouncilConfig {
    /// Council requires at least two workers for meaningful peer review.
    pub fn has_minimum_workers(&self) -> bool {
        self.workers.len() >= 2
    }

    /// Resolve whether council is enabled, allowing `COUNCIL_ENABLED` to override config.
    pub fn resolve_enabled(&self) -> bool {
        if let Ok(value) = std::env::var("COUNCIL_ENABLED")
            && let Ok(enabled) = value.to_ascii_lowercase().parse::<bool>()
        {
            return enabled;
        }

        self.enabled
    }
}

impl Default for CouncilConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            chairman_model: default_chairman_model(),
            chairman_reasoning_effort: default_chairman_reasoning_effort(),
            chairman_retry_budget: default_chairman_retry_budget(),
            anonymize_reviews: default_anonymize_reviews(),
            workers: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerConfig {
    pub cmd: String,
    #[serde(default = "default_worker_role")]
    pub role: String,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub sandbox: Option<String>,
    #[serde(default)]
    pub approval_policy: Option<String>,
}

fn default_worker_role() -> String {
    "worker".to_string()
}

#[cfg(test)]
pub(crate) static COUNCIL_ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestToml {
        council: CouncilConfig,
    }

    #[test]
    fn test_council_config_default() {
        let config = CouncilConfig::default();

        assert!(!config.enabled);
        assert!(config.workers.is_empty());
    }

    #[test]
    fn test_council_config_enabled_false_by_default() {
        assert!(!CouncilConfig::default().enabled);
    }

    #[test]
    fn test_council_config_has_minimum_workers_empty() {
        let config = CouncilConfig::default();
        assert!(!config.has_minimum_workers());
    }

    #[test]
    fn test_council_config_has_minimum_workers_one() {
        let mut config = CouncilConfig::default();
        config.workers.insert(
            "claude".to_string(),
            WorkerConfig {
                cmd: "claude".to_string(),
                role: default_worker_role(),
                flags: vec![],
                model: None,
                reasoning_effort: None,
                sandbox: None,
                approval_policy: None,
            },
        );

        assert!(!config.has_minimum_workers());
    }

    #[test]
    fn test_council_config_has_minimum_workers_two() {
        let mut config = CouncilConfig::default();
        config.workers.insert(
            "claude".to_string(),
            WorkerConfig {
                cmd: "claude".to_string(),
                role: default_worker_role(),
                flags: vec![],
                model: None,
                reasoning_effort: None,
                sandbox: None,
                approval_policy: None,
            },
        );
        config.workers.insert(
            "codex".to_string(),
            WorkerConfig {
                cmd: "codex".to_string(),
                role: default_worker_role(),
                flags: vec![],
                model: None,
                reasoning_effort: None,
                sandbox: None,
                approval_policy: None,
            },
        );

        assert!(config.has_minimum_workers());
    }

    #[test]
    fn test_council_config_parse_full_toml() {
        let parsed: TestToml = toml::from_str(
            r#"
            [council]
            enabled = true
            chairman_model = "gpt-5.4"
            chairman_reasoning_effort = "xhigh"
            chairman_retry_budget = 5
            anonymize_reviews = true

            [council.workers.claude]
            cmd = "claude"
            role = "worker"
            flags = ["--print", "--output-format", "stream-json"]

            [council.workers.codex]
            cmd = "codex"
            role = "worker"
            model = "gpt-5.4"
            reasoning_effort = "xhigh"
            sandbox = "workspace-write"
            "#,
        )
        .unwrap();

        assert!(parsed.council.enabled);
        assert_eq!(parsed.council.chairman_model, "gpt-5.4");
        assert_eq!(parsed.council.chairman_reasoning_effort, "xhigh");
        assert_eq!(parsed.council.chairman_retry_budget, 5);
        assert!(parsed.council.anonymize_reviews);
        assert_eq!(parsed.council.workers.len(), 2);

        let claude = parsed.council.workers.get("claude").unwrap();
        assert_eq!(claude.cmd, "claude");
        assert_eq!(claude.role, "worker");
        assert_eq!(
            claude.flags,
            vec!["--print", "--output-format", "stream-json"]
        );
        assert_eq!(claude.model, None);
        assert_eq!(claude.reasoning_effort, None);
        assert_eq!(claude.sandbox, None);
        assert_eq!(claude.approval_policy, None);

        let codex = parsed.council.workers.get("codex").unwrap();
        assert_eq!(codex.cmd, "codex");
        assert_eq!(codex.role, "worker");
        assert!(codex.flags.is_empty());
        assert_eq!(codex.model.as_deref(), Some("gpt-5.4"));
        assert_eq!(codex.reasoning_effort.as_deref(), Some("xhigh"));
        assert_eq!(codex.sandbox.as_deref(), Some("workspace-write"));
        assert_eq!(codex.approval_policy, None);
    }

    #[test]
    fn test_council_config_parse_minimal_toml() {
        let parsed: TestToml = toml::from_str(
            r#"
            [council]
            enabled = true
            "#,
        )
        .unwrap();

        assert!(parsed.council.enabled);
        assert!(parsed.council.workers.is_empty());
    }

    #[test]
    fn test_worker_config_defaults() {
        let parsed: TestToml = toml::from_str(
            r#"
            [council]
            enabled = true

            [council.workers.claude]
            cmd = "claude"
            "#,
        )
        .unwrap();

        let worker = parsed.council.workers.get("claude").unwrap();
        assert_eq!(worker.role, "worker");
        assert!(worker.flags.is_empty());
    }

    #[test]
    fn test_worker_config_with_all_fields() {
        let parsed: TestToml = toml::from_str(
            r#"
            [council]
            enabled = true

            [council.workers.codex]
            cmd = "codex"
            role = "worker"
            flags = ["-q", "--json"]
            model = "gpt-5.4"
            reasoning_effort = "xhigh"
            sandbox = "workspace-write"
            approval_policy = "on-failure"
            "#,
        )
        .unwrap();

        let worker = parsed.council.workers.get("codex").unwrap();
        assert_eq!(worker.cmd, "codex");
        assert_eq!(worker.role, "worker");
        assert_eq!(worker.flags, vec!["-q", "--json"]);
        assert_eq!(worker.model.as_deref(), Some("gpt-5.4"));
        assert_eq!(worker.reasoning_effort.as_deref(), Some("xhigh"));
        assert_eq!(worker.sandbox.as_deref(), Some("workspace-write"));
        assert_eq!(worker.approval_policy.as_deref(), Some("on-failure"));
    }

    #[test]
    fn test_council_config_serialization_roundtrip() {
        let mut workers = HashMap::new();
        workers.insert(
            "codex".to_string(),
            WorkerConfig {
                cmd: "codex".to_string(),
                role: "worker".to_string(),
                flags: vec!["-q".to_string(), "--json".to_string()],
                model: Some("gpt-5.4".to_string()),
                reasoning_effort: Some("xhigh".to_string()),
                sandbox: Some("workspace-write".to_string()),
                approval_policy: Some("on-failure".to_string()),
            },
        );

        let original = CouncilConfig {
            enabled: true,
            chairman_model: "gpt-5.4".to_string(),
            chairman_reasoning_effort: "xhigh".to_string(),
            chairman_retry_budget: 5,
            anonymize_reviews: true,
            workers,
        };

        let serialized = toml::to_string(&original).unwrap();
        let round_trip: CouncilConfig = toml::from_str(&serialized).unwrap();

        assert_eq!(round_trip.enabled, original.enabled);
        assert_eq!(round_trip.chairman_model, original.chairman_model);
        assert_eq!(
            round_trip.chairman_reasoning_effort,
            original.chairman_reasoning_effort
        );
        assert_eq!(
            round_trip.chairman_retry_budget,
            original.chairman_retry_budget
        );
        assert_eq!(round_trip.anonymize_reviews, original.anonymize_reviews);
        assert_eq!(round_trip.workers.len(), 1);

        let worker = round_trip.workers.get("codex").unwrap();
        assert_eq!(worker.cmd, "codex");
        assert_eq!(worker.approval_policy.as_deref(), Some("on-failure"));
    }

    #[test]
    fn test_council_config_missing_workers_is_valid() {
        let parsed: TestToml = toml::from_str(
            r#"
            [council]
            enabled = true
            chairman_model = "gpt-5.4"
            "#,
        )
        .unwrap();

        assert!(parsed.council.enabled);
        assert!(parsed.council.workers.is_empty());
    }

    #[test]
    fn test_council_config_chairman_retry_budget_default() {
        assert_eq!(CouncilConfig::default().chairman_retry_budget, 3);
    }

    #[test]
    fn test_council_config_anonymize_reviews_default_true() {
        assert!(CouncilConfig::default().anonymize_reviews);
    }

    #[test]
    fn test_resolve_enabled_no_env_returns_config_value() {
        let _guard = COUNCIL_ENV_MUTEX.lock().unwrap();
        let saved = std::env::var("COUNCIL_ENABLED").ok();
        unsafe { std::env::remove_var("COUNCIL_ENABLED") };

        let mut config = CouncilConfig::default();
        config.enabled = true;
        assert!(config.resolve_enabled());

        config.enabled = false;
        assert!(!config.resolve_enabled());

        match saved {
            Some(value) => unsafe { std::env::set_var("COUNCIL_ENABLED", value) },
            None => unsafe { std::env::remove_var("COUNCIL_ENABLED") },
        }
    }

    #[test]
    fn test_resolve_enabled_env_true_overrides() {
        let _guard = COUNCIL_ENV_MUTEX.lock().unwrap();
        let saved = std::env::var("COUNCIL_ENABLED").ok();
        unsafe { std::env::set_var("COUNCIL_ENABLED", "true") };

        let config = CouncilConfig::default();
        assert!(config.resolve_enabled());

        match saved {
            Some(value) => unsafe { std::env::set_var("COUNCIL_ENABLED", value) },
            None => unsafe { std::env::remove_var("COUNCIL_ENABLED") },
        }
    }

    #[test]
    fn test_resolve_enabled_env_false_overrides() {
        let _guard = COUNCIL_ENV_MUTEX.lock().unwrap();
        let saved = std::env::var("COUNCIL_ENABLED").ok();
        unsafe { std::env::set_var("COUNCIL_ENABLED", "false") };

        let mut config = CouncilConfig::default();
        config.enabled = true;
        assert!(!config.resolve_enabled());

        match saved {
            Some(value) => unsafe { std::env::set_var("COUNCIL_ENABLED", value) },
            None => unsafe { std::env::remove_var("COUNCIL_ENABLED") },
        }
    }

    #[test]
    fn test_resolve_enabled_env_invalid_uses_config() {
        let _guard = COUNCIL_ENV_MUTEX.lock().unwrap();
        let saved = std::env::var("COUNCIL_ENABLED").ok();
        unsafe { std::env::set_var("COUNCIL_ENABLED", "notabool") };

        let mut config = CouncilConfig::default();
        config.enabled = true;
        assert!(config.resolve_enabled());

        match saved {
            Some(value) => unsafe { std::env::set_var("COUNCIL_ENABLED", value) },
            None => unsafe { std::env::remove_var("COUNCIL_ENABLED") },
        }
    }
}
