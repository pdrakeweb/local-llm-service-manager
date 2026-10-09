/* Dashboard view — fully functional.
   - Appliance: service cards, GPU strip, model placement, throughput chart.
   - Console: dense tables with sparklines/bars.
   - Topology: mini cluster node map (SVG).
   Polls getDashboard() every 5s; Pause live stops polling; data older than
   10s without a successful poll is labeled STALE (spec §4 global UI rules).
   Backend failures keep the last good model and show an error banner —
   never fabricated data presented as live. */

import { getDashboard, isTauri } from "../api";
import { esc } from "../html";
import { gaugeRing, meter, multiLine, sparkline, stackedBar } from "../charts";
import { icon } from "../icons";
import { GPU_COLORS } from "../mock";
import type { BackendInfo, DashboardModel, GpuInfo } from "../types";

type ViewMode = "appliance" | "console" | "topology";

const VIEW_KEY = "llm.view.dashboard";
let mode: ViewMode = (localStorage.getItem(VIEW_KEY) as ViewMode) || "appliance";
if (mode !== "appliance" && mode !== "console" && mode !== "topology") mode = "appliance";

let timer: number | null = null;
let paused = false;
let lastOk = 0;
let lastModel: DashboardModel | null = null;
let lastLive = false;
let lastError: string | null = null;
let tickTimer: number | null = null;

const C = { blue: "#4a9eff", purple: "#a371f7", teal: "#39c5cf", green: "#3fb950", amber: "#d29922", red: "#f85149" };

function setMode(m: ViewMode): void {
  mode = m;
  try { localStorage.setItem(VIEW_KEY, m); } catch { /* storage unavailable */ }
}

function statusDot(s: string): string {
  const cls = s === "ok" ? "ok" : s === "degraded" || s === "starting" ? "warn" : s === "down" ? "err" : "idle";
  return `<span class="dot ${cls}"></span>`;
}

/** All services including the gateway when the backend reported one. */
function allServices(m: DashboardModel): BackendInfo[] {
  return m.gateway ? [m.gateway, ...m.backends] : [...m.backends];
}

function serviceCard(b: BackendInfo, isGateway: boolean): string {
  const ringColor = b.status === "ok" ? C.green : b.status === "starting" ? C.amber : C.red;
  const ringVal = isGateway ? String(b.requests_1h) : b.health.toFixed(0);
  const ringSub = isGateway ? "req/1h" : "health";
  const m1k = isGateway ? "REQUESTS / 1H" : "THROUGHPUT";
  const m1v = isGateway ? b.requests_1h.toLocaleString() : b.throughput_toks.toFixed(1);
  const m1u = isGateway ? "" : " tok/s";
  const m2k = isGateway ? "ERRORS / 1H" : "VRAM HELD";
  const m2v = isGateway ? String(b.errors_1h) : b.vram_held_gb.toFixed(1);
  const m2u = isGateway ? "" : " GB";
  return `<div class="card">
    <h3>${statusDot(b.status)} ${esc(b.name)} <span class="port">:${b.port}</span></h3>
    <div class="svc-top">
      ${gaugeRing(isGateway ? 100 : b.health, 64, ringColor, esc(ringVal), ringSub)}
      <div class="kv">
        <span class="k">${m1k}</span><span class="v">${esc(m1v)}<small>${m1u}</small></span>
        <span class="k" style="margin-top:4px">${m2k}</span><span class="v">${esc(m2v)}<small>${m2u}</small></span>
      </div>
    </div>
    <div class="spark-wrap"><div class="cap">${isGateway ? "Requests per minute" : "Tokens/s · 1h"}</div>
      ${sparkline(b.spark, 220, 34, isGateway ? C.blue : b.id === "coder-30b" ? C.purple : b.id === "coder-7b" ? C.teal : C.blue)}
    </div>
    <div class="card-actions">
      <button class="btn" data-act="restart" data-id="${esc(b.id)}">${icon("restart")} Restart</button>
      <button class="btn" data-act="logs" data-id="${esc(b.id)}">${icon("logs")} Logs</button>
    </div>
  </div>`;
}

