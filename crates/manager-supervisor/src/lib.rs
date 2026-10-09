//! Process supervision (spec §2.2, §6).
//!
//! The app supervises child processes — one `llama-server` per backend, the
//! LiteLLM gateway, local MCP servers, optionally WinMLServer — and never
//! embeds inference. The supervisor owns spawn args, health probing, graceful
//! drain (30 s default, then kill), exponential crash backoff (capped), and
//! stdout/stderr capture into per-scope ring buffers feeding the log bus.
//!
//! Design notes:
//! - `SupervisedProcess` is a single live child. `Supervisor` owns the
//!   registry (`ProcessSpec` per id), the live map, crash budgets, health
//!   probes and 3-strike/2-pass health trackers.
//! - `drain()` returns `Ok` once the process is stopped — including when the
//!   graceful window expired and the process had to be killed. The desired end
//!   state (stopped) is what the caller asked for; a note is pushed to the
//!   log ring when the kill path was taken. `DrainTimeout` is reserved for the
//!   pathological case where even the kill fails.
//! - Graceful signal is SIGTERM on Unix. On Windows v1 there is no console-
//!   control delivery wired yet, so `drain()` waits the timeout for natural
//!   exit and then kills (future work: GenerateConsoleCtrlEvent).
//! - Children are spawned with `kill_on_drop(true)`: dropping a
//!   `SupervisedProcess` without an explicit stop still reaps the child.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

/// Graceful-drain timeout used by `Supervisor::stop(id, graceful = true)`
/// (spec §2.2: 30 s, then kill).
pub const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

/// Upper bound for reaping a child after the kill in `drain()`. If the
/// child is still alive past this, `drain()` returns `DrainTimeout`
/// instead of blocking the supervisor forever.
pub const KILL_REAP_TIMEOUT: Duration = Duration::from_secs(5);

/// Default ring-buffer capacity per supervised process (spec §2.4: 10k lines).
pub const DEFAULT_LOG_RING_CAPACITY: usize = 10_000;

/// Lifecycle state per backend (spec §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

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
///
/// 2xx → Healthy; other status codes → Degraded; connection errors and
/// timeouts → Unhealthy.
#[derive(Debug, Clone)]
pub struct HttpHealthProbe {
    pub url: String,
    pub timeout: Duration,
    client: reqwest::Client,
}

impl HttpHealthProbe {
    pub fn new(url: impl Into<String>, timeout: Duration) -> Self {
        Self {
            url: url.into(),
            timeout,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl HealthProbe for HttpHealthProbe {
    async fn probe(&self) -> Result<HealthStatus, SupervisorError> {
        match tokio::time::timeout(self.timeout, self.client.get(&self.url).send()).await {
            Err(_) => Ok(HealthStatus::Unhealthy(format!(
                "probe timed out after {:?}",
                self.timeout
            ))),
            Ok(Err(e)) => Ok(HealthStatus::Unhealthy(format!(
                "probe request failed: {e}"
            ))),
            Ok(Ok(resp)) => {
                let status = resp.status();
                if status.is_success() {
                    Ok(HealthStatus::Healthy)
                } else {
                    Ok(HealthStatus::Degraded(format!("HTTP {}", status.as_u16())))
                }
            }
        }
    }
}

/// 3-strike / 2-pass flapping guard (spec §6.2): 3 consecutive failures move
/// the displayed status to the failure; recovery requires 2 consecutive
/// passes. Intermediate observations keep the previous displayed status so
/// the UI does not flap.
#[derive(Debug, Clone)]
pub struct HealthTracker {
    consecutive_failures: u32,
    consecutive_passes: u32,
    displayed: HealthStatus,
}

impl HealthTracker {
    pub fn new() -> Self {
        Self {
            consecutive_failures: 0,
            consecutive_passes: 0,
            displayed: HealthStatus::Healthy,
        }
    }

    /// Feed one probe result; returns the status the UI should display.
    pub fn observe(&mut self, result: HealthStatus) -> HealthStatus {
        match &result {
            HealthStatus::Healthy => {
                self.consecutive_failures = 0;
                self.consecutive_passes += 1;
                if self.consecutive_passes >= 2 {
                    self.displayed = HealthStatus::Healthy;
                }
            }
            other => {
                self.consecutive_passes = 0;
                self.consecutive_failures += 1;
                if self.consecutive_failures >= 3 {
                    self.displayed = other.clone();
                }
            }
        }
        self.displayed.clone()
    }

    pub fn displayed(&self) -> &HealthStatus {
        &self.displayed
    }
}

impl Default for HealthTracker {
    fn default() -> Self {
        Self::new()
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
    /// The shift saturates so large attempt counts cannot overflow.
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

impl LogRecord {
    pub fn new(stream: LogStream, line: impl Into<String>) -> Self {
        Self {
            ts: Utc::now(),
            stream,
            line: line.into(),
        }
    }
}

/// Fixed-capacity ring buffer (default 10k lines per scope, spec §2.4).
/// When full, the oldest record is evicted on push.
#[derive(Debug)]
pub struct RingBuffer<T> {
    buf: VecDeque<T>,
    capacity: usize,
}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
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

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Newest-first view for UI queries.
    pub fn recent(&self, limit: usize) -> Vec<&T> {
        self.buf.iter().rev().take(limit).collect()
    }
}

/// Static launch description for one supervised process. The supervisor
/// spawns exactly `program + args` with `env`/`workdir`; `launch_argv()`
/// exposes the same argv the wizard's "Show commands" displays.
#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub id: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub workdir: Option<PathBuf>,
    /// Optional TCP port checked for conflicts before spawn (spec §6.2).
    pub port: Option<u16>,
}

impl ProcessSpec {
    /// Full argv: `[program, args...]`.
    pub fn argv(&self) -> Vec<String> {
        let mut v = Vec::with_capacity(self.args.len() + 1);
        v.push(self.program.to_string_lossy().into_owned());
        v.extend(self.args.iter().cloned());
        v
    }
}

/// Point-in-time snapshot of one registered process for the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessStatus {
    pub id: String,
    pub state: ProcessState,
    pub pid: Option<u32>,
    /// Crash-restart attempts since the last healthy observation or manual
    /// (re)start.
    pub restarts: u32,
    pub uptime: Option<Duration>,
}

/// Send the graceful-termination signal. Unix: SIGTERM. Windows v1: no-op —
/// `drain()` still waits the timeout for natural exit before killing.
fn send_graceful_signal(child: &Child) {
    #[cfg(unix)]
    {
        if let Some(pid) = child.id() {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGTERM,
            );
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child;
    }
}

fn spawn_reader<R>(
    stream: R,
    which: LogStream,
    ring: Arc<Mutex<RingBuffer<LogRecord>>>,
) -> JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        // read_until + from_utf8_lossy: a non-UTF8 byte sequence must not
        // kill the reader (BufRead::lines would end the loop on the first
        // decode error and silently drop all subsequent output).
        let mut reader = BufReader::new(stream);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) => break, // EOF
                Ok(_) => {
                    let text = String::from_utf8_lossy(&buf);
                    let line = text.strip_suffix('\n').unwrap_or(&text);
                    let line = line.strip_suffix('\r').unwrap_or(line);
                    let rec = LogRecord::new(which, line.to_string());
                    if let Ok(mut r) = ring.lock() {
                        r.push(rec);
                    }
                }
                Err(_) => break,
            }
        }
    })
}

