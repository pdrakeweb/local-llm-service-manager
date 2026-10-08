//! MXC (Microsoft Execution Containers) sandbox policy (spec §11).
//!
//! Sandboxes agent *tool execution only* — inference and LiteLLM stay on the
//! host; contained agents reach them over loopback per policy. Policies are
//! default-deny. Secrets are never stored in the policy JSON: credentials are
//! resolved from Windows Credential Manager per invocation, referenced here
//! only by target name.
//!
//! Lifecycle: begin in Learning mode, inspect the activity report, then derive
//! an enforced least-privilege policy.

/// Policy decision for one access request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyDecision {
    Allow,
    Prompt,
    Deny,
}

use serde::{Deserialize, Serialize};

/// Filesystem access level for a path rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FsAccess {
    Read,
    Write,
    ReadWrite,
    Deny,
}

/// One filesystem rule: approved project dirs read/write, tool runtimes
/// read-only, `.ssh`/credential stores/personal data denied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsRule {
    pub path: String,
    pub access: FsAccess,
}

/// One network rule. Loopback allowances cover LiteLLM :4000 and the
/// model-server ports; named endpoints cover GitHub, Google APIs, OpenRouter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetRule {
    pub host: String,
    pub ports: Vec<u16>,
    pub decision: PolicyDecision,
}

/// A credential reference: Credential Manager target name only.
/// The value is resolved per invocation, never stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialRef {
    pub name: String,
    pub target: String,
}

/// The sandbox policy document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub version: u32,
    pub filesystem: Vec<FsRule>,
    pub network: Vec<NetRule>,
    pub credentials: Vec<CredentialRef>,
    /// Default-deny: anything not explicitly allowed is denied.
    pub default_deny: bool,
}

impl Policy {
    /// Start from default-deny with no allowances; Learning mode observes
    /// from here.
    pub fn default_policy() -> Self {
        Self {
            version: 1,
            filesystem: Vec::new(),
            network: Vec::new(),
            credentials: Vec::new(),
            default_deny: true,
        }
    }
}

/// What kind of access is being requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessKind {
    FileRead,
    FileWrite,
    Network,
}

/// One access request to decide on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessRequest {
    pub kind: AccessKind,
    pub path_or_host: String,
    pub port: Option<u16>,
}

/// Decide one request against the policy (default-deny when no rule matches).
pub fn decide(policy: &Policy, request: &AccessRequest) -> PolicyDecision {
    let _ = (policy, request);
    todo!("policy decision for {request:?}")
}

/// Validation report for a policy document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Validate a policy: no secrets embedded, paths absolute, ports in range,
/// default-deny set, no allow-all network rules.
pub fn validate(policy: &Policy) -> Result<ValidationReport, MxcError> {
    let _ = policy;
    todo!("validate policy")
}

/// One observed decision in a Learning-mode activity report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningEntry {
    pub timestamp: String,
    pub decision: PolicyDecision,
    pub description: String,
}

/// Activity report produced by a Learning-mode run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningReport {
    pub entries: Vec<LearningEntry>,
}

/// Parse a Learning-mode activity report (JSON) for UI display.
pub fn parse_learning_report(json: &str) -> Result<LearningReport, MxcError> {
    let _ = json;
    todo!("parse learning report")
}

/// MXC errors.
#[derive(Debug, thiserror::Error)]
pub enum MxcError {
    #[error("policy error: {0}")]
    Policy(String),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("runtime error: {0}")]
    Runtime(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_denies_by_default() {
        let p = Policy::default_policy();
        assert!(p.default_deny);
        assert!(p.filesystem.is_empty());
        assert!(p.network.is_empty());
        assert!(p.credentials.is_empty());
        // serde round-trip: policy JSON must never carry secret values
        let json = serde_json::to_string(&p).expect("serialize");
        assert!(json.contains("\"default_deny\":true"));
    }
}
