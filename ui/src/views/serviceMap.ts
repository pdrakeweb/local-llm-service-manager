/* Service Map — full cluster topology (spec §4.3).
   Static layout faithful to mockup 05-service-map.png: clients -> LiteLLM ->
   backends -> GPUs -> loaded models, MCP servers, dashed MXC boundary,
   throughput edge labels, cluster summary inspector, recent events.
   Live data wiring pending: node positions/labels come from sample data;
   node click would deep-link to canonical windows once live. */

import { mockDashboard } from "../mock";
import { gaugeRing } from "../charts";
import { icon } from "../icons";
import { esc } from "../html";

const C = { green: "#3fb950", amber: "#d29922", red: "#f85149", blue: "#4a9eff", grey: "#6d7686" };

export function renderServiceMap(): string {
  const m = mockDashboard();
  const W = 1200, H = 640;

  interface N { x: number; y: number; title: string; sub: string; dot: string; ring?: { pct: number; color: string; label: string } }
  const clients: N[] = [
    { x: 90, y: 120, title: "VS Code", sub: "2 sessions", dot: C.green },
    { x: 90, y: 250, title: "Continue.dev", sub: "1 session", dot: C.green },
    { x: 90, y: 380, title: "Cline", sub: "1 session", dot: C.green },
    { x: 90, y: 510, title: "CLI test", sub: "idle", dot: C.grey },
  ];
  const gateway: N = { x: 300, y: 315, title: "LiteLLM", sub: ":4000 · 1,284 req/1h", dot: C.green };
  const backends: N[] = [
    { x: 510, y: 90, title: "planner-27b", sub: ":8081 · 8.4 tok/s", dot: C.green, ring: { pct: 92, color: C.green, label: "92" } },
    { x: 510, y: 235, title: "coder-30b", sub: ":8082 · 22.6 tok/s", dot: C.green, ring: { pct: 96, color: C.green, label: "96" } },
    { x: 510, y: 380, title: "coder-7b", sub: ":8083 · 41.2 tok/s", dot: C.green, ring: { pct: 88, color: C.green, label: "88" } },
    { x: 510, y: 525, title: "tool-runner", sub: ":8084 · starting", dot: C.amber, ring: { pct: 60, color: C.amber, label: "60" } },
  ];
  const gpus: N[] = [
    { x: 730, y: 140, title: "A4000", sub: "16 GB · 71 °C", dot: C.green, ring: { pct: 62, color: C.amber, label: "62%" } },
    { x: 730, y: 330, title: "P100-0", sub: "16 GB · 68 °C", dot: C.green, ring: { pct: 45, color: C.green, label: "45%" } },
    { x: 730, y: 520, title: "P100-1", sub: "16 GB · 54 °C", dot: C.green, ring: { pct: 12, color: C.green, label: "12%" } },
  ];
  const models: N[] = [
    { x: 950, y: 100, title: "Qwen3.8-27B", sub: "Q4_K_M · 16.2 GB", dot: C.green },
    { x: 950, y: 240, title: "Qwen3-Coder-30B", sub: "Q4_K_M · 18.1 GB", dot: C.green },
    { x: 950, y: 380, title: "Qwen2.5-Coder-7B", sub: "Q5_K_M · 5.2 GB", dot: C.green },
    { x: 950, y: 520, title: "Qwen3-8B", sub: "Q8_0 · 8.6 GB · loading", dot: C.amber },
  ];
  const mcp: N[] = [
    { x: 300, y: 590, title: "mcp-github", sub: "connected", dot: C.green },
    { x: 510, y: 590, title: "mcp-gdrive", sub: "reconnecting", dot: C.amber },
  ];

  const NW = 150, NH = 62;
  let s = `<svg width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" style="display:block;width:100%" role="img" aria-label="Full cluster service map">`;
  // MXC dashed boundary around backends
  s += `<rect x="470" y="30" width="230" height="540" rx="10" fill="none" stroke="#4a9eff" stroke-width="1" stroke-dasharray="7 5" opacity="0.55"/>
        <text x="585" y="22" text-anchor="middle" fill="#4a9eff" font-size="10" letter-spacing="1">MXC · LEARNING-MODE</text>`;
  const edge = (x1: number, y1: number, x2: number, y2: number, label = "", cls = "") => {
    const mx = (x1 + x2) / 2;
    s += `<path class="map-edge ${cls}" d="M${x1},${y1} C${mx},${y1} ${mx},${y2} ${x2},${y2}"/>`;
    if (label) s += `<text class="map-edge-label" x="${mx}" y="${(y1 + y2) / 2 - 5}" text-anchor="middle">${label}</text>`;
  };
  const node = (n: N, accent = false) => {
    s += `<g>
      <rect class="map-node" x="${n.x}" y="${n.y - NH / 2}" width="${NW}" height="${NH}" rx="8"${accent ? ` stroke="${C.blue}" stroke-width="1.5"` : ""}/>
      <circle cx="${n.x + 16}" cy="${n.y - 10}" r="4" fill="${n.dot}"/>
      <text class="map-node-text" x="${n.x + 28}" y="${n.y - 4}">${n.title}</text>
      <text class="map-node-sub" x="${n.x + 16}" y="${n.y + 16}">${n.sub}</text>
      ${n.ring ? `<g transform="translate(${n.x + NW - 26},${n.y - 12})"><circle r="13" fill="none" stroke="#2b313b" stroke-width="4"/><circle r="13" fill="none" stroke="${n.ring.color}" stroke-width="4" stroke-linecap="round" stroke-dasharray="${(2 * Math.PI * 13).toFixed(1)}" stroke-dashoffset="${(2 * Math.PI * 13 * (1 - n.ring.pct / 100)).toFixed(1)}" transform="rotate(-90)"/><text y="4" text-anchor="middle" fill="#e8ebf0" font-size="9" font-weight="700">${n.ring.label}</text></g>` : ""}
    </g>`;
  };
  clients.forEach((c, i) => {
    node(c);
    edge(c.x + NW, c.y, gateway.x, gateway.y, ["0.14/s", "0.09/s", "0.08/s", "idle"][i], i === 3 ? "" : "hot");
  });
  node(gateway, true);
  backends.forEach((b, i) => {
    node(b);
    edge(gateway.x + NW, gateway.y, b.x, b.y, ["28%", "24%", "26%", "22%"][i], i === 3 ? "warn" : "hot");
  });
  // backend -> gpu edges (tensor-split shares)
  const shares: [number, number, string][] = [
    [0, 0, "60%"], [0, 1, "40%"],
    [1, 0, "45%"], [1, 1, "35%"], [1, 2, "20%"],
    [2, 1, "100%"],
    [3, 2, "100%"],
  ];
  shares.forEach(([bi, gi, label]) => edge(backends[bi].x + NW, backends[bi].y, gpus[gi].x, gpus[gi].y, label, "hot"));
  gpus.forEach((g) => node(g));
  models.forEach((p, i) => {
    node(p);
    const gi = i === 0 ? 0 : i === 1 ? 1 : i === 2 ? 1 : 2;
    edge(gpus[gi].x + NW, gpus[gi].y, p.x, p.y);
  });
  mcp.forEach((n, i) => {
    node(n);
    if (i === 1) edge(gateway.x + 40, gateway.y + NH / 2, n.x + 40, n.y - NH / 2, "", "warn");
  });
  s += `<text class="map-col-label" x="90" y="52">CLIENTS</text>
    <text class="map-col-label" x="300" y="240">GATEWAY</text>
    <text class="map-col-label" x="510" y="52">BACKENDS · POLICY-CHECKED</text>
    <text class="map-col-label" x="730" y="72">GPUS</text>
    <text class="map-col-label" x="950" y="52">LOADED MODELS</text>
    <text class="map-col-label" x="300" y="566">MCP SERVERS</text></svg>`;

  const summary: [string, string][] = [
    ["Backends", "4 / 4 up"], ["GPUs", "3 / 3 healthy"], ["Throughput", "72.2 tok/s"],
    ["VRAM held", "35.7 / 48 GB"], ["MXC policy", "learning-mode"], ["Download", "qwen2.5-coder-7b · 62%"],
  ];
  return `<div class="topbar"><h1>Service Map</h1>
      <span class="status-pill"><span class="dot ok"></span> All systems normal</span>
      <span class="spacer"></span><span class="updated">Last updated 3s ago · sample data</span>
      <button class="btn" disabled>Pause live</button></div>
    <div class="view"><div class="banner">${icon("warn")} live data wiring pending — topology rendered from sample data</div>
    <div class="map-wrap">
      <div class="map-canvas">${s}
        <div class="legend" style="margin-top:8px">
          <span><span class="sw" style="background:${C.green}"></span>healthy</span>
          <span><span class="sw" style="background:${C.amber}"></span>degraded</span>
          <span><span class="sw" style="background:${C.red}"></span>down</span>
          <span style="border:1px dashed #4a9eff;border-radius:3px;padding:0 5px">MXC boundary</span>
          <span>edge width = tok/s</span>
        </div>
      </div>
      <div>
        <div class="card" style="margin-bottom:12px"><div class="sec-title">Cluster summary</div>
          <div class="kv">${summary.map(([k, v]) => `<div style="display:flex;justify-content:space-between;padding:5px 0;border-bottom:1px solid #232a34"><span class="k">${k}</span><span class="v" style="font-size:12px">${v}</span></div>`).join("")}</div>
        </div>
        <div class="card"><div class="sec-title">Recent events</div>
          <div class="events">${m.events.map((e) => `<div class="ev"><span class="dot ${e.level === "ok" ? "ok" : e.level === "warn" ? "warn" : "idle"}"></span><time>${esc(e.time)}</time><span>${esc(e.text)}</span></div>`).join("")}</div>
        </div>
        <div class="card" style="margin-top:12px"><div class="sec-title">GPU load</div>
          ${m.gpus.map((g) => `<div style="display:flex;align-items:center;gap:10px;margin:8px 0"><span style="font-size:12px;width:70px">${esc(g.name.replace("Tesla ", ""))}</span>${gaugeRing(g.util_pct, 44, g.util_pct > 60 ? C.amber : C.green, String(g.util_pct), "%")}</div>`).join("")}
        </div>
      </div>
    </div></div>`;
}