function gpuStripCard(g: GpuInfo): string {
  const vramPct = Math.round((g.vram_used_gb / g.vram_total_gb) * 100);
  return `<div class="card">
    <div class="gpu-head">${statusDot(g.status)} ${icon("gpu")} ${esc(g.name)}</div>
    <div class="gpu-metrics">
      <div class="mrow"><span class="mlabel">Utilization</span><span class="mval">${g.util_pct}%</span></div>
      <div class="mrow"><span></span>${sparkline(g.util_spark, 220, 26, C.blue)}</div>
      <div class="mrow"><span class="mlabel">VRAM ${g.vram_used_gb.toFixed(1)}/${g.vram_total_gb} GB</span><span class="mval">${vramPct}%</span></div>
      <div class="mrow"><span></span>${meter(vramPct, 220, 8, C.purple)}</div>
      <div class="mrow"><span class="mlabel">Temperature</span><span class="mval">${g.temp_c} °C · peak ${g.temp_peak_c}</span></div>
      <div class="mrow"><span></span>${sparkline(g.temp_spark, 220, 26, C.amber)}</div>
      <div class="mrow"><span class="mlabel">Power</span><span class="mval">${g.power_w} / ${g.power_limit_w} W</span></div>
      <div class="mrow"><span></span>${sparkline(g.power_spark, 220, 26, C.green)}</div>
    </div>
  </div>`;
}

function applianceView(m: DashboardModel): string {
  const svcs = allServices(m);
  const cards = svcs.map((b, i) => serviceCard(b, i === 0 && m.gateway !== null)).join("");
  const gpus = m.gpus.map(gpuStripCard).join("");
  const placeHtml = m.placement.map((p) => {
    const total = p.segments.reduce((a, s) => a + s.pct, 0) || 1;
    return `<div class="place-row"><span class="mname">${esc(p.model)}</span>
      <div>${stackedBar(p.segments.map((s) => ({ pct: (s.pct / total) * 100, color: GPU_COLORS[s.gpu] ?? "#6d7686", label: `${s.gpu} ${s.pct}%` })))}</div></div>`;
  }).join("");
  const legend = Object.entries({ a4000: "A4000", "p100-0": "P100-0", "p100-1": "P100-1" })
    .map(([k, v]) => `<span><span class="sw" style="background:${GPU_COLORS[k]}"></span>${esc(v)}</span>`).join("");
  const legendHtml = m.placement.length ? `<div class="legend">${legend}</div>` : "";
  const chart = m.throughputSeries.length
    ? `<div class="card"><div class="sec-title">${icon("bolt")} Throughput — all backends · 1h</div>
       <div class="legend" style="margin:0 0 8px">${m.throughputSeries.map((s) => `<span><span class="sw" style="background:${esc(s.color)}"></span>${esc(s.name)}</span>`).join("")}</div>
       ${multiLine(m.throughputSeries, 560, 220)}</div>`
    : "";
  const events = m.events.length
    ? `<div class="card"><div class="sec-title">${icon("clock")} Recent activity</div>
       <div class="events">${m.events.map((e) => `<div class="ev"><span class="dot ${e.level === "ok" ? "ok" : e.level === "warn" ? "warn" : e.level === "error" ? "err" : "idle"}"></span><time>${esc(e.time)}</time><span>${esc(e.text)}</span></div>`).join("")}</div></div>`
    : "";
  const empty = !svcs.length && !m.gpus.length
    ? `<div class="card"><div class="empty">No backends or GPUs reported. Run the setup wizard to install and configure the stack.</div></div>` : "";
  return `${empty}
    <div class="cards5">${cards}</div>
    <div class="cards3" style="margin-top:12px">${gpus}</div>
    <div class="grid2">
      <div class="card"><div class="sec-title">${icon("layers")} Model placement</div>${placeHtml}${legendHtml}</div>
      ${chart}
    </div>
    ${events ? `<div style="margin-top:12px">${events}</div>` : ""}`;
}

function consoleView(m: DashboardModel): string {
  const svcRows = allServices(m).map((b) => `<tr>
    <td>${statusDot(b.status)} ${esc(b.name)}</td><td>:${b.port}</td><td>${b.model ? esc(b.model) : "—"}</td>
    <td>${b.health}</td><td>${b.throughput_toks ? b.throughput_toks.toFixed(1) + " tok/s" : b.requests_1h + " req/h"}</td>
    <td>${sparkline(b.spark, 120, 20, C.blue)}</td></tr>`).join("");
  const gpuRows = m.gpus.map((g) => {
    const vramPct = Math.round((g.vram_used_gb / g.vram_total_gb) * 100);
    return `<tr><td>${statusDot(g.status)} ${esc(g.name)}</td><td>${g.util_pct}%</td>
      <td>${meter(g.util_pct, 120, 8, C.blue)}</td>
      <td>${g.vram_used_gb.toFixed(1)}/${g.vram_total_gb} GB</td>
      <td>${meter(vramPct, 120, 8, C.purple)}</td>
      <td>${g.temp_c}°C</td><td>${g.power_w}W</td></tr>`;
  }).join("");
  return `<div class="card" style="margin-bottom:12px"><div class="sec-title">Services</div>
    <table class="tbl"><thead><tr><th>Service</th><th>Port</th><th>Model</th><th>Health</th><th>Rate</th><th>Trend</th></tr></thead>
    <tbody>${svcRows || `<tr><td colspan="6" class="empty">no services reported</td></tr>`}</tbody></table></div>
    <div class="card"><div class="sec-title">GPUs</div>
    <table class="tbl"><thead><tr><th>GPU</th><th>Util</th><th></th><th>VRAM</th><th></th><th>Temp</th><th>Power</th></tr></thead>
    <tbody>${gpuRows || `<tr><td colspan="7" class="empty">no GPUs reported</td></tr>`}</tbody></table></div>`;
}

