// src/types.ts
var PROTOCOL_VERSION = 1;

// src/renderer.ts
var RenderError = class extends Error {
  kind;
  line;
  constructor(kind, message, line = null) {
    super(message);
    this.name = "RenderError";
    this.kind = kind;
    this.line = line;
  }
};
var ASSET_BASE = "dozer://plantuml-viewer/";
var VENDORED_STDLIB_SCRIPTS = ["c4.min.js"];
var stdlibStore = {};
var stdlibJsonStore = {};
var stdlibInfoStore = {};
var enginePromise = null;
var renderQueue = Promise.resolve();
function globals() {
  return globalThis;
}
function ensureNamespaces() {
  const g = globals();
  g.PLANTUML_STDLIB = stdlibStore;
  g.PLANTUML_STDLIB_JSON = stdlibJsonStore;
  g.PLANTUML_STDLIB_INFO = stdlibInfoStore;
  g.PLANTUML_STDLIB_LOADER = (name, ok) => {
    const base = String(name).replace(/\.(min\.)?js$/, "");
    const ns = stdlibStore[base] ?? stdlibJsonStore[base];
    if (!ns) {
      return false;
    }
    ok();
    return true;
  };
}
function loadScript(src) {
  return new Promise((resolve, reject) => {
    const el = document.createElement("script");
    el.src = src;
    el.onload = () => resolve();
    el.onerror = () => reject(new RenderError("engine", `\u65E0\u6CD5\u52A0\u8F7D\u5F15\u64CE\u8D44\u6E90 ${src}`));
    document.head.appendChild(el);
  });
}
function loadEngine() {
  if (enginePromise) {
    return enginePromise;
  }
  enginePromise = (async () => {
    ensureNamespaces();
    await loadScript(`${ASSET_BASE}viz-global.js`);
    for (const name of VENDORED_STDLIB_SCRIPTS) {
      await loadScript(`${ASSET_BASE}stdlib/${name}`);
    }
    const mod = await import(
      /* @vite-ignore */
      `${ASSET_BASE}plantuml.js`
    );
    if (typeof mod.renderToString !== "function") {
      throw new RenderError("engine", "\u5F15\u64CE\u672A\u5BFC\u51FA renderToString");
    }
    return mod;
  })();
  return enginePromise;
}
function setLocalIncludes(includes) {
  const local = {};
  for (const inc of includes) {
    const key = inc.path.replace(/\\/g, "/");
    local[key] = inc.content.split(/\r\n|\r|\n/);
  }
  stdlibStore["local"] = local;
}
function classifyEngineError(err) {
  if (err instanceof RenderError) {
    return err;
  }
  const message = err instanceof Error ? err.message : String(err);
  const lineMatch = message.match(/line\s+(\d+)/i) ?? message.match(/第\s*(\d+)\s*行/);
  const line = lineMatch ? Number.parseInt(lineMatch[1], 10) : null;
  return new RenderError("syntax", message, line);
}
function renderToString(source) {
  const run = renderQueue.then(async () => {
    const engine = await loadEngine();
    const lines = source.split(/\r\n|\r|\n/);
    return await new Promise((resolve, reject) => {
      try {
        engine.renderToString(
          lines,
          (svg) => resolve(svg),
          (err) => reject(classifyEngineError(err))
        );
      } catch (err) {
        reject(classifyEngineError(err));
      }
    });
  });
  renderQueue = run.catch(() => void 0);
  return run;
}