/// A supervised child process: one live OS process with captured output.
pub struct SupervisedProcess {
    pub id: String,
    child: Option<Child>,
    readers: Vec<JoinHandle<()>>,
    log_ring: Arc<Mutex<RingBuffer<LogRecord>>>,
    state: ProcessState,
    restart_attempts: u32,
    started_at: Option<Instant>,
    last_exit: Option<std::process::ExitStatus>,
}

impl std::fmt::Debug for SupervisedProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SupervisedProcess")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("pid", &self.pid())
            .field("restart_attempts", &self.restart_attempts)
            .finish()
    }
}

impl SupervisedProcess {
    /// Spawn `program` with `args`/`env`, capturing stdout/stderr into the
    /// shared ring buffer. The child is `kill_on_drop`: dropping this struct
    /// without an explicit stop still reaps the child.
    pub async fn spawn(
        id: &str,
        program: &Path,
        args: &[String],
        env: &[(String, String)],
        log_ring: Arc<Mutex<RingBuffer<LogRecord>>>,
    ) -> Result<Self, SupervisorError> {
        Self::spawn_with_workdir(id, program, args, env, None, log_ring).await
    }

    /// `spawn` plus an explicit working directory.
    pub async fn spawn_with_workdir(
        id: &str,
        program: &Path,
        args: &[String],
        env: &[(String, String)],
        workdir: Option<&Path>,
        log_ring: Arc<Mutex<RingBuffer<LogRecord>>>,
    ) -> Result<Self, SupervisorError> {
        let mut cmd = Command::new(program);
        cmd.args(args);
        for (k, v) in env {
            cmd.env(k, v);
        }
        if let Some(w) = workdir {
            cmd.current_dir(w);
        }
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .map_err(|e| SupervisorError::SpawnFailed(id.to_string(), e.to_string()))?;
        tracing::info!(process = id, pid = child.id(), "spawned supervised process");

        let mut readers = Vec::with_capacity(2);
        if let Some(out) = child.stdout.take() {
            readers.push(spawn_reader(out, LogStream::Stdout, Arc::clone(&log_ring)));
        }
        if let Some(err) = child.stderr.take() {
            readers.push(spawn_reader(err, LogStream::Stderr, Arc::clone(&log_ring)));
        }

        Ok(Self {
            id: id.to_string(),
            child: Some(child),
            readers,
            log_ring,
            state: ProcessState::Running,
            restart_attempts: 0,
            started_at: Some(Instant::now()),
            last_exit: None,
        })
    }

