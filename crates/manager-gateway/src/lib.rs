//! LiteLLM gateway config generation (spec §7).
//!
//! Generates LiteLLM `config.yaml` from [`AppConfig`]. The generated file is
//! never hand-edited: the UI shows a diff preview, and applying writes a new
//! generated file (hot-reload where supported, drained restart otherwise).
//!
//! Model groups and default strategies (spec §7):
//! `planner` (latency-based) → coder → OpenRouter;
//! `coder-fast` (least-busy) → coder → OpenRouter;
//! `coder` (simple-shuffle) → OpenRouter;
//! `tool-runner` (least-busy, + WinML :8090 if registered) → OpenRouter.

use manager_config::{AppConfig, ModelGroup, RoutingStrategy};
use serde::{Deserialize, Serialize};

/// A LiteLLM `model` entry generated from a [`ModelGroup`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiteLlmModelEntry {
    pub model_name: String,
    pub litellm_params: LiteLlmParams,
}

/// The `litellm_params` block: OpenAI-compatible endpoint + routing knobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiteLlmParams {
    pub api_base: String,
    pub api_key: String,
    /// Backend-specific strategy hint consumed by the strategy picker.
    pub strategy: RoutingStrategy,
}

/// OpenRouter fallback target for a group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterTarget {
    pub enabled: bool,
    pub model_allowlist: Vec<String>,
    pub daily_cap_usd: f64,
}

/// Generate the full LiteLLM `config.yaml` from the app config.
pub fn generate_config_yaml(config: &AppConfig) -> Result<String, GatewayError> {
    let _ = config;
    todo!("render LiteLLM config.yaml from AppConfig")
}

/// Build the `model` entries for one group (members + fallbacks).
pub fn model_entries_for_group(
    group: &ModelGroup,
    config: &AppConfig,
) -> Result<Vec<LiteLlmModelEntry>, GatewayError> {
    let _ = (group, config);
    todo!("model entries for group {}", group.name)
}

/// Strategy picker: resolves the effective strategy for a group given
/// live backend health (e.g. degrade latency-based to simple-shuffle when
/// latency data is unavailable).
pub fn pick_strategy(group: &ModelGroup, latency_data_available: bool) -> RoutingStrategy {
    let _ = group;
    todo!("pick strategy (latency_data_available={latency_data_available})")
}

/// Gateway errors.
#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("invalid gateway config: {0}")]
    ConfigInvalid(String),
    #[error("YAML serialization failed: {0}")]
    Serialization(#[from] serde_yaml::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_group_constructs_with_strategy() {
        let g = ModelGroup {
            name: "planner".to_string(),
            members: vec!["planner".to_string()],
            strategy: RoutingStrategy::LatencyBased,
            fallbacks: vec!["coder".to_string(), "openrouter".to_string()],
        };
        assert_eq!(g.strategy, RoutingStrategy::LatencyBased);
        assert_eq!(g.fallbacks.len(), 2);
    }
}