// src/sanitize.ts
var SVG_NS = "http://www.w3.org/2000/svg";
var MAX_SVG_BYTES = 32 * 1024 * 1024;
var BANNED_ELEMENTS = /* @__PURE__ */ new Set(["script", "foreignobject", "iframe", "object", "embed"]);
var URL_ATTRS = /* @__PURE__ */ new Set(["href", "xlink:href", "src", "xlink:src"]);
function isSafeUrl(value) {
  const v = value.trim().toLowerCase();
  return v.startsWith("#") || v === "";
}
function walk(el, onRemove) {
  const children = Array.from(el.children);
  for (const child of children) {
    if (BANNED_ELEMENTS.has(child.tagName.toLowerCase())) {
      child.remove();
      onRemove();
      continue;
    }
    walk(child, onRemove);
  }
  for (const attr of Array.from(el.attributes)) {
    const name = attr.name.toLowerCase();
    if (name.startsWith("on")) {
      el.removeAttribute(attr.name);
      onRemove();
      continue;
    }
    if (URL_ATTRS.has(name) && !isSafeUrl(attr.value)) {
      el.removeAttribute(attr.name);
      onRemove();
      continue;
    }
  }
  if (el.tagName.toLowerCase() === "a") {
    el.removeAttribute("href");
    el.removeAttribute("xlink:href");
  }
}
function sanitizeSvg(svgText) {
  if (new TextEncoder().encode(svgText).length > MAX_SVG_BYTES) {
    throw new Error(`SVG \u8F93\u51FA\u8D85\u8FC7\u4E0A\u9650 ${MAX_SVG_BYTES} \u5B57\u8282`);
  }
  const doc = new DOMParser().parseFromString(svgText, "image/svg+xml");
  const parseError = doc.querySelector("parsererror");
  if (parseError || doc.documentElement.tagName.toLowerCase() !== "svg") {
    throw new Error("\u5F15\u64CE\u8F93\u51FA\u4E0D\u662F\u5408\u6CD5 SVG");
  }
  const svg = doc.documentElement;
  let removed = 0;
  walk(svg, () => {
    removed += 1;
  });
  const clean = new DOMParser().parseFromString(
    new XMLSerializer().serializeToString(svg),
    "image/svg+xml"
  ).documentElement;
  clean.setAttribute("xmlns", SVG_NS);
  clean.removeAttribute("data-plantuml-src");
  return { svg: clean, removed };
}

// src/viewport.ts
var MIN_SCALE = 0.1;
var MAX_SCALE = 40;
var ZOOM_STEP = 1.15;
var Viewport = class {
  host;
  content;
  state = { scale: 1, x: 0, y: 0 };
  dragging = false;
  dragStart = { x: 0, y: 0, tx: 0, ty: 0 };
  contentSize = { width: 0, height: 0 };
  constructor(host, content2) {
    this.host = host;
    this.content = content2;
    this.host.addEventListener("wheel", this.onWheel, { passive: false });
    this.host.addEventListener("mousedown", this.onMouseDown);
    window.addEventListener("mousemove", this.onMouseMove);
    window.addEventListener("mouseup", this.onMouseUp);
  }
  setContentSize(width, height) {
    this.contentSize = { width, height };
  }
  getState() {
    return { ...this.state };
  }
  apply() {
    const { scale, x, y } = this.state;
    this.content.style.transform = `translate(${x}px, ${y}px) scale(${scale})`;
  }
  onWheel = (ev) => {
    ev.preventDefault();
    const rect = this.host.getBoundingClientRect();
    const cx = ev.clientX - rect.left;
    const cy = ev.clientY - rect.top;
    const factor = ev.deltaY < 0 ? ZOOM_STEP : 1 / ZOOM_STEP;
    this.zoomAt(cx, cy, factor);
  };
  zoomAt(cx, cy, factor) {
    const next = clamp(this.state.scale * factor, MIN_SCALE, MAX_SCALE);
    const ratio = next / this.state.scale;
    this.state.x = cx - (cx - this.state.x) * ratio;
    this.state.y = cy - (cy - this.state.y) * ratio;
    this.state.scale = next;
    this.apply();
  }
  zoomIn() {
    const rect = this.host.getBoundingClientRect();
    this.zoomAt(rect.width / 2, rect.height / 2, ZOOM_STEP);
  }
  zoomOut() {
    const rect = this.host.getBoundingClientRect();
    this.zoomAt(rect.width / 2, rect.height / 2, 1 / ZOOM_STEP);
  }
  actualSize() {
    this.state = { scale: 1, x: 0, y: 0 };
    this.apply();
  }
  reset() {
    this.fit();
  }
  fit() {
    const rect = this.host.getBoundingClientRect();
    const w = this.contentSize.width || 1;
    const h = this.contentSize.height || 1;
    if (rect.width === 0 || rect.height === 0) {
      return;
    }
    const scale = Math.min(rect.width / w, rect.height / h) * 0.98;
    const clamped = clamp(scale, MIN_SCALE, MAX_SCALE);
    this.state = {
      scale: clamped,
      x: (rect.width - w * clamped) / 2,
      y: (rect.height - h * clamped) / 2
    };
    this.apply();
  }
  onMouseDown = (ev) => {
    if (ev.button !== 0) {
      return;
    }
    this.dragging = true;
    this.dragStart = { x: ev.clientX, y: ev.clientY, tx: this.state.x, ty: this.state.y };
    this.host.style.cursor = "grabbing";
    ev.preventDefault();
  };
  onMouseMove = (ev) => {
    if (!this.dragging) {
      return;
    }
    this.state.x = this.dragStart.tx + (ev.clientX - this.dragStart.x);
    this.state.y = this.dragStart.ty + (ev.clientY - this.dragStart.y);
    this.apply();
  };
  onMouseUp = () => {
    if (!this.dragging) {
      return;
    }
    this.dragging = false;
    this.host.style.cursor = "";
  };
  dispose() {
    this.host.removeEventListener("wheel", this.onWheel);
    this.host.removeEventListener("mousedown", this.onMouseDown);
    window.removeEventListener("mousemove", this.onMouseMove);
    window.removeEventListener("mouseup", this.onMouseUp);
  }
};
function clamp(v, lo, hi) {
  return Math.min(hi, Math.max(lo, v));
}

