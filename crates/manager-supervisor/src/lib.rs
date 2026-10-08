//! Process supervision (spec §2.2, §6).
//!
//! The app supervises child processes — one `llama-server` per backend, the
//! LiteLLM gateway, local MCP servers, optionally WinMLServer — and never
//! embeds inference. The supervisor owns spawn args, health probing, graceful
//! drain (30 s timeout, then kill), exponential crash backoff (capped), and
//! stdout/stderr capture into per-scope ring buffers feeding the log bus.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::{Child, Command};

/// Health-probe result for a supervised process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy,
    Degraded(String),
    Unhealthy(String),
}

/// Pluggable health probe: HTTP endpoint, port-open check, etc.
#[async_trait]
pub trait HealthProbe: Send + Sync {
    async fn probe(&self) -> Result<HealthStatus, SupervisorError>;
}

/// HTTP health probe (e.g. `GET http://127.0.0.1:8081/health`).
#[derive(Debug, Clone)]
pub struct HttpHealthProbe {
    pub url: String,
    pub timeout: Duration,
}

#[async_trait]
impl HealthProbe for HttpHealthProbe {
    async fn probe(&self) -> Result<HealthStatus, SupervisorError> {
        todo!("GET {}", self.url)
    }
}

/// Exponential backoff policy for crash restarts.
#[derive(Debug, Clone)]
pub struct BackoffPolicy {
    pub base: Duration,
    pub max: Duration,
    pub max_retries: u32,
}

impl BackoffPolicy {
    /// Delay before retry number `attempt` (0-based), capped at `max`.
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let shift = attempt.min(10);
        self.base.saturating_mul(1u32 << shift).min(self.max)
    }

    /// `true` while another restart is allowed.
    pub fn allows_retry(&self, attempt: u32) -> bool {
        attempt < self.max_retries
    }
}

/// Which stream a captured line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogStream {
    Stdout,
    Stderr,
}

/// One captured log line, feeding the log bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRecord {
    pub ts: DateTime<Utc>,
    pub stream: LogStream,
    pub line: String,
}

/// Fixed-capacity ring buffer (default 10k lines per scope, spec §13).
#[derive(Debug)]
pub struct RingBuffer<T> {
    buf: VecDeque<T>,
    capacity: usize,
}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(capacity.min(1024)),
            capacity,
        }
    }

    pub fn push(&mut self, item: T) {
        if self.buf.len() >= self.capacity {
            self.buf.pop_front();
        }
        self.buf.push_back(item);
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Newest-first drain for UI queries.
    pub fn recent(&self, limit: usize) -> Vec<&T> {
        self.buf.iter().rev().take(limit).collect()
    }
}

/// A supervised child process.
pub struct SupervisedProcess {
    pub id: String,
    child: Child,
    log_ring: Arc<Mutex<RingBuffer<LogRecord>>>,
    restart_attempts: u32,
}

impl SupervisedProcess {
    /// Spawn `program` with `args`/`env`, capturing stdout/stderr into the
    /// shared ring buffer.
    pub async fn spawn(
        id: &str,
        program: &PathBuf,
        args: &[String],
        env: &[(String, String)],
        log_ring: Arc<Mutex<RingBuffer<LogRecord>>>,
    ) -> Result<Self, SupervisorError> {
        let _ = (program, args, env);
        let _ = Stdio::piped;
        todo!("spawn supervised process {id}")
    }

    /// Graceful drain: SIGTERM-equivalent, wait `timeout`, then kill.
    /// (30 s default per spec §2.2.)
    pub async fn drain(&mut self, timeout: Duration) -> Result<(), SupervisorError> {
        todo!("drain {} with timeout {timeout:?}", self.id)
    }

    /// Force-kill immediately.
    pub async fn kill(&mut self) -> Result<(), SupervisorError> {
        todo!("kill {}", self.id)
    }

    pub fn restart_attempts(&self) -> u32 {
        self.restart_attempts
    }
}

/// Owns all supervised processes: llama-servers, LiteLLM, MCP servers, WinML.
pub struct Supervisor {
    processes: HashMap<String, SupervisedProcess>,
    backoff: BackoffPolicy,
}

use std::collections::HashMap;

impl Supervisor {
    pub fn new(backoff: BackoffPolicy) -> Self {
        todo!("new supervisor with {backoff:?}")
    }

    pub async fn spawn_backend(&mut self, id: &str) -> Result<(), SupervisorError> {
        todo!("spawn backend {id}")
    }

    /// Stop a process; graceful drain first when `graceful` is true.
    pub async fn stop(&mut self, id: &str, graceful: bool) -> Result<(), SupervisorError> {
        todo!("stop {id} graceful={graceful}")
    }

    /// Graceful stop then spawn again.
    pub async fn restart(&mut self, id: &str, graceful: bool) -> Result<(), SupervisorError> {
        todo!("restart {id} graceful={graceful}")
    }

    /// Exact argv the supervisor would spawn for `id` (wizard "Show commands"
    /// and backend "launch command" UI must equal this).
    pub fn launch_argv(&self, id: &str) -> Result<Vec<String>, SupervisorError> {
        todo!("launch argv for {id}")
    }
}

/// Supervisor errors.
#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    #[error("spawn failed for {0}: {1}")]
    SpawnFailed(String, String),
    #[error("process not found: {0}")]
    NotFound(String),
    #[error("drain timed out for {0}")]
    DrainTimeout(String),
    #[error("health probe failed for {0}: {1}")]
    ProbeFailed(String, String),
    #[error("restart budget exhausted for {0}")]
    RestartBudgetExhausted(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_and_ring_buffer_behave() {
        let policy = BackoffPolicy {
            base: Duration::from_secs(1),
            max: Duration::from_secs(30),
            max_retries: 5,
        };
        assert_eq!(policy.delay_for_attempt(0), Duration::from_secs(1));
        assert_eq!(policy.delay_for_attempt(1), Duration::from_secs(2));
        assert_eq!(policy.delay_for_attempt(2), Duration::from_secs(4));
        assert_eq!(policy.delay_for_attempt(10), Duration::from_secs(30));
        assert!(policy.allows_retry(4));
        assert!(!policy.allows_retry(5));

        let mut ring = RingBuffer::new(3);
        for i in 0..5 {
            ring.push(i);
        }
        assert_eq!(ring.len(), 3);
        let recent: Vec<i32> = ring.recent(3).into_iter().copied().collect();
        assert_eq!(recent, vec![4, 3, 2]);
    }
}
