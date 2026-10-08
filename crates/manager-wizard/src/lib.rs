//! Setup wizard step state machine (spec §4.1).
//!
//! The wizard is a path, not a place: ten ordered steps run automatically,
//! each a `queued → running → done | failed` state machine with `skipped`
//! and `manual` terminal states. [`RunMode::Audit`] re-runs check-type work
//! without reinstalling.

use async_trait::async_trait;
use manager_config::AppConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Lifecycle state of one wizard step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepState {
    Queued,
    Running,
    Done,
    Failed,
    Skipped,
    Manual,
}

/// Whether the wizard is installing fresh or re-verifying an existing setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunMode {
    /// Full install: all ten steps, installs included.
    Install,
    /// Re-run audit: check-type steps only, no reinstalls.
    Audit,
}

/// Context handed to every step.
#[derive(Debug, Clone)]
pub struct WizardContext {
    pub config: AppConfig,
    pub data_dir: PathBuf,
    pub mode: RunMode,
}

/// Outcome of a single step execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepOutcome {
    pub state: StepState,
    pub message: String,
    /// Last N log lines for inline display on failure.
    pub log_tail: Vec<String>,
    pub duration_ms: u64,
}

/// One wizard step. Implemented once per step (hardware detect, llama.cpp
/// install, tensor-split config, model downloads, gateway start, VS Code
/// extensions, MCP config, WinML registration, MXC policy, smoke test).
#[async_trait]
pub trait WizardStep: Send + Sync {
    /// Stable id, e.g. "detect-hardware".
    fn id(&self) -> &str;
    /// Human-readable name.
    fn name(&self) -> &str;
    /// Ids that must be `Done` or `Manual` before this step may run.
    fn prerequisites(&self) -> &[String];
    /// `true` for check-type steps, which also run in [`RunMode::Audit`].
    fn is_check(&self) -> bool {
        false
    }
    /// Exact command(s) this step will run, for the "Show commands" UI.
    /// Must equal the argv actually spawned.
    fn planned_commands(&self, ctx: &WizardContext) -> Vec<PlannedCommand>;
    async fn run(&self, ctx: &WizardContext) -> Result<StepOutcome, WizardError>;
}

/// An exact command a step plans to run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedCommand {
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: HashMap<String, String>,
}

/// Per-step report entry in [`WizardReport`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepReport {
    pub id: String,
    pub name: String,
    pub state: StepState,
    pub duration_ms: u64,
    pub log_tail: Vec<String>,
}

/// Aggregate result of a wizard run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WizardReport {
    pub mode: RunMode,
    pub steps: Vec<StepReport>,
}

impl WizardReport {
    /// (done, total) for the overall completion bar.
    pub fn progress(&self) -> (usize, usize) {
        let total = self.steps.len();
        let done = self
            .steps
            .iter()
            .filter(|s| matches!(s.state, StepState::Done | StepState::Manual))
            .count();
        (done, total)
    }
}

/// Ordered auto-runner for the wizard steps.
pub struct Wizard {
    steps: Vec<Box<dyn WizardStep>>,
    states: HashMap<String, StepState>,
    mode: RunMode,
}

impl Wizard {
    pub fn new(steps: Vec<Box<dyn WizardStep>>, mode: RunMode) -> Self {
        todo!("build wizard with {} steps in {mode:?} mode", steps.len())
    }

    /// Run all steps in order, honoring prerequisites. Steps run
    /// automatically; a step cannot run until its prerequisites are
    /// `Done` or `Manual`.
    pub async fn run_all(&mut self, ctx: &WizardContext) -> WizardReport {
        todo!("run all steps for {ctx:?}")
    }

    /// Run (or re-run) one step by id.
    pub async fn run_step(
        &mut self,
        ctx: &WizardContext,
        id: &str,
    ) -> Result<StepOutcome, WizardError> {
        todo!("run step {id} for {ctx:?}")
    }

    /// Reset a failed step to `Queued` so it can run again.
    pub fn retry(&mut self, id: &str) {
        todo!("retry step {id}")
    }

    /// Mark a step skipped with a user note.
    pub fn skip(&mut self, id: &str, note: String) {
        todo!("skip step {id}: {note}")
    }

    /// Mark a step as manually completed (user installed it themselves).
    pub fn mark_manual(&mut self, id: &str, path: String, version: String) {
        todo!("mark step {id} manual: {path} {version}")
    }

    /// (done, total) for the overall completion bar.
    pub fn progress(&self) -> (usize, usize) {
        todo!("wizard progress")
    }
}

/// Wizard errors.
#[derive(Debug, thiserror::Error)]
pub enum WizardError {
    #[error("prerequisite not satisfied for step {step}: waiting on {waiting_on}")]
    PrerequisiteFailed { step: String, waiting_on: String },
    #[error("step {0} failed: {1}")]
    StepFailed(String, String),
    #[error("unknown step: {0}")]
    UnknownStep(String),
    #[error("wizard cancelled")]
    Cancelled,
    #[error(transparent)]
    Config(#[from] manager_config::ConfigError),
}

#[cfg(test)]
mod tests {
    use super::*;

    struct OkStep;

    #[async_trait]
    impl WizardStep for OkStep {
        fn id(&self) -> &str {
            "ok-step"
        }
        fn name(&self) -> &str {
            "OK step"
        }
        fn prerequisites(&self) -> &[String] {
            &[]
        }
        fn planned_commands(&self, _ctx: &WizardContext) -> Vec<PlannedCommand> {
            vec![]
        }
        async fn run(&self, _ctx: &WizardContext) -> Result<StepOutcome, WizardError> {
            Ok(StepOutcome {
                state: StepState::Done,
                message: "ok".to_string(),
                log_tail: vec![],
                duration_ms: 1,
            })
        }
    }

    #[tokio::test]
    async fn step_contract_returns_done() {
        let step = OkStep;
        assert_eq!(step.id(), "ok-step");
        assert!(step.prerequisites().is_empty());
        let ctx = WizardContext {
            config: AppConfig::default_config(),
            data_dir: PathBuf::from("test-data"),
            mode: RunMode::Install,
        };
        let outcome = step.run(&ctx).await.expect("step runs");
        assert_eq!(outcome.state, StepState::Done);
    }
}