// src/index.ts
function qs() {
  return new URLSearchParams(location.search);
}
function postEvent(event, revision = 0) {
  const q = qs();
  const envelope = {
    protocol_version: PROTOCOL_VERSION,
    project_id: Number(q.get("proj") || 0) || 0,
    panel: q.get("panel") || "files",
    tab_id: Number(q.get("tab") || 0) || 0,
    document_id: q.get("doc") || "",
    revision,
    request_id: null,
    payload: event
  };
  try {
    window.ipc.postMessage(JSON.stringify(envelope));
  } catch {
  }
}
var stage = document.getElementById("stage");
var content = document.getElementById("content");
var status = document.getElementById("status");
var diagnostics = document.getElementById("diagnostics");
var viewport = new Viewport(stage, content);
function showStatus(title, detail, error = false) {
  status.classList.remove("hidden");
  status.innerHTML = "";
  const t = document.createElement("div");
  t.className = "title";
  t.textContent = title;
  status.appendChild(t);
  if (detail) {
    const d = document.createElement("div");
    d.className = error ? "error" : "";
    d.textContent = detail;
    status.appendChild(d);
  }
  if (error) {
    const btn = document.createElement("button");
    btn.textContent = "\u67E5\u770B\u6E90\u7801";
    btn.addEventListener("click", () => postEvent({ kind: "open_source", line: null }));
    status.appendChild(btn);
  }
}
function hideStatus() {
  status.classList.add("hidden");
  status.innerHTML = "";
}
function applyTheme(theme) {
  document.documentElement.classList.toggle("theme-light", theme === "light");
}
var currentRevision = 0;
function fail(revision, kind, message, line) {
  if (revision < currentRevision) {
    return;
  }
  showStatus("\u6E32\u67D3\u5931\u8D25", `${message}${line ? `\uFF08\u7B2C ${line} \u884C\uFF09` : ""}`, true);
  postEvent({ kind: "failed", failure_kind: kind, message, line }, revision);
}
async function setDocument(cmd) {
  currentRevision = cmd.revision;
  applyTheme(cmd.theme);
  setLocalIncludes(cmd.includes);
  content.innerHTML = "";
  diagnostics.textContent = "";
  if (cmd.source.trim() === "") {
    showStatus("\u6682\u65E0\u53EF\u6E32\u67D3\u5185\u5BB9", "\u8BE5 PlantUML \u6587\u4EF6\u4E3A\u7A7A\u3002");
    postEvent({ kind: "rendered", width: 0, height: 0, duration_ms: 0 }, cmd.revision);
    return;
  }
  showStatus("\u6B63\u5728\u6E32\u67D3\u2026");
  const t0 = performance.now();
  try {
    const svgText = await renderToString(cmd.source);
    if (cmd.revision < currentRevision) {
      return;
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
    diagnostics.textContent = `${width}\xD7${height} \xB7 ${duration}ms${removed ? ` \xB7 \u6E05\u7406 ${removed}` : ""}`;
    if (cmd.revision === currentRevision) {
      postEvent({ kind: "rendered", width, height, duration_ms: duration }, cmd.revision);
    }
  } catch (err) {
    const e = classifyEngineError(err);
    fail(cmd.revision, e.kind, e.message, e.line);
  }
}
function dispatch(raw) {
  let env;
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
document.getElementById("btn-fit")?.addEventListener("click", () => viewport.fit());
document.getElementById("btn-100")?.addEventListener("click", () => viewport.actualSize());
document.getElementById("btn-zoom-in")?.addEventListener("click", () => viewport.zoomIn());
document.getElementById("btn-zoom-out")?.addEventListener("click", () => viewport.zoomOut());
document.getElementById("btn-reset")?.addEventListener("click", () => viewport.reset());
postEvent({ kind: "ready" });
