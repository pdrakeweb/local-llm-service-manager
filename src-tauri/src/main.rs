//! Tauri 2 shell for the Local LLM Service Manager.
//!
//! The webview never spawns processes, touches the filesystem, or holds
//! secrets. All privileged operations are Tauri commands executed by the
//! Rust backend, delegating to the `manager-*` crates (spec §2.2, §14).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde_json::{json, Value};

/// Overall service status snapshot (stub).
#[tauri::command]
fn get_status() -> Value {
    json!({
        "status": "ok",
        "scaffold": true,
        "version": env!("CARGO_PKG_VERSION"),
    })
}

/// List configured backends (stub).
#[tauri::command]
fn list_backends() -> Value {
    json!({ "backends": [] })
}

/// Current GPU telemetry snapshot (stub).
#[tauri::command]
fn get_gpu_telemetry() -> Value {
    json!({ "gpus": [], "stale": false })
}

/// Run one wizard step; returns a task id and streams `task-progress` events (stub).
#[tauri::command]
fn run_wizard_step(step_id: String) -> Value {
    json!({ "task_id": "task-stub-001", "step_id": step_id, "accepted": true })
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_status,
            list_backends,
            get_gpu_telemetry,
            run_wizard_step
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Local LLM Service Manager");
}
