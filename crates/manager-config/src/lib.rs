//! Versioned, serde-typed application configuration (spec §10).
//!
//! Single JSON file at `%APPDATA%\local-llm-service-manager\config.json`
//! (schema `version: u32`, current [`CURRENT_VERSION`]). Secrets are never
//! stored here — only credential *references* (Credential Manager target names).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Current config schema version.
pub const CURRENT_VERSION: u32 = 1;

/// Top-level application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub version: u32,
    /// Models, builds, logs, managed Python env.
    pub data_dir: PathBuf,
    /// window_id -> preferred presenter.
    pub view_modes: HashMap<String, ViewMode>,
    pub backends: Vec<BackendConfig>,
    pub gateway: GatewayConfig,
    pub models: Vec<ModelConfig>,
    /// Poll interval and retention.
    pub telemetry: TelemetryConfig,
    pub mxc: MxcPolicyRef,
    pub winml: WinMlConfig,
    /// Per-component pin / auto-update toggles.
    pub updates: UpdatePolicy,
    pub notifications: NotificationConfig,
}

/// Presenter mode for a data window (spec §4 hybrid view matrix).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    Appliance,
    Console,
    Topology,
}

/// One llama-server backend (planner | coder | coder-fast | tool-runner).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    pub id: String,
    pub enabled: bool,
    pub model_file: PathBuf,
    pub port: u16,
    /// Structured server flags.
    pub flags: ServerFlags,
    /// Raw extra flags appended verbatim; None means none.
    pub raw_flags: Option<String>,
    pub restart_policy: RestartPolicy,
    pub overrides_global: bool,
}

/// Structured llama-server flags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerFlags {
    pub n_ctx: u32,
    pub n_batch: u32,
    /// Per-GPU fractions; must sum to 1.0 when non-empty.
    pub tensor_split: Vec<f32>,
    pub split_mode: SplitMode,
}

/// llama.cpp tensor-split placement mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitMode {
    None,
    Layer,
    Row,
}

/// Crash-restart policy for a supervised backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestartPolicy {
    pub max_retries: u32,
    pub backoff_base_secs: u64,
}

/// LiteLLM gateway configuration (spec §7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// 4000 by convention.
    pub port: u16,
    pub groups: Vec<ModelGroup>,
    /// Plain-language rule + LiteLLM fragment + source.
    pub routing_rules: Vec<RoutingRule>,
    pub openrouter: CloudTierConfig,
    /// Request ring length (default 100).
    pub request_log_len: usize,
}

/// A LiteLLM model group routing to one or more backends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelGroup {
    pub name: String,
    /// Backend ids (e.g. "planner"), or "host:port" references.
    pub members: Vec<String>,
    pub strategy: RoutingStrategy,
    /// Ordered fallback chain, e.g. ["coder", "openrouter"].
    pub fallbacks: Vec<String>,
}

/// Routing strategy for a model group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RoutingStrategy {
    SimpleShuffle,
    LeastBusy,
    LatencyBased,
}

/// A plain-language routing rule with its LiteLLM fragment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRule {
    pub id: String,
    pub description: String,
    pub litellm_fragment: serde_json::Value,
    /// Where the rule came from (wizard default | user).
    pub source: String,
}

/// OpenRouter cloud tier: failover, overflow, rubric-based escalation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudTierConfig {
    pub enabled: bool,
    /// Credential Manager target name — never the key itself.
    pub cred_ref: Option<String>,
    pub daily_cap_usd: f64,
    pub model_allowlist: Vec<String>,
}

/// A downloaded GGUF model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub id: String,
    pub gguf_path: PathBuf,
    pub quant: String,
    pub params_b: f32,
    pub sha256: String,
    pub verified_at: Option<DateTime<Utc>>,
}

/// Telemetry polling configuration (spec §5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    /// 1 s default; 0 = off.
    pub poll_interval_ms: u64,
    /// High-res retention window in seconds.
    pub retention_secs: u64,
}

/// Reference to the MXC sandbox policy (spec §11).
///
/// The policy JSON itself lives outside this config; only the path and the
/// enforcement mode are stored here. Secrets are never in the policy JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MxcPolicyRef {
    pub path: PathBuf,
    pub mode: MxcMode,
}

/// MXC enforcement mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MxcMode {
    Learning,
    Enforced,
    Disabled,
}