    /// Graceful drain: send the graceful signal, wait `timeout` for natural
    /// exit, then kill. Returns `Ok` once the process is stopped either way.
    pub async fn drain(&mut self, timeout: Duration) -> Result<(), SupervisorError> {
        let mut child = match self.child.take() {
            None => {
                self.state = ProcessState::Stopped;
                return Ok(());
            }
            Some(c) => c,
        };

        // Fast path: already exited.
        match child.try_wait() {
            Ok(Some(status)) => {
                self.record_exit(status, ProcessState::Stopped);
                self.abort_readers();
                return Ok(());
            }
            Ok(None) => {}
            Err(e) => {
                self.child = Some(child);
                return Err(SupervisorError::Io(e));
            }
        }

        self.state = ProcessState::Stopping;
        send_graceful_signal(&child);

        match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(status)) => {
                tracing::info!(process = self.id, "drained gracefully");
                self.record_exit(status, ProcessState::Stopped);
                self.abort_readers();
                Ok(())
            }
            Ok(Err(e)) => {
                self.child = Some(child);
                Err(SupervisorError::Io(e))
            }
            Err(_) => {
                // Graceful window expired: kill and reap.
                self.note(
                    LogStream::Stderr,
                    format!("drain timed out after {timeout:?}; killing process"),
                );
                tracing::warn!(process = self.id, "drain timed out; killing");
                let _ = child.kill().await;
                // The kill is best-effort: if the child still will not die,
                // report DrainTimeout instead of hanging the supervisor on
                // wait() forever.
                match tokio::time::timeout(KILL_REAP_TIMEOUT, child.wait()).await {
                    Ok(Ok(status)) => {
                        self.record_exit(status, ProcessState::Stopped);
                        self.abort_readers();
                        Ok(())
                    }
                    _ => {
                        self.record_exit_unknown();
                        self.abort_readers();
                        // `child` drops here; kill_on_drop makes one last
                        // best-effort attempt. The process is still alive,
                        // which is the pathological case DrainTimeout exists for.
                        Err(SupervisorError::DrainTimeout(self.id.clone()))
                    }
                }
            }
        }
    }

    /// Force-kill immediately and reap.
    pub async fn kill(&mut self) -> Result<(), SupervisorError> {
        let mut child = match self.child.take() {
            None => {
                self.state = ProcessState::Stopped;
                return Ok(());
            }
            Some(c) => c,
        };
        // May already be dead; ignore the error and reap regardless.
        let _ = child.kill().await;
        match child.wait().await {
            Ok(status) => self.record_exit(status, ProcessState::Stopped),
            Err(_) => self.record_exit_unknown(),
        }
        self.abort_readers();
        tracing::info!(process = self.id, "killed");
        Ok(())
    }

    /// Non-blocking check: reaps the child if it has exited. Returns the
    /// exit status on transition, `None` while still running. An exit
    /// observed here is unexpected (explicit stops remove the process from
    /// the supervisor first), so the state becomes `Failed`.
    pub fn poll_exited(&mut self) -> Result<Option<std::process::ExitStatus>, SupervisorError> {
        let Some(child) = self.child.as_mut() else {
            return Ok(None);
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                self.child = None;
                self.last_exit = Some(status);
                self.state = ProcessState::Failed;
                self.abort_readers();
                Ok(Some(status))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(SupervisorError::Io(e)),
        }
    }

    /// Record a crash against the backoff policy. Returns the delay before
    /// the next restart attempt, or `RestartBudgetExhausted`.
    pub fn note_crash(&mut self, backoff: &BackoffPolicy) -> Result<Duration, SupervisorError> {
        if !backoff.allows_retry(self.restart_attempts) {
            return Err(SupervisorError::RestartBudgetExhausted(self.id.clone()));
        }
        let delay = backoff.delay_for_attempt(self.restart_attempts);
        self.restart_attempts += 1;
        Ok(delay)
    }

    /// Reset the crash budget after a healthy observation.
    pub fn note_healthy(&mut self) {
        self.restart_attempts = 0;
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn state(&self) -> ProcessState {
        self.state
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(|c| c.id())
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    pub fn uptime(&self) -> Option<Duration> {
        self.started_at.map(|t| t.elapsed())
    }

    pub fn restart_attempts(&self) -> u32 {
        self.restart_attempts
    }

    pub fn last_exit(&self) -> Option<std::process::ExitStatus> {
        self.last_exit
    }

    pub fn log_ring(&self) -> Arc<Mutex<RingBuffer<LogRecord>>> {
        Arc::clone(&self.log_ring)
    }

    fn record_exit(&mut self, status: std::process::ExitStatus, state: ProcessState) {
        self.last_exit = Some(status);
        self.state = state;
        self.started_at = None;
    }

    fn record_exit_unknown(&mut self) {
        self.state = ProcessState::Stopped;
        self.started_at = None;
    }

    fn abort_readers(&mut self) {
        for h in self.readers.drain(..) {
            h.abort();
        }
    }

    fn note(&self, stream: LogStream, line: String) {
        if let Ok(mut r) = self.log_ring.lock() {
            r.push(LogRecord::new(stream, line));
        }
    }
}

fn port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// Owns all supervised processes: llama-servers, LiteLLM, MCP servers, WinML.
pub struct Supervisor {
    specs: HashMap<String, ProcessSpec>,
    live: HashMap<String, SupervisedProcess>,
    /// Retained log rings of stopped/reaped processes so `logs_query` keeps
    /// working after a process exits. Replaced on the next spawn of the id.
    dead_rings: HashMap<String, Arc<Mutex<RingBuffer<LogRecord>>>>,
    probes: HashMap<String, Arc<dyn HealthProbe>>,
    trackers: HashMap<String, HealthTracker>,
    failed_ids: HashSet<String>,
    crash_counts: HashMap<String, u32>,
    backoff: BackoffPolicy,
}

impl Supervisor {
    pub fn new(backoff: BackoffPolicy) -> Self {
        Self {
            specs: HashMap::new(),
            live: HashMap::new(),
            dead_rings: HashMap::new(),
            probes: HashMap::new(),
            trackers: HashMap::new(),
            failed_ids: HashSet::new(),
            crash_counts: HashMap::new(),
            backoff,
        }
    }

    pub fn backoff(&self) -> &BackoffPolicy {
        &self.backoff
    }

    /// Register (or replace) the launch spec for `id`. The spec is what
    /// `spawn_backend` spawns and what `launch_argv` reports.
    pub fn register_spec(&mut self, spec: ProcessSpec) {
        self.specs.insert(spec.id.clone(), spec);
    }

    pub fn spec_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.specs.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Attach a health probe for `id`.
    pub fn register_probe(&mut self, id: &str, probe: Arc<dyn HealthProbe>) {
        self.probes.insert(id.to_string(), probe);
    }

    /// Spawn the registered backend `id` (spec §6.2: port-conflict check
    /// before spawn). A manual spawn resets the crash budget for `id`.
    pub async fn spawn_backend(&mut self, id: &str) -> Result<(), SupervisorError> {
        if self.live.contains_key(id) {
            return Err(SupervisorError::AlreadyRunning(id.to_string()));
        }
        let spec = self
            .specs
            .get(id)
            .ok_or_else(|| SupervisorError::NotFound(id.to_string()))?
            .clone();
        if let Some(port) = spec.port {
            if !port_free(port) {
                return Err(SupervisorError::PortInUse(port));
            }
        }
        let ring = Arc::new(Mutex::new(RingBuffer::new(DEFAULT_LOG_RING_CAPACITY)));
        let proc = SupervisedProcess::spawn_with_workdir(
            id,
            &spec.program,
            &spec.args,
            &spec.env,
            spec.workdir.as_deref(),
            ring,
        )
        .await?;
        self.live.insert(id.to_string(), proc);
        self.dead_rings.remove(id);
        self.failed_ids.remove(id);
        self.crash_counts.insert(id.to_string(), 0);
        self.trackers.entry(id.to_string()).or_default();
        Ok(())
    }

    /// Stop a process; graceful drain first when `graceful` is true.
    /// The process's log ring is retained so `logs_query` still serves its
    /// history until the id is spawned again.
    pub async fn stop(&mut self, id: &str, graceful: bool) -> Result<(), SupervisorError> {
        let mut proc = self
            .live
            .remove(id)
            .ok_or_else(|| SupervisorError::NotFound(id.to_string()))?;
        if graceful {
            proc.drain(DEFAULT_DRAIN_TIMEOUT).await?;
        } else {
            proc.kill().await?;
        }
        self.dead_rings.insert(id.to_string(), proc.log_ring());
        Ok(())
    }

    /// Graceful stop then spawn again. Unknown ids are a fresh start.
    pub async fn restart(&mut self, id: &str, graceful: bool) -> Result<(), SupervisorError> {
        if self.live.contains_key(id) {
            self.stop(id, graceful).await?;
        }
        self.spawn_backend(id).await
    }

    /// Non-blocking reap of unexpectedly exited children. Returns
    /// `(id, exit_status)` for each; reaped ids show as `Failed` in
    /// `status()` until respawned. Callers apply `crash_delay()` for
    /// backoff before respawning.
    pub fn reap_exited(&mut self) -> Vec<(String, std::process::ExitStatus)> {
        let mut dead = Vec::new();
        let ids: Vec<String> = self.live.keys().cloned().collect();
        for id in ids {
            let exited = self
                .live
                .get_mut(&id)
                .and_then(|p| p.poll_exited().ok().flatten());
            if let Some(status) = exited {
                if let Some(proc) = self.live.remove(&id) {
                    self.dead_rings.insert(id.clone(), proc.log_ring());
                }
                self.failed_ids.insert(id.clone());
                tracing::warn!(
                    process = id,
                    ?status,
                    "supervised process exited unexpectedly"
                );
                dead.push((id, status));
            }
        }
        dead
    }

    /// Crash-budget accounting for `id`: returns the backoff delay before the
    /// next restart, or `RestartBudgetExhausted`. Call `note_healthy(id)`
    /// when the process is observed healthy to reset the budget.
    pub fn crash_delay(&mut self, id: &str) -> Result<Duration, SupervisorError> {
        if !self.specs.contains_key(id) {
            return Err(SupervisorError::NotFound(id.to_string()));
        }
        let attempts = self.crash_counts.get(id).copied().unwrap_or(0);
        if !self.backoff.allows_retry(attempts) {
            return Err(SupervisorError::RestartBudgetExhausted(id.to_string()));
        }
        self.crash_counts.insert(id.to_string(), attempts + 1);
        Ok(self.backoff.delay_for_attempt(attempts))
    }

    /// Reset the crash budget for `id` after a healthy observation.
    pub fn note_healthy(&mut self, id: &str) {
        self.crash_counts.insert(id.to_string(), 0);
    }

    /// Raw probe call for `id` (no flapping guard).
    pub async fn probe(&self, id: &str) -> Result<HealthStatus, SupervisorError> {
        let probe = self
            .probes
            .get(id)
            .ok_or_else(|| SupervisorError::NotFound(format!("no probe registered for {id}")))?;
        probe.probe().await
    }

    /// Probe `id` through its 3-strike/2-pass tracker; returns the status the
    /// UI should display.
    pub async fn probe_tracked(&mut self, id: &str) -> Result<HealthStatus, SupervisorError> {
        let raw = self.probe(id).await?;
        let tracker = self.trackers.entry(id.to_string()).or_default();
        Ok(tracker.observe(raw))
    }

    /// Status snapshot over all registered specs, sorted by id.
    pub fn status(&self) -> Vec<ProcessStatus> {
        let mut out: Vec<ProcessStatus> = self
            .specs
            .keys()
            .map(|id| {
                let restarts = self.crash_counts.get(id).copied().unwrap_or(0);
                if let Some(proc) = self.live.get(id) {
                    ProcessStatus {
                        id: id.clone(),
                        state: proc.state(),
                        pid: proc.pid(),
                        restarts,
                        uptime: proc.uptime(),
                    }
                } else if self.failed_ids.contains(id) {
                    ProcessStatus {
                        id: id.clone(),
                        state: ProcessState::Failed,
                        pid: None,
                        restarts,
                        uptime: None,
                    }
                } else {
                    ProcessStatus {
                        id: id.clone(),
                        state: ProcessState::Stopped,
                        pid: None,
                        restarts,
                        uptime: None,
                    }
                }
            })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Exact argv the supervisor would spawn for `id` (wizard "Show commands"
    /// and backend "launch command" UI must equal this).
    pub fn launch_argv(&self, id: &str) -> Result<Vec<String>, SupervisorError> {
        self.specs
            .get(id)
            .map(|s| s.argv())
            .ok_or_else(|| SupervisorError::NotFound(id.to_string()))
    }

    /// Query captured logs across scopes (spec §14 `logs_query`).
    ///
    /// `scope` is a process id or `"all"`. `level` is an optional
    /// error/warn/info/debug filter; llama-server emits no structured
    /// levels, so this is a documented heuristic substring match on the
    /// line. `search` is an optional case-insensitive substring. Returns
    /// newest-first, capped at `limit`. Covers live processes and retained
    /// rings of stopped/reaped ones.
    pub fn logs_query(
        &self,
        scope: &str,
        level: Option<&str>,
        search: Option<&str>,
        limit: usize,
    ) -> Vec<LogRecord> {
        let level_filter = level.and_then(LogLevelFilter::parse);
        let search_lc = search.map(|s| s.to_lowercase());
        let mut ids: Vec<&String> = if scope == "all" {
            let mut all: Vec<&String> = self.live.keys().chain(self.dead_rings.keys()).collect();
            all.sort();
            all.dedup();
            all
        } else {
            let mut one: Vec<&String> = Vec::new();
            if let Some(id) = self.live.keys().find(|k| k.as_str() == scope) {
                one.push(id);
            } else if let Some(id) = self.dead_rings.keys().find(|k| k.as_str() == scope) {
                one.push(id);
            }
            one
        };
        // Deterministic scope order for "all".
        ids.sort();
        let mut out: Vec<LogRecord> = Vec::new();
        for id in ids {
            let ring = self
                .live
                .get(id)
                .map(|p| p.log_ring())
                .or_else(|| self.dead_rings.get(id).cloned());
            let Some(ring) = ring else { continue };
            let Ok(guard) = ring.lock() else { continue };
            for rec in guard.recent(usize::MAX) {
                if let Some(lf) = level_filter {
                    if !lf.matches_line(&rec.line) {
                        continue;
                    }
                }
                if let Some(ref needle) = search_lc {
                    if !rec.line.to_lowercase().contains(needle) {
                        continue;
                    }
                }
                out.push(rec.clone());
                if out.len() >= limit.max(1) {
                    break;
                }
            }
            if out.len() >= limit.max(1) {
                break;
            }
        }
        out
    }
}

/// Log-level filter for [`Supervisor::logs_query`].
///
/// llama-server has no structured log levels; matching is a heuristic over
/// well-known tokens in the line. `Error` also matches stderr-origin lines
/// via "error"/"fail"/"panic"/"fatal"; `Warn` matches "warn"; `Info`
/// matches "info"; `Debug` matches "debug"/"trace".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevelFilter {
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevelFilter {
    /// Parse a UI-supplied level string; unknown strings are `None`
    /// (caller treats as no filter).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "error" | "err" | "fatal" => Some(Self::Error),
            "warn" | "warning" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "debug" | "trace" => Some(Self::Debug),
            _ => None,
        }
    }

    fn matches_line(&self, line: &str) -> bool {
        let lc = line.to_lowercase();
        match self {
            Self::Error => ["error", "fail", "panic", "fatal"]
                .iter()
                .any(|t| lc.contains(t)),
            Self::Warn => lc.contains("warn"),
            Self::Info => lc.contains("info"),
            Self::Debug => lc.contains("debug") || lc.contains("trace"),
        }
    }
}

/// Result of a backend test request (spec §14 `backend_test_request`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub status: u16,
    pub content: Option<String>,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub latency_ms: u64,
}