function topologyView(m: DashboardModel): string {
  // Mini node map: clients -> LiteLLM -> backends -> GPUs -> models (SVG).
  // Honest empty state when the backend reports nothing — never fake nodes.
  if (!m.backends.length && !m.gpus.length && !m.gateway) {
    return `<div class="card"><div class="empty">No topology to draw — no backends or GPUs reported. Run the setup wizard to install and configure the stack.</div></div>`;
  }
  const W = 980, H = 380;
  const colX = [40, 240, 430, 640, 830];
  const nodeW = 130, nodeH = 56;
  const clients = ["Continue.dev", "Cline"];
  const backs = m.backends;
  const gpus = m.gpus;
  const models = m.placement;

  const cy = (i: number, n: number) => 60 + (i * (H - 120)) / Math.max(1, n - 1 || 1);
  let svg = `<svg width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" style="display:block;width:100%" role="img" aria-label="Cluster topology mini map">`;
  const edge = (x1: number, y1: number, x2: number, y2: number, label: string, cls = "") => {
    const mx = (x1 + x2) / 2;
    svg += `<path class="map-edge ${cls}" d="M${x1},${y1} C${mx},${y1} ${mx},${y2} ${x2},${y2}"/>`;
    if (label) svg += `<text class="map-edge-label" x="${mx}" y="${(y1 + y2) / 2 - 4}" text-anchor="middle">${esc(label)}</text>`;
  };
  const node = (x: number, y: number, title: string, sub: string, dot: string) => {
    svg += `<g><rect class="map-node" x="${x}" y="${y - nodeH / 2}" width="${nodeW}" height="${nodeH}" rx="8"/>
      <circle cx="${x + 14}" cy="${y - 8}" r="4" fill="${dot}"/>
      <text class="map-node-text" x="${x + 26}" y="${y - 2}">${esc(title)}</text>
      <text class="map-node-sub" x="${x + 14}" y="${y + 16}">${esc(sub)}</text></g>`;
  };

  clients.forEach((c, i) => {
    const y = cy(i, clients.length);
    node(colX[0], y, c, "client", C.green);
    edge(colX[0] + nodeW, y, colX[1], H / 2, "");
  });
  node(colX[1], H / 2, "LiteLLM", m.gateway ? `:${m.gateway.port} · least-busy` : ":4000", C.green);
  backs.forEach((b, i) => {
    const y = cy(i, backs.length);
    node(colX[2], y, b.name, `:${b.port} · ${b.throughput_toks.toFixed(1)} tok/s`, b.status === "ok" ? C.green : C.amber);
    edge(colX[1] + nodeW, H / 2, colX[2], y, "", b.status === "starting" ? "warn" : "hot");
  });
  gpus.forEach((g, i) => {
    const y = cy(i, gpus.length);
    node(colX[3], y, g.name, `${g.util_pct}% util`, C.green);
    backs.forEach((b, j) => edge(colX[2] + nodeW, cy(j, backs.length), colX[3], y, "", "hot"));
  });
  models.slice(0, 4).forEach((p, i) => {
    const y = cy(i, Math.min(4, models.length));
    node(colX[4], y, p.model, "loaded", C.green);
    if (gpus[i]) edge(colX[3] + nodeW, cy(i, gpus.length), colX[4], y, "");
  });
  svg += `<text class="map-col-label" x="${colX[0]}" y="24">CLIENTS</text>
    <text class="map-col-label" x="${colX[1]}" y="24">GATEWAY</text>
    <text class="map-col-label" x="${colX[2]}" y="24">BACKENDS</text>
    <text class="map-col-label" x="${colX[3]}" y="24">GPUS</text>
    <text class="map-col-label" x="${colX[4]}" y="24">MODELS</text></svg>`;
  return `<div class="card"><div class="sec-title">${icon("map")} Topology — same data, node view</div>
    <div class="map-canvas" style="border:0;padding:0">${svg}</div>
    <div class="legend"><span><span class="sw" style="background:${C.green}"></span>healthy</span>
    <span><span class="sw" style="background:${C.amber}"></span>degraded / starting</span>
    <span>edge width ∝ tok/s</span>
    <a href="#/map" style="color:var(--accent)">Open full service map →</a></div></div>`;
}

