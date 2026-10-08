# Crate map — implementation contract

Rust workspace scaffold for the Local LLM Service Manager. Eight library
crates under `crates/` (workspace members) plus the Tauri shell in
`src-tauri/` (excluded from the workspace; path-deps to the crates).
The UI shell is in `ui/` (Vite + plain TypeScript; framework-independent
Tauri command boundary per spec).

Conventions: every crate exposes its public API from `src/lib.rs`; key
functions are `todo!()` stubs awaiting implementation; each crate has
exactly one passing `#[test]`. Shared versions live in the root
`[workspace.dependencies]`. Spec references below are to
`docs/technical-specification.md`.

---

## manager-config — spec §10

Versioned, serde-typed application configuration. Single JSON file at
`%APPDATA%\local-llm-service-manager\config.json`. Secrets never stored —
only credential references.

**Types:** `AppConfig` (version, data_dir, view_modes, backends, gateway,
models, telemetry, mxc, winml, updates, notifications), `ViewMode`
(Appliance|Console|Topology), `BackendConfig` (id, enabled, model_file, port,
flags, raw_flags, restart_policy, overrides_global), `ServerFlags`
(n_ctx, n_batch, tensor_split, split_mode), `SplitMode` (None|Layer|Row),
`RestartPolicy`, `GatewayConfig` (port, groups, routing_rules, openrouter,
request_log_len), `ModelGroup` (name, members, strategy, fallbacks),
`RoutingStrategy` (SimpleShuffle|LeastBusy|LatencyBased),
`RoutingRule`, `CloudTierConfig`, `ModelConfig`, `TelemetryConfig`,
`MxcPolicyRef` (path, mode), `MxcMode` (Learning|Enforced|Disabled),
`WinMlConfig`, `UpdatePolicy`, `ComponentUpdate`, `NotificationConfig`,
`ConfigError`.

**Functions (stubs):**
`load(path: &PathBuf) -> Result<AppConfig, ConfigError>` (migrate, validate,
fall back to `.bak`); `save(config, path)` (validate, `.bak` rotation);
`validate(config)` (ports unique/in range, splits sum to 1.0, backend model
refs exist); `migrate(raw: serde_json::Value)`; `default_config()`
(implemented); `default_data_dir()` (implemented); `CURRENT_VERSION: u32 = 1`.

---

## manager-wizard — spec §4.1

Setup wizard step state machine. Ten ordered auto-run steps; audit mode
re-runs check-type steps without reinstalling.

**Types:** `StepState` (Queued|Running|Done|Failed|Skipped|Manual),
`RunMode` (Install|Audit), `WizardContext` (config, data_dir, mode),
`StepOutcome` (state, message, log_tail, duration_ms), `WizardStep` trait
(async: `id()`, `name()`, `prerequisites() -> &[String]`,
`is_check()` default false, `planned_commands(ctx)` — must equal spawned
argv, `run(ctx)`), `PlannedCommand` (argv, cwd, env), `StepReport`,
`WizardReport` (with `progress() -> (done, total)`), `Wizard` runner,
`WizardError`.

**Functions (stubs):** `Wizard::new(steps, mode)`,
`Wizard::run_all(ctx)` (ordered auto-run, prerequisite-gated),
`Wizard::run_step(ctx, id)`, `Wizard::retry(id)`, `Wizard::skip(id, note)`,
`Wizard::mark_manual(id, path, version)`, `Wizard::progress()`.

---

## manager-supervisor — spec §2.2, §6

Process supervision: one llama-server per backend, LiteLLM gateway, local
MCP servers, optional WinMLServer. Graceful drain (30 s default, then kill),
exponential capped crash backoff, per-scope stdout/stderr ring buffers.

**Types:** `HealthStatus` (Healthy|Degraded|Unhealthy), `HealthProbe` trait
(async `probe()`), `HttpHealthProbe` (url, timeout), `BackoffPolicy`
(base, max, max_retries) with `delay_for_attempt()` (implemented) and
`allows_retry()` (implemented), `LogStream` (Stdout|Stderr), `LogRecord`,
`RingBuffer<T>` (implemented: new/push/len/recent), `SupervisedProcess`,
`Supervisor`, `SupervisorError`.

**Functions (stubs):** `SupervisedProcess::spawn(id, program, args, env,
log_ring)`, `drain(timeout)`, `kill()`; `Supervisor::new(backoff)`,
`spawn_backend(id)`, `stop(id, graceful)`, `restart(id, graceful)`,
`launch_argv(id)` — must equal the wizard's `planned_commands` output.

---

## manager-telemetry — spec §5

GPU telemetry. NVML primary, `nvidia-smi --query --xml` load-bearing
fallback. STALE after [`STALE_AFTER_SECS`] = 10 s without a successful poll;
last-known values shown, never synthesized. Source shown per GPU in UI.