/// Minimal inference test request: HTTP POST to
/// `{base_url}/v1/chat/completions` with a tiny prompt, returning the first
/// choice's content plus usage when the server reports it.
///
/// `base_url` is e.g. `http://127.0.0.1:8081`. Non-2xx responses are not
/// errors here — the status is reported and content is `None` — but
/// transport failures and unparseable bodies are.
pub async fn test_request(base_url: &str, prompt: &str) -> Result<TestResult, SupervisorError> {
    let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": "test",
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 16,
        "stream": false,
    });
    // NOTE: built without reqwest's `json` feature (workspace keeps reqwest
    // minimal); body serialization and response parsing go through
    // serde_json directly.
    let body_str =
        serde_json::to_string(&body).map_err(|e| SupervisorError::Http(e.to_string()))?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| SupervisorError::Http(e.to_string()))?;
    let start = Instant::now();
    let resp = client
        .post(&url)
        .header("content-type", "application/json")
        .body(body_str)
        .send()
        .await
        .map_err(|e| SupervisorError::Http(e.to_string()))?;
    let status = resp.status().as_u16();
    let latency_ms = start.elapsed().as_millis() as u64;
    let text = resp
        .text()
        .await
        .map_err(|e| SupervisorError::Http(e.to_string()))?;
    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| SupervisorError::Http(format!("unparseable response body: {e}")))?;
    let content = json
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let prompt_tokens = json
        .pointer("/usage/prompt_tokens")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    let completion_tokens = json
        .pointer("/usage/completion_tokens")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    Ok(TestResult {
        status,
        content,
        prompt_tokens,
        completion_tokens,
        latency_ms,
    })
}