/// Windows ML secondary backend registration (spec §6.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WinMlConfig {
    pub enabled: bool,
    /// 8090 by convention.
    pub port: u16,
    pub model_id: Option<String>,
}

/// Per-component update pins and auto-update toggles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePolicy {
    pub components: HashMap<String, ComponentUpdate>,
}

/// Update policy for one component (llama.cpp build, LiteLLM, models, ...).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentUpdate {
    pub pinned_version: Option<String>,
    pub auto_update: bool,
    pub last_checked: Option<DateTime<Utc>>,
}

/// Notification preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationConfig {
    pub enabled: bool,
}

/// Errors from config load/save/validate/migrate.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("migration failed: {0}")]
    MigrationFailed(String),
    #[error("backup failed: {0}")]
    BackupFailed(String),
}

impl AppConfig {
    /// Sensible first-run defaults matching the spec's port/strategy conventions.
    pub fn default_config() -> Self {
        let strategies = [
            ("planner", RoutingStrategy::LatencyBased, vec!["coder", "openrouter"]),
            ("coder-fast", RoutingStrategy::LeastBusy, vec!["coder", "openrouter"]),
            ("coder", RoutingStrategy::SimpleShuffle, vec!["openrouter"]),
            ("tool-runner", RoutingStrategy::LeastBusy, vec!["openrouter"]),
        ];
        let groups = strategies
            .into_iter()
            .map(|(name, strategy, fallbacks)| ModelGroup {
                name: name.to_string(),
                members: vec![name.to_string()],
                strategy,
                fallbacks: fallbacks.into_iter().map(str::to_string).collect(),
            })
            .collect();
        Self {
            version: CURRENT_VERSION,
            data_dir: default_data_dir(),
            view_modes: HashMap::new(),
            backends: Vec::new(),
            gateway: GatewayConfig {
                port: 4000,
                groups,
                routing_rules: Vec::new(),
                openrouter: CloudTierConfig {
                    enabled: false,
                    cred_ref: None,
                    daily_cap_usd: 5.0,
                    model_allowlist: Vec::new(),
                },
                request_log_len: 100,
            },
            models: Vec::new(),
            telemetry: TelemetryConfig {
                poll_interval_ms: 1000,
                retention_secs: 60,
            },
            mxc: MxcPolicyRef {
                path: default_data_dir().join("mxc-policy.json"),
                mode: MxcMode::Learning,
            },
            winml: WinMlConfig {
                enabled: false,
                port: 8090,
                model_id: None,
            },
            updates: UpdatePolicy {
                components: HashMap::new(),
            },
            notifications: NotificationConfig { enabled: true },
        }
    }
}

/// Load, migrate, and validate the config at `path`.
///
/// On validation failure, falls back to the `.bak` last-known-good copy.
pub fn load(path: &PathBuf) -> Result<AppConfig, ConfigError> {
    todo!("load config from {path:?}, migrate, validate, fall back to .bak")
}

/// Validate and save the config, keeping a `.bak` of the previous file.
pub fn save(config: &AppConfig, path: &PathBuf) -> Result<(), ConfigError> {
    todo!("validate then save config to {path:?} with .bak rotation")
}

/// Structural validation: ports unique and in range, tensor splits sum to 1.0,
/// model files referenced by backends exist in `models`, etc.
pub fn validate(config: &AppConfig) -> Result<(), ConfigError> {
    todo!("validate {config:?}")
}

/// Migrate an older raw JSON config forward to [`CURRENT_VERSION`].
pub fn migrate(raw: serde_json::Value) -> Result<AppConfig, ConfigError> {
    todo!("migrate config version in {raw}")
}

/// Default data directory: `%APPDATA%\local-llm-service-manager`
/// (falls back to `~/.local-llm-service-manager` off-Windows).
pub fn default_data_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        PathBuf::from(appdata).join("local-llm-service-manager")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local-llm-service-manager")
    } else {
        PathBuf::from(".local-llm-service-manager")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_round_trips() {
        let cfg = AppConfig::default_config();
        assert_eq!(cfg.version, CURRENT_VERSION);
        assert_eq!(cfg.gateway.port, 4000);
        assert_eq!(cfg.gateway.groups.len(), 4);
        let json = serde_json::to_string(&cfg).expect("serialize");
        let back: AppConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.version, CURRENT_VERSION);
        assert_eq!(back.gateway.groups[0].strategy, RoutingStrategy::LatencyBased);
    }
}
