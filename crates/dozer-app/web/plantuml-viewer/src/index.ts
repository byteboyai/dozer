//! PlantUML viewer host bootstrap.
//!
//! Rust → host commands arrive via `window.__dozer.dispatch(envelopeJson)` (the
//! shared injection channel). Host → Rust events go through the `WebviewEnvelope`
//! bridge (`window.ipc.postMessage`) with project/panel/tab/doc taken from the
//! URL query string — the host never self-reports its binding.

import "./style.css";
import { PROTOCOL_VERSION } from "./types.ts";
import type { PlantUmlCommand, PlantUmlEvent, PlantUmlFailureKind } from "./types.ts";
import { renderToString, classifyEngineError, setLocalIncludes } from "./renderer.ts";
import { sanitizeSvg } from "./sanitize.ts";
import { Viewport } from "./viewport.ts";
import { setLocale, t } from "./i18n.ts";

function qs(): URLSearchParams {
  return new URLSearchParams(location.search);
}

// 语言由 Rust 经 URL `&locale=` 传入(与 `&theme=` 同一机制)。
setLocale(qs().get("locale") || "en");

function postEvent(event: PlantUmlEvent, revision = 0): void {
  const q = qs();
  const envelope = {
    protocol_version: PROTOCOL_VERSION,
    project_id: Number(q.get("proj") || 0) || 0,
    panel: q.get("panel") || "files",
    tab_id: Number(q.get("tab") || 0) || 0,
    document_id: q.get("doc") || "",
    revision,
    request_id: null,
    payload: event,
  };
  try {
    window.ipc.postMessage(JSON.stringify(envelope));
  } catch {
    // No bridge yet (e.g. fixture/test page): silently ignore.
  }
}

const stage = document.getElementById("stage") as HTMLElement;
const content = document.getElementById("content") as HTMLElement;
const status = document.getElementById("status") as HTMLElement;
const diagnostics = document.getElementById("diagnostics") as HTMLElement;

const viewport = new Viewport(stage, content);

function showStatus(title: string, detail?: string, error = false): void {
  status.classList.remove("hidden");
  status.innerHTML = "";
  const titleEl = document.createElement("div");
  titleEl.className = "title";
  titleEl.textContent = title;
  status.appendChild(titleEl);
  if (detail) {
    const d = document.createElement("div");
    d.className = error ? "error" : "";
    d.textContent = detail;
    status.appendChild(d);
  }
  if (error) {
    const btn = document.createElement("button");
    btn.textContent = t("plantuml_viewer.view_source", "View source");
    btn.addEventListener("click", () => postEvent({ kind: "open_source", line: null }));
    status.appendChild(btn);
  }
}

function hideStatus(): void {
  status.classList.add("hidden");
  status.innerHTML = "";
}

function applyTheme(theme: string): void {
  document.documentElement.classList.toggle("theme-light", theme === "light");
}

/** Latest revision whose result we still care about; older results are dropped. */
let currentRevision = 0;

function fail(revision: number, kind: PlantUmlFailureKind, message: string, line: number | null): void {
  if (revision < currentRevision) {
    return;
  }
  showStatus(
    t("plantuml_viewer.render_failed", "Render failed"),
    line !== null
      ? `${message} (line ${line})`
      : message,
    true,
  );
  postEvent({ kind: "failed", failure_kind: kind, message, line }, revision);
}

async function setDocument(cmd: Extract<PlantUmlCommand, { kind: "set_document" }>): Promise<void> {
  currentRevision = cmd.revision;
  applyTheme(cmd.theme);
  setLocalIncludes(cmd.includes);
  content.innerHTML = "";
  diagnostics.textContent = "";
  if (cmd.source.trim() === "") {
    showStatus(
      t("plantuml_viewer.nothing_to_render", "Nothing to render yet"),
      t("plantuml_viewer.empty_file", "This PlantUML file is empty."),
    );
    postEvent({ kind: "rendered", width: 0, height: 0, duration_ms: 0 }, cmd.revision);
    return;
  }
  showStatus(t("plantuml_viewer.rendering", "Rendering…"));
  const t0 = performance.now();
  try {
    const svgText = await renderToString(cmd.source);
    if (cmd.revision < currentRevision) {
      return; // superseded by a newer revision
    }
    const { svg, removed } = sanitizeSvg(svgText);
    content.innerHTML = "";
    content.appendChild(svg);
    const box = svg.getBoundingClientRect();
    const width = Math.round(box.width) || 0;
    const height = Math.round(box.height) || 0;
    viewport.setContentSize(width || svg.clientWidth, height || svg.clientHeight);
    hideStatus();
    viewport.fit();
    const duration = Math.round(performance.now() - t0);
    diagnostics.textContent = `${width}×${height} · ${duration}ms${removed ? ` · cleaned ${removed}` : ""}`;
    if (cmd.revision === currentRevision) {
      postEvent({ kind: "rendered", width, height, duration_ms: duration }, cmd.revision);
    }
  } catch (err) {
    const e = classifyEngineError(err);
    fail(cmd.revision, e.kind, e.message, e.line);
  }
}

function dispatch(raw: string): void {
  let env: { payload?: PlantUmlCommand };
  try {
    env = JSON.parse(raw);
  } catch {
    return;
  }
  const cmd = env.payload;
  if (!cmd) {
    return;
  }
  switch (cmd.kind) {
    case "set_document":
      void setDocument(cmd);
      break;
    case "fit_view":
      viewport.fit();
      break;
    case "actual_size":
      viewport.actualSize();
      break;
    case "reset_view":
      viewport.reset();
      break;
  }
}

window.__dozer = { dispatch };

// Toolbar wiring.
document.getElementById("btn-fit")?.addEventListener("click", () => viewport.fit());
document.getElementById("btn-100")?.addEventListener("click", () => viewport.actualSize());
document.getElementById("btn-zoom-in")?.addEventListener("click", () => viewport.zoomIn());
document.getElementById("btn-zoom-out")?.addEventListener("click", () => viewport.zoomOut());
document.getElementById("btn-reset")?.addEventListener("click", () => viewport.reset());

declare global {
  interface Window {
    __dozer?: { dispatch: (raw: string) => void };
    ipc: { postMessage: (s: string) => void };
  }
}

// Signal readiness (script boot complete; not that a diagram is rendered).
postEvent({ kind: "ready" });