**Types:** `TelemetrySourceKind` (Nvml|NvidiaSmi), `GpuInfo`
(index, name, uuid, compute_capability, total_vram_mib, source),
`GpuSample` (index, name, ts, utilization_pct, vram_used_mib,
vram_total_mib, temp_c, power_w, stale, source), `ProcessSample`
(pid, name, vram_mib, backend_id), `TelemetrySource` trait
(async `sample()`, `processes()`), `NvmlSource`, `NvidiaSmiSource`
(with `parse_xml(&str)` stub), `TelemetryStore`, `TelemetryError`.

**Functions (stubs):** `TelemetrySource::sample/processes`,
`NvidiaSmiSource::parse_xml`, `TelemetryStore::new/update/snapshot`
(snapshot applies staleness).

---

## manager-gateway — spec §7

LiteLLM `config.yaml` generation from `AppConfig`. Generated file is never
hand-edited; UI previews a diff before apply.

**Types:** `LiteLlmModelEntry` (model_name, litellm_params),
`LiteLlmParams` (api_base, api_key, strategy), `OpenRouterTarget`
(enabled, model_allowlist, daily_cap_usd), `GatewayError`.
Reuses `manager_config::{AppConfig, ModelGroup, RoutingStrategy}`.

**Functions (stubs):** `generate_config_yaml(config) -> Result<String>`,
`model_entries_for_group(group, config)`,
`pick_strategy(group, latency_data_available)` (degrade latency-based →
simple-shuffle when no latency data).

Group/strategy defaults (spec §7): planner/latency-based → coder →
openrouter; coder-fast/least-busy → coder → openrouter;
coder/simple-shuffle → openrouter; tool-runner/least-busy (+ WinML :8090 if
registered) → openrouter.

---

## manager-models — spec §4.6

Model library: curated GGUF catalog, downloads with progress/pause/resume/
cancel, SHA-256 verification before "installed", fit-check vs discovered VRAM.

**Types:** `ModelSpec` (id, name, quant, size_bytes, sha256, url),
`DownloadProgress` (downloaded_bytes, total_bytes; `fraction()`
implemented), `DownloadHandle` (async `pause/resume/cancel` stubs),
`FitVerdict` (Fits{gpu} | FitsWithSplit{split} | TooLarge{needed_mib,
free_mib}), `ModelError`.

**Functions (stubs):** `download(spec, dest, on_progress)` (HTTP range
resume), `verify_sha256(path, expected_hex)`,
`fit_check(spec, free_vram_mib_per_gpu)`.

---

## manager-mxc — spec §11

MXC sandbox policy: default-deny, tool execution only (inference stays on
host). Secrets never in policy JSON — `CredentialRef` holds only the
Credential Manager target name. Begin in Learning mode, derive enforced
policy from the activity report.

**Types:** `PolicyDecision` (Allow|Prompt|Deny), `FsAccess`
(Read|Write|ReadWrite|Deny), `FsRule` (path, access), `NetRule`
(host, ports, decision), `CredentialRef` (name, target), `Policy`
(version, filesystem, network, credentials, default_deny) with
`default_policy()` (implemented), `AccessKind`
(FileRead|FileWrite|Network), `AccessRequest`, `ValidationReport`,
`LearningEntry`, `LearningReport`, `MxcError`.

**Functions (stubs):** `decide(policy, request)` (default-deny on no match),
`validate(policy)` (no secrets, absolute paths, ports in range,
default-deny set, no allow-all), `parse_learning_report(json)`.

---

## manager-winml — spec §6.5

Windows ML secondary backend: optional WinMLServer on :8090, registered in
LiteLLM as a plain OpenAI-compatible backend. Single-GPU tool-runner-class
models only; never planner/coder-30b. Researched limits bannered in UI.

**Types:** `GpuSelection` (SystemDefault|TargetGpuCoarse),
`WinMlLimits` (supports_tensor_split, max_registered_models,
gpu_selection, experimental) with `known_limits()` (implemented:
no tensor split, 1 model, coarse GPU selection, experimental),
`WinMlRegistration` (model_id, port, endpoint, registered_at,
responding_adapter, limits), `WinMlError`.

**Functions (stubs):** `probe()` (record limits verbatim), `register(config,
model_id)` (refuse ineligible model classes), `unregister()`,
`health(port)`.

---

## src-tauri — spec §2, §14

Tauri 2 shell (excluded from the Cargo workspace). Registers placeholder
commands `get_status`, `list_backends`, `get_gpu_telemetry`,
`run_wizard_step` (stub JSON; full §14 command inventory is the
implementation target). `tauri.conf.json`: productName "Local LLM Service
Manager", version 0.1.0, identifier `com.pdrakeweb.llm-manager`, publisher
pdrakeweb, category DeveloperTool, NSIS per-user install, English, LICENSE,
Start Menu shortcut, no desktop shortcut, `installer-hooks.nsi`
(uninstall asks before removing `%APPDATA%\local-llm-service-manager`;
finish offers Launch). See `src-tauri/INSTALLER.md` for wizard pages and
silent flags (`/S`, `/D=`). Does not compile on the Linux VM (no
GTK/WebKit); kept syntactically valid.

## ui/

Vite + plain TypeScript scaffold (no framework lock-in). `npm run build`
emits `ui/dist`. `src/main.ts` renders the scaffold page and calls the
`get_status` Tauri command when running under Tauri.