function headerHtml(live: boolean, stale: boolean, updatedAgo: string, allOk: boolean): string {
  return `<div class="topbar">
    <h1>Dashboard</h1>
    <span class="status-pill">${statusDot(allOk ? "ok" : "degraded")} ${allOk ? "All systems normal" : "Attention needed"}</span>
    <span class="viewsel-label">View:</span>
    <span class="viewsel" role="tablist">
      <button data-view="appliance" class="${mode === "appliance" ? "active" : ""}">Appliance</button>
      <button data-view="console" class="${mode === "console" ? "active" : ""}">Console</button>
      <button data-view="topology" class="${mode === "topology" ? "active" : ""}">Topology</button>
    </span>
    <span class="spacer"></span>
    ${stale ? `<span class="stale-tag">STALE</span>` : ""}
    <span class="updated" id="updatedAgo">Last updated ${esc(updatedAgo)}</span>
    <button class="btn" id="pauseBtn">${icon(paused ? "play" : "pause")} ${paused ? "Resume live" : "Pause live"}</button>
    <a class="btn" href="#/wizard">Setup wizard</a>
  </div>
  ${lastError ? `<div style="padding:12px 20px 0"><div class="banner">${icon("warn")} backend error — showing last known data: ${esc(lastError)}</div></div>` : ""}
  ${!live ? `<div style="padding:12px 20px 0"><div class="banner">${icon("warn")} disconnected — showing sample data (not running under Tauri)</div></div>` : ""}`;
}

function ago(ts: number): string {
  const s = Math.max(0, Math.round((Date.now() - ts) / 1000));
  return s < 2 ? "just now" : `${s}s ago`;
}

function render(): void {
  const el = document.getElementById("view");
  if (!el) return;
  if (!lastModel) {
    // No data yet — either still loading or the first poll failed.
    el.innerHTML = lastError
      ? headerHtml(lastLive, true, "never", false) + `<div class="view"><div class="card"><div class="empty">Backend error: ${esc(lastError)} — retrying…</div></div></div>`
      : "";
    return;
  }
  const m = lastModel;
  const severe = allServices(m).some((b) => b.status === "degraded" || b.status === "down");
  const allOk = !severe && !m.stale && !lastError;
  const body = mode === "appliance" ? applianceView(m) : mode === "console" ? consoleView(m) : topologyView(m);
  el.innerHTML = headerHtml(lastLive, m.stale, ago(m.updatedAt), allOk) + `<div class="view">${body}</div>`;
  el.querySelectorAll("[data-view]").forEach((b) =>
    b.addEventListener("click", () => { setMode((b as HTMLElement).dataset.view as ViewMode); render(); }));
  const pb = el.querySelector("#pauseBtn");
  pb?.addEventListener("click", () => { paused = !paused; render(); });
  el.querySelectorAll("[data-act]").forEach((b) =>
    b.addEventListener("click", () => {
      const act = (b as HTMLElement).dataset.act;
      const id = (b as HTMLElement).dataset.id;
      if (act === "logs") location.hash = "#/logs";
      else if (act === "restart") alert(`Restart for ${id ?? "?"} is not wired to a backend command yet.`);
    }));
}

async function poll(): Promise<void> {
  if (paused) return;
  const r = await getDashboard(); // never rejects; failures arrive as r.error
  if (r.error) {
    lastError = r.error;
    // Keep the last good model; mark stale when it ages out.
    if (lastModel && Date.now() - lastOk > 10_000) lastModel = { ...lastModel, stale: true };
  } else {
    lastModel = r.data;
    lastLive = r.live;
    lastError = null;
    lastOk = Date.now();
  }
  render();
}

export async function mountDashboard(): Promise<() => void> {
  await poll();
  timer = window.setInterval(poll, 5000);
  // "Xs ago" ticker: update the timestamp node in place instead of a full re-render.
  tickTimer = window.setInterval(() => {
    const node = document.getElementById("updatedAgo");
    if (node && lastModel) node.textContent = `Last updated ${ago(lastModel.updatedAt)}`;
  }, 1000);
  return () => {
    if (timer) clearInterval(timer);
    if (tickTimer) clearInterval(tickTimer);
    timer = tickTimer = null;
  };
}
