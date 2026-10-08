//! Windows ML secondary backend (spec §6.5).
//!
//! `WinMLServer.exe` managed as an optional supervised process on :8090 and
//! registered in LiteLLM as a plain OpenAI-compatible backend. Constraints
//! enforced from research: single-GPU models only (no tensor-split surface
//! exists), coarse `--target gpu` with undocumented physical-GPU selection,
//! experimental status bannered in the UI.
//!
//! Eligible model class: tool-runner-sized single-GPU loads (e.g. a Qwen3-8B
//! instance) — never the planner or coder-30b. "Re-check capabilities"
//! re-probes and updates the banner.

use chrono::{DateTime, Utc};
use manager_config::WinMlConfig;
use serde::{Deserialize, Serialize};

/// How the physical GPU is selected (coarse: undocumented which adapter
/// responds; recorded at registration, warned on ambiguity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuSelection {
    SystemDefault,
    TargetGpuCoarse,
}

/// Researched capability limits. These are recorded verbatim and bannered in
/// the UI; if Microsoft later publishes tensor-split/device-index controls,
/// this struct is revisited — the primary path is not.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WinMlLimits {
    pub supports_tensor_split: bool,
    pub max_registered_models: u32,
    pub gpu_selection: GpuSelection,
    pub experimental: bool,
}

impl WinMlLimits {
    /// Limits as researched (Oct 2026).
    pub fn known_limits() -> Self {
        Self {
            supports_tensor_split: false,
            max_registered_models: 1,
            gpu_selection: GpuSelection::TargetGpuCoarse,
            experimental: true,
        }
    }
}

/// A registered WinML backend instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WinMlRegistration {
    pub model_id: String,
    pub port: u16,
    /// OpenAI-compatible endpoint, e.g. http://127.0.0.1:8090/v1.
    pub endpoint: String,
    pub registered_at: DateTime<Utc>,
    /// Which adapter actually responded at registration (ambiguity warned).
    pub responding_adapter: Option<String>,
    pub limits: WinMlLimits,
}

/// Probe WinMLServer availability and record capability limits verbatim.
pub async fn probe() -> Result<WinMlLimits, WinMlError> {
    todo!("probe WinMLServer availability")
}

/// Register one small (tool-runner-class) model as a secondary LiteLLM
/// backend. Refuses planner/coder-30b-class models.
pub async fn register(
    config: &WinMlConfig,
    model_id: &str,
) -> Result<WinMlRegistration, WinMlError> {
    let _ = (config, model_id);
    todo!("register WinML model {model_id}")
}

/// Unregister the WinML backend.
pub async fn unregister() -> Result<(), WinMlError> {
    todo!("unregister WinML backend")
}

/// Health probe against the /v1 endpoint.
pub async fn health(port: u16) -> Result<bool, WinMlError> {
    todo!("health probe on :{port}")
}

/// WinML errors.
#[derive(Debug, thiserror::Error)]
pub enum WinMlError {
    #[error("Windows ML unavailable: {0}")]
    Unavailable(String),
    #[error("registration failed: {0}")]
    RegistrationFailed(String),
    #[error("model class not eligible for WinML secondary target: {0}")]
    IneligibleModel(String),
    #[error("health check failed: {0}")]
    HealthCheckFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winml_limits_have_no_tensor_split() {
        let limits = WinMlLimits::known_limits();
        assert!(!limits.supports_tensor_split);
        assert_eq!(limits.max_registered_models, 1);
        assert!(limits.experimental);
        assert_eq!(limits.gpu_selection, GpuSelection::TargetGpuCoarse);
    }
}