/// Supervisor errors.
#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    #[error("spawn failed for {0}: {1}")]
    SpawnFailed(String, String),
    #[error("process not found: {0}")]
    NotFound(String),
    #[error("process already running: {0}")]
    AlreadyRunning(String),
    #[error("port {0} is already in use")]
    PortInUse(u16),
    #[error("drain timed out for {0}")]
    DrainTimeout(String),
    #[error("health probe failed for {0}: {1}")]
    ProbeFailed(String, String),
    #[error("restart budget exhausted for {0}")]
    RestartBudgetExhausted(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP error: {0}")]
    Http(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener as StdTcpListener;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn test_policy() -> BackoffPolicy {
        BackoffPolicy {
            base: ms(100),
            max: Duration::from_secs(5),
            max_retries: 3,
        }
    }

    fn test_ring() -> Arc<Mutex<RingBuffer<LogRecord>>> {
        Arc::new(Mutex::new(RingBuffer::new(1024)))
    }

    /// NOTE: `exec` replaces the shell so signals hit the real child and no
    /// orphaned grandchildren survive the test.
    #[cfg(unix)]
    fn sh_spec(id: &str, script: &str) -> ProcessSpec {
        ProcessSpec {
            id: id.into(),
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), script.into()],
            env: vec![],
            workdir: None,
            port: None,
        }
    }

    async fn wait_for<F>(mut cond: F, timeout: Duration)
    where
        F: FnMut() -> bool,
    {
        let start = Instant::now();
        loop {
            if cond() {
                return;
            }
            assert!(
                start.elapsed() < timeout,
                "condition not met within {timeout:?}"
            );
            tokio::time::sleep(ms(20)).await;
        }
    }

    fn ring_lines_chronological(ring: &Arc<Mutex<RingBuffer<LogRecord>>>) -> Vec<String> {
        ring.lock()
            .unwrap()
            .recent(usize::MAX)
            .into_iter()
            .rev()
            .map(|r| r.line.clone())
            .collect()
    }

    async fn serve_once(status_code: u16) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = tokio::io::AsyncReadExt::read(&mut sock, &mut buf).await;
                let body = "ok";
                let resp = format!(
                    "HTTP/1.1 {status_code} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tokio::io::AsyncWriteExt::write_all(&mut sock, resp.as_bytes()).await;
            }
        });
        port
    }

    struct StubProbe(HealthStatus);

    #[async_trait]
    impl HealthProbe for StubProbe {
        async fn probe(&self) -> Result<HealthStatus, SupervisorError> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn backoff_delay_sequence_caps_and_allows_retry() {
        let p = test_policy();
        assert_eq!(p.delay_for_attempt(0), ms(100));
        assert_eq!(p.delay_for_attempt(1), ms(200));
        assert_eq!(p.delay_for_attempt(2), ms(400));
        // Capped at max, and the shift saturates (no overflow on huge attempts).
        assert_eq!(p.delay_for_attempt(10), Duration::from_secs(5));
        assert_eq!(p.delay_for_attempt(u32::MAX), Duration::from_secs(5));
        assert!(p.allows_retry(0));
        assert!(p.allows_retry(2));
        assert!(!p.allows_retry(3));
    }

    #[test]
    fn ring_buffer_overwrites_oldest_newest_first() {
        let mut ring = RingBuffer::new(3);
        for i in 0..5 {
            ring.push(i);
        }
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.capacity(), 3);
        let recent: Vec<i32> = ring.recent(10).into_iter().copied().collect();
        assert_eq!(recent, vec![4, 3, 2]);
        // Zero capacity is clamped to 1 rather than panicking.
        let mut z = RingBuffer::new(0);
        z.push(1);
        assert_eq!(z.len(), 1);
    }

    #[test]
    fn log_record_carries_stream_and_timestamp() {
        let r = LogRecord::new(LogStream::Stderr, "boom");
        assert_eq!(r.stream, LogStream::Stderr);
        assert_eq!(r.line, "boom");
        assert!(r.ts <= Utc::now());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn spawn_captures_stdout_and_stderr() {
        let ring = test_ring();
        let mut proc = SupervisedProcess::spawn(
            "echo-test",
            &PathBuf::from("/bin/sh"),
            &[
                "-c".to_string(),
                "echo out-line; echo err-line >&2".to_string(),
            ],
            &[],
            ring.clone(),
        )
        .await
        .unwrap();

        wait_for(|| ring.lock().unwrap().len() >= 2, Duration::from_secs(5)).await;
        let lines = ring_lines_chronological(&ring);
        assert_eq!(lines, vec!["out-line".to_string(), "err-line".to_string()]);

        let streams: Vec<LogStream> = ring
            .lock()
            .unwrap()
            .recent(10)
            .into_iter()
            .map(|r| r.stream)
            .collect();
        assert!(streams.contains(&LogStream::Stdout));
        assert!(streams.contains(&LogStream::Stderr));

        // Already exited; kill/drain must still be Ok and idempotent.
        proc.kill().await.unwrap();
        assert!(!proc.is_running());
        proc.drain(ms(100)).await.unwrap();
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn spawn_with_workdir_sets_cwd() {
        let ring = test_ring();
        let mut proc = SupervisedProcess::spawn_with_workdir(
            "pwd-test",
            Path::new("/bin/sh"),
            &["-c".to_string(), "pwd".to_string()],
            &[],
            Some(Path::new("/tmp")),
            ring.clone(),
        )
        .await
        .unwrap();
        wait_for(|| !ring.lock().unwrap().is_empty(), Duration::from_secs(5)).await;
        let lines = ring_lines_chronological(&ring);
        assert!(lines.iter().any(|l| l == "/tmp"), "lines: {lines:?}");
        proc.kill().await.unwrap();
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn spawn_failure_reports_program() {
        let err = SupervisedProcess::spawn(
            "missing",
            &PathBuf::from("/nonexistent/binary-xyz"),
            &[],
            &[],
            test_ring(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SupervisorError::SpawnFailed(id, _) if id == "missing"));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn drain_graceful_terminates_quickly() {
        let ring = test_ring();
        let mut proc = SupervisedProcess::spawn(
            "sleepy",
            &PathBuf::from("/bin/sh"),
            &["-c".to_string(), "exec sleep 30".to_string()],
            &[],
            ring,
        )
        .await
        .unwrap();
        assert!(proc.is_running());
        assert!(proc.pid().is_some());

        let start = Instant::now();
        proc.drain(Duration::from_secs(5)).await.unwrap();
        // SIGTERM kills `sleep` immediately: well under the 5 s budget.
        assert!(start.elapsed() < Duration::from_secs(5));
        assert!(!proc.is_running());
        assert_eq!(proc.state(), ProcessState::Stopped);
        assert!(proc.uptime().is_none());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn drain_timeout_kills_term_ignoring_process() {
        let ring = test_ring();
        let mut proc = SupervisedProcess::spawn(
            "stubborn",
            &PathBuf::from("/bin/sh"),
            &[
                "-c".to_string(),
                // `ready` is printed AFTER the trap is installed, so SIGTERM
                // is guaranteed ignored from here on (the disposition
                // survives `exec` per POSIX). Without this handshake the
                // test races the child's startup.
                "trap '' TERM; echo ready; exec sleep 30".to_string(),
            ],
            &[],
            ring.clone(),
        )
        .await
        .unwrap();

        wait_for(
            || ring_lines_chronological(&ring).iter().any(|l| l == "ready"),
            Duration::from_secs(5),
        )
        .await;

        // Premise check: the child must be alive AND actually ignoring
        // SIGTERM before we measure drain's timing. `trap '' TERM` is
        // installed before `echo ready`, and SIG_IGN survives `exec` per
        // POSIX, so a live child at this point is provably TERM-ignoring.
        // If the child is already gone (e.g. a sandbox that blocks
        // `exec sleep`), the scenario cannot run here: fail with the
        // premise spelled out, never as a confusing "drain returned too
        // fast" timing flake.
        let pid = proc
            .pid()
            .expect("child must be alive after the ready handshake");
        let raw = nix::unistd::Pid::from_raw(pid as i32);
        nix::sys::signal::kill(raw, nix::sys::signal::Signal::SIGTERM)
            .expect("premise probe: SIGTERM delivery to the child must succeed");
        tokio::time::sleep(ms(150)).await;
        assert!(
            nix::sys::signal::kill(raw, None).is_ok(),
            "premise broken: TERM-ignoring child (pid {pid}) is gone before drain; \
             this environment cannot exec `sleep` or does not honor the trap, so the \
             drain-timeout scenario is unsupported here"
        );

        let start = Instant::now();
        proc.drain(ms(300)).await.unwrap();
        let elapsed = start.elapsed();
        // The full graceful window must elapse before the kill path is taken.
        assert!(elapsed >= ms(250), "drain returned too fast: {elapsed:?}");
        assert!(
            elapsed < Duration::from_secs(10),
            "drain took too long: {elapsed:?}"
        );
        assert!(!proc.is_running());

        // The kill path is observable in the log ring.
        wait_for(
            || {
                ring_lines_chronological(&ring)
                    .iter()
                    .any(|l| l.contains("drain timed out"))
            },
            Duration::from_secs(2),
        )
        .await;
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn kill_terminates_and_reaps() {
        let mut proc = SupervisedProcess::spawn(
            "killme",
            &PathBuf::from("/bin/sh"),
            &["-c".to_string(), "exec sleep 30".to_string()],
            &[],
            test_ring(),
        )
        .await
        .unwrap();
        proc.kill().await.unwrap();
        assert!(!proc.is_running());
        assert_eq!(proc.state(), ProcessState::Stopped);
        assert!(proc.last_exit().is_some());
        // Second kill is a no-op.
        proc.kill().await.unwrap();
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn process_crash_budget_counts_and_resets() {
        let policy = test_policy();
        let mut proc = SupervisedProcess::spawn(
            "budget",
            &PathBuf::from("/bin/sh"),
            &["-c".to_string(), "exec sleep 30".to_string()],
            &[],
            test_ring(),
        )
        .await
        .unwrap();

        assert_eq!(proc.note_crash(&policy).unwrap(), ms(100));
        assert_eq!(proc.note_crash(&policy).unwrap(), ms(200));
        assert_eq!(proc.restart_attempts(), 2);
        proc.note_healthy();
        assert_eq!(proc.restart_attempts(), 0);
        assert_eq!(proc.note_crash(&policy).unwrap(), ms(100));

        // Exhaust the budget: 3 allowed, 4th fails.
        proc.note_healthy();
        for _ in 0..3 {
            proc.note_crash(&policy).unwrap();
        }
        assert!(matches!(
            proc.note_crash(&policy),
            Err(SupervisorError::RestartBudgetExhausted(_))
        ));
        proc.kill().await.unwrap();
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn supervisor_lifecycle_status_and_restart() {
        let mut sup = Supervisor::new(test_policy());
        sup.register_spec(sh_spec("sleeper", "exec sleep 30"));

        // Unknown id / unknown launch argv.
        assert!(matches!(
            sup.spawn_backend("ghost").await,
            Err(SupervisorError::NotFound(_))
        ));
        assert!(matches!(
            sup.launch_argv("ghost"),
            Err(SupervisorError::NotFound(_))
        ));

        sup.spawn_backend("sleeper").await.unwrap();
        let st = sup.status();
        assert_eq!(st.len(), 1);
        assert_eq!(st[0].id, "sleeper");
        assert_eq!(st[0].state, ProcessState::Running);
        assert!(st[0].pid.is_some());
        assert!(st[0].uptime.is_some());
        assert_eq!(st[0].restarts, 0);

        // Double-spawn is rejected.
        assert!(matches!(
            sup.spawn_backend("sleeper").await,
            Err(SupervisorError::AlreadyRunning(_))
        ));

        sup.restart("sleeper", false).await.unwrap();
        assert_eq!(sup.status()[0].state, ProcessState::Running);

        sup.stop("sleeper", false).await.unwrap();
        let st = sup.status();
        assert_eq!(st[0].state, ProcessState::Stopped);
        assert!(st[0].pid.is_none());

        // Stopping an unknown id is NotFound.
        assert!(matches!(
            sup.stop("ghost", false).await,
            Err(SupervisorError::NotFound(_))
        ));
        // Restart of an unknown id is a fresh-start attempt -> NotFound (no spec).
        assert!(matches!(
            sup.restart("ghost", false).await,
            Err(SupervisorError::NotFound(_))
        ));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn launch_argv_matches_spawned_command() {
        let mut sup = Supervisor::new(test_policy());
        sup.register_spec(ProcessSpec {
            id: "llama".into(),
            program: PathBuf::from("/opt/llama/llama-server"),
            args: vec![
                "--port".into(),
                "8081".into(),
                "--tensor-split".into(),
                "0.6,0.4".into(),
            ],
            env: vec![("CUDA_VISIBLE_DEVICES".into(), "0,1".into())],
            workdir: None,
            port: Some(8081),
        });
        assert_eq!(
            sup.launch_argv("llama").unwrap(),
            vec![
                "/opt/llama/llama-server",
                "--port",
                "8081",
                "--tensor-split",
                "0.6,0.4"
            ]
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn spawn_rejected_when_port_occupied() {
        // Hold a port open for the duration of the test.
        let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let mut sup = Supervisor::new(test_policy());
        let mut spec = sh_spec("web", "exec sleep 30");
        spec.port = Some(port);
        sup.register_spec(spec);

        let err = sup.spawn_backend("web").await.unwrap_err();
        assert!(matches!(err, SupervisorError::PortInUse(p) if p == port));
        // Nothing was spawned.
        assert_eq!(sup.status()[0].state, ProcessState::Stopped);
        drop(listener);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn reap_exited_marks_failed_and_crash_delay_applies() {
        let mut sup = Supervisor::new(test_policy());
        sup.register_spec(sh_spec("quitter", "exit 3"));
        sup.spawn_backend("quitter").await.unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let dead = loop {
            let d = sup.reap_exited();
            if !d.is_empty() {
                break d;
            }
            assert!(Instant::now() < deadline, "child never exited");
            tokio::time::sleep(ms(20)).await;
        };
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].0, "quitter");
        assert!(!dead[0].1.success());

        assert_eq!(sup.status()[0].state, ProcessState::Failed);

        // Backoff sequence via the supervisor-level budget.
        assert_eq!(sup.crash_delay("quitter").unwrap(), ms(100));
        assert_eq!(sup.crash_delay("quitter").unwrap(), ms(200));
        assert_eq!(sup.crash_delay("quitter").unwrap(), ms(400));
        assert!(matches!(
            sup.crash_delay("quitter"),
            Err(SupervisorError::RestartBudgetExhausted(_))
        ));
        sup.note_healthy("quitter");
        assert_eq!(sup.crash_delay("quitter").unwrap(), ms(100));
        assert!(matches!(
            sup.crash_delay("ghost"),
            Err(SupervisorError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn http_probe_maps_status_codes() {
        let port = serve_once(200).await;
        let p = HttpHealthProbe::new(
            format!("http://127.0.0.1:{port}/health"),
            Duration::from_secs(2),
        );
        assert_eq!(p.probe().await.unwrap(), HealthStatus::Healthy);

        let port = serve_once(500).await;
        let p = HttpHealthProbe::new(
            format!("http://127.0.0.1:{port}/health"),
            Duration::from_secs(2),
        );
        assert!(matches!(
            p.probe().await.unwrap(),
            HealthStatus::Degraded(_)
        ));

        // Nothing listening: connection refused -> Unhealthy, not an error.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let p = HttpHealthProbe::new(
            format!("http://127.0.0.1:{port}/health"),
            Duration::from_secs(2),
        );
        assert!(matches!(
            p.probe().await.unwrap(),
            HealthStatus::Unhealthy(_)
        ));
    }

    #[tokio::test]
    async fn http_probe_times_out() {
        // Accept but never respond: the client timeout must fire.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (_sock, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(30)).await;
        });
        let p = HttpHealthProbe::new(format!("http://127.0.0.1:{port}/health"), ms(200));
        assert!(matches!(
            p.probe().await.unwrap(),
            HealthStatus::Unhealthy(msg) if msg.contains("timed out")
        ));
    }

    #[test]
    fn health_tracker_three_strikes_two_passes() {
        let mut t = HealthTracker::new();
        assert_eq!(t.observe(HealthStatus::Healthy), HealthStatus::Healthy);

        // Strikes 1-2: displayed status does not flap.
        assert_eq!(
            t.observe(HealthStatus::Unhealthy("x".into())),
            HealthStatus::Healthy
        );
        assert_eq!(
            t.observe(HealthStatus::Unhealthy("x".into())),
            HealthStatus::Healthy
        );
        // Strike 3: the failure is displayed.
        assert_eq!(
            t.observe(HealthStatus::Unhealthy("boom".into())),
            HealthStatus::Unhealthy("boom".into())
        );
        // One pass is not enough to recover.
        assert_eq!(
            t.observe(HealthStatus::Healthy),
            HealthStatus::Unhealthy("boom".into())
        );
        // Two consecutive passes recover.
        assert_eq!(t.observe(HealthStatus::Healthy), HealthStatus::Healthy);

        // A Degraded (non-fatal) result also counts as a strike toward display.
        let mut t2 = HealthTracker::new();
        t2.observe(HealthStatus::Healthy);
        t2.observe(HealthStatus::Degraded("warm".into()));
        t2.observe(HealthStatus::Degraded("warm".into()));
        assert_eq!(
            t2.observe(HealthStatus::Degraded("warm".into())),
            HealthStatus::Degraded("warm".into())
        );
    }

    #[tokio::test]
    async fn supervisor_probe_uses_registered_probe() {
        let mut sup = Supervisor::new(test_policy());
        sup.register_probe(
            "svc",
            Arc::new(StubProbe(HealthStatus::Degraded("warming up".into()))),
        );
        assert!(matches!(
            sup.probe("svc").await.unwrap(),
            HealthStatus::Degraded(_)
        ));
        // Tracked probing applies the flapping guard (2 strikes: still Healthy).
        assert_eq!(
            sup.probe_tracked("svc").await.unwrap(),
            HealthStatus::Healthy
        );
        assert!(matches!(
            sup.probe("ghost").await,
            Err(SupervisorError::NotFound(_))
        ));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn logs_query_filters_search_limits_and_retains_after_stop() {
        let mut sup = Supervisor::new(test_policy());
        sup.register_spec(sh_spec(
            "a",
            "echo 'alpha info: booted'; echo 'beta warn: slow disk'; echo 'gamma error: boom'",
        ));
        sup.spawn_backend("a").await.unwrap();
        wait_for(
            || sup.logs_query("a", None, None, 10).len() >= 3,
            Duration::from_secs(5),
        )
        .await;

        let all = sup.logs_query("a", None, None, 10);
        assert_eq!(all.len(), 3);
        // Newest-first.
        assert!(all[0].line.contains("gamma"));
        assert!(all[2].line.contains("alpha"));

        assert_eq!(sup.logs_query("a", Some("error"), None, 10).len(), 1);
        assert_eq!(sup.logs_query("a", Some("warn"), None, 10).len(), 1);
        assert_eq!(sup.logs_query("a", Some("info"), None, 10).len(), 1);
        // Unknown level string = no filter.
        assert_eq!(sup.logs_query("a", Some("bogus"), None, 10).len(), 3);

        let found = sup.logs_query("a", None, Some("ALPHA"), 10);
        assert_eq!(found.len(), 1);
        assert!(found[0].line.contains("alpha"));

        let limited = sup.logs_query("a", None, None, 2);
        assert_eq!(limited.len(), 2);
        assert!(limited[0].line.contains("gamma"));

        assert!(sup.logs_query("ghost", None, None, 10).is_empty());

        // Ring is retained after stop; "all" scope sees it.
        sup.stop("a", false).await.unwrap();
        let retained = sup.logs_query("a", None, None, 10);
        assert_eq!(retained.len(), 3);
        assert_eq!(sup.logs_query("all", None, None, 10).len(), 3);
    }

    /// Read one HTTP request from `sock` (headers + body per Content-Length).
    async fn read_http_request(sock: &mut tokio::net::TcpStream) -> Vec<u8> {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let body_len: usize = loop {
            let n = sock.read(&mut tmp).await.unwrap();
            assert!(n > 0, "client closed before sending a full request");
            buf.extend_from_slice(&tmp[..n]);
            if let Some(hdr_end) = find_subslice(&buf, b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&buf[..hdr_end]);
                let len = headers
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("Content-Length:")
                            .or_else(|| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if buf.len() >= hdr_end + 4 + len {
                    break len;
                }
            }
        };
        let _ = body_len;
        buf
    }

    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    /// Spawn a hermetic fake HTTP server replying `status` + `body` once.
    /// Returns the base URL. Also asserts the request targeted
    /// `/v1/chat/completions` with a JSON body.
    async fn fake_chat_server(status: u16, body: &str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = body.to_string();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let req = read_http_request(&mut sock).await;
            let head = String::from_utf8_lossy(&req);
            assert!(
                head.starts_with("POST /v1/chat/completions "),
                "unexpected request line: {}",
                head.lines().next().unwrap_or("")
            );
            assert!(head.contains("content-type: application/json"));
            let resp = format!(
                "HTTP/1.1 {status} test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            use tokio::io::AsyncWriteExt;
            sock.write_all(resp.as_bytes()).await.unwrap();
        });
        format!("http://127.0.0.1:{port}")
    }

    #[tokio::test]
    async fn test_request_parses_chat_completion() {
        let base = fake_chat_server(
            200,
            r#"{"id":"t","choices":[{"message":{"content":"hello back"}}],"usage":{"prompt_tokens":5,"completion_tokens":2}}"#,
        )
        .await;
        let r = test_request(&base, "hi").await.unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.content.as_deref(), Some("hello back"));
        assert_eq!(r.prompt_tokens, Some(5));
        assert_eq!(r.completion_tokens, Some(2));
        assert!(r.latency_ms < 5000);
    }

    #[tokio::test]
    async fn test_request_tolerates_trailing_slash_and_missing_usage() {
        let base = fake_chat_server(200, r#"{"choices":[{"message":{"content":"ok"}}]}"#).await;
        let r = test_request(&format!("{base}/"), "hi").await.unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.content.as_deref(), Some("ok"));
        assert_eq!(r.prompt_tokens, None);
    }

    #[tokio::test]
    async fn test_request_reports_non_2xx_without_content() {
        let base = fake_chat_server(500, r#"{"error":"overloaded"}"#).await;
        let r = test_request(&base, "hi").await.unwrap();
        assert_eq!(r.status, 500);
        assert!(r.content.is_none());
    }

    #[tokio::test]
    async fn test_request_transport_failure_is_http_error() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let err = test_request(&format!("http://127.0.0.1:{port}"), "hi")
            .await
            .unwrap_err();
        assert!(matches!(err, SupervisorError::Http(_)));
    }

    #[test]
    fn log_level_filter_parse_and_match() {
        assert_eq!(LogLevelFilter::parse("error"), Some(LogLevelFilter::Error));
        assert_eq!(LogLevelFilter::parse("WARN"), Some(LogLevelFilter::Warn));
        assert_eq!(LogLevelFilter::parse("bogus"), None);
        assert!(LogLevelFilter::Error.matches_line("PANIC: something failed"));
        assert!(LogLevelFilter::Warn.matches_line("warning: slow"));
        assert!(!LogLevelFilter::Warn.matches_line("all good"));
    }
}
