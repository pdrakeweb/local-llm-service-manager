// Local LLM Service Manager — UI scaffold (plain TypeScript, no framework).
// The Tauri command boundary stays framework-independent: a framework can
// replace this file later without changing the command contracts.

export {}; // make this a module so `declare global` is legal

declare global {
  interface Window {
    __TAURI__?: {
      core: { invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> };
    };
  }
}

interface StatusResponse {
  status: string;
  scaffold: boolean;
  version: string;
}

function render(status: StatusResponse | null, error: string | null): void {
  const app = document.getElementById("app");
  if (!app) return;
  app.innerHTML = `
    <main style="font-family: system-ui, sans-serif; background: #12151a; color: #e8ebf0; min-height: 100vh; margin: 0; padding: 48px;">
      <h1 style="font-size: 22px; margin: 0 0 8px;">Local LLM Service Manager</h1>
      <p style="color: #6d7686; margin: 0 0 24px;">UI scaffold — framework to be chosen later.</p>
      <section style="background: #1b1f26; border: 1px solid #2b313b; border-radius: 8px; padding: 16px 20px; max-width: 560px;">
        <h2 style="font-size: 14px; margin: 0 0 8px;">Backend status</h2>
        ${
          error
            ? `<p style="color: #e06c75;">${error}</p>`
            : status
              ? `<p style="font-family: monospace;">status=${status.status} scaffold=${status.scaffold} version=${status.version}</p>`
              : `<p style="color: #6d7686;">Querying get_status…</p>`
        }
      </section>
    </main>`;
}

async function main(): Promise<void> {
  render(null, null);
  const invoke = window.__TAURI__?.core.invoke;
  if (!invoke) {
    render(null, "Not running under Tauri — get_status unavailable (dev browser mode).");
    return;
  }
  try {
    const status = await invoke<StatusResponse>("get_status");
    render(status, null);
  } catch (e) {
    render(null, `get_status failed: ${String(e)}`);
  }
}

void main();
