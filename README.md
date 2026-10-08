# Local LLM Service Manager

Windows desktop application (Tauri 2 + Rust) that installs, configures,
supervises, and monitors a local LLM serving stack on a single workstation.

Target machine: Dell Precision 7865 — Threadripper PRO 5945WX, RTX A4000 16 GB
+ 2× Tesla P100 16 GB, Windows 11 Pro.

## What it manages

- `llama-server` (CUDA build) subprocesses, one per backend, with tensor-split
  placement across the GPUs
- LiteLLM gateway (`:4000`) unifying the backends, with OpenRouter cloud tier
  for failover and overflow
- GPU telemetry (NVML, with `nvidia-smi` XML fallback)
- Model library: downloads with SHA-256 verification, VRAM fit checks
- VS Code wiring for Continue.dev and Cline
- MCP servers (GitHub, Google Drive)
- Windows ML `WinMLServer` as a secondary backend for small single-GPU models
- MXC (Microsoft Execution Containers) sandbox policies for agent tool execution
- Setup wizard with auto-running checklist, progress tracking, and on-demand
  re-audit

## Documentation

- [`docs/technical-specification.md`](docs/technical-specification.md) — full
  technical specification (architecture, functional requirements, API surface,
  test plan, implementation phases)
- [`docs/design-b-hybrid.md`](docs/design-b-hybrid.md) — approved UI design
  record (Design B base with Console and Topology alternate views)
- [`docs/mockups/`](docs/mockups/) — rendered UI mockup galleries for all
  designs, including the final 15-window hybrid set

## Repository layout

- `crates/` — Rust workspace: core logic crates (config, wizard state machine,
  supervisor, telemetry, gateway, models, MXC, Windows ML)
- `src-tauri/` — Tauri 2 application shell and command/event API
- `ui/` — frontend
- `.github/workflows/` — CI (tests) and release autobuild (Windows installer)

## Status

v0.1.0 — initial implementation. See the technical specification for scope
and open questions.
