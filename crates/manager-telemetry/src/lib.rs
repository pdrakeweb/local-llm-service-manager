//! GPU telemetry (spec §5).
//!
//! Primary source: `nvml-wrapper` (device handles, per-process accounting).
//! Load-bearing fallback: `nvidia-smi --query --xml` parsed on NVML init
//! failure or per-GPU handle failure — the app must start and monitor
//! correctly with NVML fully unavailable (Pascal-era quirks on the P100s).
//! The UI shows the source per GPU ("NVML" / "nvidia-smi fallback").
//!
//! Poll loop: default 1 s cadence, single poller, snapshot store is the
//! single writer. Affected GPUs are marked STALE after one missed tick
//! ([`STALE_AFTER_SECS`]); last-known values stay visible, never synthesized.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Seconds without a successful poll before a GPU is labeled STALE.
pub const STALE_AFTER_SECS: u64 = 10;

/// Which source produced a sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TelemetrySourceKind {
    Nvml,
    NvidiaSmi,
}

/// Static inventory for one GPU.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuInfo {
    pub index: u32,
    pub name: String,
    pub uuid: String,
    /// e.g. (8, 6) for Ampere, (6, 0) for Pascal P100.
    pub compute_capability: (u32, u32),
    pub total_vram_mib: u64,
    pub source: TelemetrySourceKind,
}

/// One telemetry sample for one GPU.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuSample {
    pub index: u32,
    pub name: String,
    pub ts: DateTime<Utc>,
    pub utilization_pct: f32,
    pub vram_used_mib: u64,
    pub vram_total_mib: u64,
    pub temp_c: f32,
    pub power_w: f32,
    /// True when no successful poll within [`STALE_AFTER_SECS`].
    pub stale: bool,
    pub source: TelemetrySourceKind,
}

/// Per-process VRAM attribution (NVML accounting; best-effort on fallback).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessSample {
    pub pid: u32,
    pub name: String,
    pub vram_mib: u64,
    /// Backend id this process belongs to, when known.
    pub backend_id: Option<String>,
}

/// A pollable telemetry source.
#[async_trait]
pub trait TelemetrySource: Send + Sync {
    fn kind(&self) -> TelemetrySourceKind;
    async fn sample(&self) -> Result<Vec<GpuSample>, TelemetryError>;
    async fn processes(&self) -> Result<Vec<ProcessSample>, TelemetryError>;
}

/// NVML-backed source (primary).
pub struct NvmlSource;

#[async_trait]
impl TelemetrySource for NvmlSource {
    fn kind(&self) -> TelemetrySourceKind {
        TelemetrySourceKind::Nvml
    }
    async fn sample(&self) -> Result<Vec<GpuSample>, TelemetryError> {
        todo!("NVML poll via nvml-wrapper")
    }
    async fn processes(&self) -> Result<Vec<ProcessSample>, TelemetryError> {
        todo!("NVML per-process accounting")
    }
}

/// `nvidia-smi --query --xml` source (load-bearing fallback).
pub struct NvidiaSmiSource {
    pub nvidia_smi_path: std::path::PathBuf,
}

impl NvidiaSmiSource {
    /// Parse `nvidia-smi -q -x` XML output into samples.
    pub fn parse_xml(xml: &str) -> Result<Vec<GpuSample>, TelemetryError> {
        let _ = xml;
        todo!("parse nvidia-smi XML with quick-xml")
    }
}

#[async_trait]
impl TelemetrySource for NvidiaSmiSource {
    fn kind(&self) -> TelemetrySourceKind {
        TelemetrySourceKind::NvidiaSmi
    }
    async fn sample(&self) -> Result<Vec<GpuSample>, TelemetryError> {
        todo!("run nvidia-smi -q -x and parse")
    }
    async fn processes(&self) -> Result<Vec<ProcessSample>, TelemetryError> {
        todo!("parse nvidia-smi process list")
    }
}

/// In-memory snapshot store: single writer (the poll loop), many readers.
/// Marks GPUs STALE after [`STALE_AFTER_SECS`] without a successful poll.
pub struct TelemetryStore {
    last: Vec<GpuSample>,
}

impl TelemetryStore {
    pub fn new() -> Self {
        todo!("empty telemetry store")
    }

    /// Record a fresh poll; clears STALE for reported GPUs.
    pub fn update(&mut self, samples: Vec<GpuSample>) {
        todo!("store {samples:?}")
    }

    /// Current snapshot; GPUs older than [`STALE_AFTER_SECS`] are
    /// returned with `stale: true` and last-known values.
    pub fn snapshot(&self) -> Vec<GpuSample> {
        todo!("snapshot with staleness applied")
    }
}

impl Default for TelemetryStore {
    fn default() -> Self {
        Self { last: Vec::new() }
    }
}

/// Telemetry errors.
#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    #[error("NVML unavailable: {0}")]
    NvmlUnavailable(String),
    #[error("nvidia-smi failed: {0}")]
    SmiFailed(String),
    #[error("XML parse error: {0}")]
    ParseError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_constructs_with_source() {
        let s = GpuSample {
            index: 0,
            name: "NVIDIA RTX A4000".to_string(),
            ts: Utc::now(),
            utilization_pct: 12.5,
            vram_used_mib: 1024,
            vram_total_mib: 16384,
            temp_c: 45.0,
            power_w: 60.0,
            stale: false,
            source: TelemetrySourceKind::Nvml,
        };
        assert!(!s.stale);
        assert_eq!(s.source, TelemetrySourceKind::Nvml);
        assert_eq!(STALE_AFTER_SECS, 10);
    }
}
