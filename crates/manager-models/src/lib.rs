//! Model library: downloads, checksums, fit-check (spec §4.6).
//!
//! Curated GGUF catalog with SHA-256 verification before a model is marked
//! "installed". Downloads support progress callbacks, pause/resume (HTTP
//! range), and cancel. Fit-check compares model size against discovered
//! VRAM and recommends a placement (fits / fits with tensor split / too
//! large).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One downloadable model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSpec {
    pub id: String,
    pub name: String,
    /// e.g. "Q4_K_M".
    pub quant: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub url: String,
}

/// Download progress snapshot for the progress callback.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

impl DownloadProgress {
    pub fn fraction(&self) -> f64 {
        if self.total_bytes == 0 {
            0.0
        } else {
            self.downloaded_bytes as f64 / self.total_bytes as f64
        }
    }
}

/// Handle to an in-flight download.
pub struct DownloadHandle {
    pub spec_id: String,
}

impl DownloadHandle {
    /// Pause the download (keeps partial file for resume).
    pub async fn pause(&self) -> Result<(), ModelError> {
        todo!("pause download {}", self.spec_id)
    }

    /// Resume a paused download via HTTP range requests.
    pub async fn resume(&self) -> Result<(), ModelError> {
        todo!("resume download {}", self.spec_id)
    }

    /// Cancel and delete the partial file.
    pub async fn cancel(&self) -> Result<(), ModelError> {
        todo!("cancel download {}", self.spec_id)
    }
}

/// Start downloading `spec` to `dest`, invoking `on_progress` as chunks land.
pub async fn download<F>(
    spec: &ModelSpec,
    dest: &PathBuf,
    on_progress: F,
) -> Result<DownloadHandle, ModelError>
where
    F: Fn(DownloadProgress) + Send + 'static,
{
    let _ = (spec, dest, on_progress);
    todo!("download {}", spec.id)
}

/// SHA-256-verify a downloaded file against the catalog hash.
pub async fn verify_sha256(path: &PathBuf, expected_hex: &str) -> Result<bool, ModelError> {
    let _ = (path, expected_hex);
    todo!("sha256 verify {path:?}")
}

/// Fit verdict against discovered per-GPU free VRAM (MiB).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FitVerdict {
    /// Fits on a single GPU (index).
    Fits { gpu: usize },
    /// Needs a tensor split; recommended per-GPU fractions.
    FitsWithSplit { split: Vec<f32> },
    /// Does not fit even split across all GPUs.
    TooLarge { needed_mib: u64, free_mib: u64 },
}

/// Recommend a placement for `spec` given per-GPU free VRAM.
pub fn fit_check(spec: &ModelSpec, free_vram_mib_per_gpu: &[u64]) -> FitVerdict {
    let _ = (spec, free_vram_mib_per_gpu);
    todo!("fit check for {}", spec.id)
}

/// Model library errors.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("network error: {0}")]
    Network(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("checksum mismatch for {0}")]
    ChecksumMismatch(String),
    #[error("download cancelled: {0}")]
    Cancelled(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_spec_and_progress_construct() {
        let spec = ModelSpec {
            id: "qwen3-8b".to_string(),
            name: "Qwen3-8B".to_string(),
            quant: "Q4_K_M".to_string(),
            size_bytes: 4_700_000_000,
            sha256: "abc123".to_string(),
            url: "https://example.com/qwen3-8b-q4_k_m.gguf".to_string(),
        };
        assert_eq!(spec.quant, "Q4_K_M");
        let p = DownloadProgress {
            downloaded_bytes: 2_350_000_000,
            total_bytes: 4_700_000_000,
        };
        assert!((p.fraction() - 0.5).abs() < f64::EPSILON);
        let verdict = FitVerdict::Fits { gpu: 0 };
        assert!(matches!(verdict, FitVerdict::Fits { gpu: 0 }));
    }
}
