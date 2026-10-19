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
      throw new RenderError("engine", "engine did not export renderToString");
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
    throw new Error(`SVG output exceeds the limit of ${MAX_SVG_BYTES} bytes`);
  }
  const doc = new DOMParser().parseFromString(svgText, "image/svg+xml");
  const parseError = doc.querySelector("parsererror");
  if (parseError || doc.documentElement.tagName.toLowerCase() !== "svg") {
    throw new Error("engine output is not valid SVG");
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

// ../shared/i18n/i18n.ts
var I18nError = class extends Error {
};
var NAME_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;
function isValidParamName(name) {
  return NAME_RE.test(name);
}
function interpolate(template, args = {}) {
  let out = "";
  let i = 0;
  const n = template.length;
  while (i < n) {
    const c = template[i];
    if (c === "{" && template[i + 1] === "{") {
      out += "{";
      i += 2;
      continue;
    }
    if (c === "}" && template[i + 1] === "}") {
      out += "}";
      i += 2;
      continue;
    }
    if (c === "{") {
      const end = template.indexOf("}", i + 1);
      if (end < 0) throw new I18nError("\u672A\u95ED\u5408\u7684 `{`");
      const name = template.slice(i + 1, end);
      if (!isValidParamName(name)) throw new I18nError(`\u975E\u6CD5\u5360\u4F4D\u7B26\u540D {${name}}`);
      if (!(name in args)) throw new I18nError(`\u7F3A\u5C11\u53C2\u6570 ${name}`);
      out += args[name];
      i = end + 1;
      continue;
    }
    if (c === "}") throw new I18nError("\u672A\u914D\u5BF9\u7684 `}`");
    out += c;
    i += 1;
  }
  return out;
}
var SAFE_FALLBACK = "Something went wrong";
var DEFAULT_OWNER_LOCALE = "en";
var Resolver = class {
  sets = /* @__PURE__ */ new Map();
  insert(set2) {
    this.sets.set(set2.owner, set2);
  }
  resolve(text, locale) {
    const { owner, key, fallback, args = {} } = text;
    if (key === void 0) {
      return fallback.trim() ? fallback : SAFE_FALLBACK;
    }
    const set2 = this.sets.get(owner);
    if (!set2) return this.fromFallback(fallback, args);
    const defaultLoc = set2.defaultLocale ?? DEFAULT_OWNER_LOCALE;
    for (const cand of [locale, defaultLoc]) {
      const bundle = set2.bundles[cand];
      if (!bundle) continue;
      const template = bundle[key];
      if (template === void 0) continue;
      try {
        const rendered = interpolate(template, args);
        if (rendered.trim()) return rendered;
      } catch {
      }
    }
    return this.fromFallback(fallback, args);
  }
  fromFallback(fallback, args) {
    if (fallback.trim()) {
      try {
        const rendered = interpolate(fallback, args);
        if (rendered.trim()) return rendered;
      } catch {
      }
    }
    return SAFE_FALLBACK;
  }
};

// ../../locales/panels/preview/en.json
var en_default = {
  "loading.profiling": "Detecting file\u2026",
  "loading.reserving": "Preparing preview resources\u2026",
  "loading.creating_host": "Starting preview\u2026",
  "loading.reading": "Reading file\u2026",
  "loading.indexing": "Indexing large file\u2026",
  "loading.loading_window": "Loading file content\u2026",
  "loading.parsing": "Parsing file\u2026",
  "loading.searching": "Searching the whole file\u2026",
  "loading.switching_mode": "Switching preview mode\u2026",
  "route.tabular_extension": "Table extension",
  "route.json_tree_extension": "JSON tree extension",
  "route.json_lines_extension": "JSON Lines extension",
  "route.rendered_extension": "Rendered extension .{e}",
  "route.code_extension": "Code extension",
  "route.filename_rule": "Filename rule {rule}",
  "route.media_document_extension": "Media/document extension",
  "route.archive": "Archive",
  "route.unknown_binary": "Unknown binary",
  "route.unknown_text_probe": "Unknown text (content probe)",
  "route.empty_file": "Empty file",
  "route.persisted_mode": "User-persisted mode",
  "policy.max_line_force_windowed": "Longest line {max_line} exceeds 5MiB; forced windowing ({size} bytes)",
  "policy.editable_in_budget": "Editable working set within budget and <= 30MiB",
  "policy.readonly_highlight_in_budget": "Editable over budget/30MiB; read-only highlight within budget and <= 64MiB",
  "policy.readonly_plain_in_budget": "Highlight over budget/64MiB; read-only plain text within budget and <= 128MiB",
  "policy.over_single_editor_budget": "File {size} bytes or working set exceeds the single-editor budget {budget} bytes; windowing",
  "policy.reason_detail": "{base_reason} (file {size} bytes, longest line {max_line} bytes)",
  "policy.wrap_off": ";longest line >100KiB, auto-wrap off",
  "policy.highlight_off": ";longest line >1MiB, highlight/folding off",
  "plantuml.root_too_large": "Root file too large ({bytes} bytes, limit {limit})",
  "plantuml.root_unreadable": "Cannot read root file",
  "plantuml.root_not_utf8": "Root file is not UTF-8 text",
  "plantuml.include_nested_too_deep": "include nesting too deep ({depth} levels, limit {limit})",
  "plantuml.include_too_many": "too many include files ({count}, limit {limit})",
  "plantuml.include_too_large": "include content too large ({bytes} bytes, limit {limit})",
  "plantuml.include_cycle": "include cycle detected",
  "plantuml.include_remote_disabled": "Remote include is disabled",
  "plantuml.include_file_url_disabled": "file:// include is disabled",
  "plantuml.include_absolute_disabled": "Absolute-path include is disabled",
  "plantuml.include_escapes_root": "include escapes the project root",
  "plantuml.include_unreadable": "Cannot read include file",
  "plantuml.include_not_plain_file": "include target is not a regular file",
  "plantuml.include_not_utf8": "include file is not UTF-8 text",
  "plantuml.include_unparseable": "Cannot statically parse this include",
  "resources.single_viewer_over_budget": "Single viewer {bytes} bytes exceeds the total preview budget {budget}",
  "resources.no_evictable_viewer": "Over budget and no evictable viewer",
  "resources.still_over_after_evict": "Still over budget after evicting all evictable viewers",
  "view.disk_conflict_notice": 'The file on disk changed externally; choose "Keep my changes" or "Reload from disk".',
  "view.blank": "Blank",
  "view.tab_missing": "tab does not exist",
  "view.no_text_editor": "This tab has no text editor",
  "view.no_table": "This tab is not a table",
  "view.sheet_not_loaded": "Target sheet not loaded yet",
  "view.readonly_or_not_writable": "Read-only or non-writable backend",
  "view.replace_needs_revision": "replace must carry expected_revision",
  "text_save.temp_create_failed": "Cannot create temporary save file",
  "editor.not_utf8_readonly": "This file is not valid UTF-8 text; shown read-only and saving is disabled to avoid corrupting the original file",
  "editor.utf16_readonly": "This file is UTF-16; it has been transcoded to read-only text and saving is disabled to preserve the original encoding",
  "editor.global_scroll_label": "Global file scroll position",
  "editor.long_line_truncated": "This line is too long; only the beginning is shown",
  "editor.read_failed": "Failed to read file: {status}",
  "editor.read_error": "Error reading file: {error}",
  "editor.json_parse_failed": "JSON parse failed: {error}",
  "plantuml_viewer.view_source": "View source",
  "plantuml_viewer.render_failed": "Render failed",
  "plantuml_viewer.nothing_to_render": "Nothing to render yet",
  "plantuml_viewer.empty_file": "This PlantUML file is empty.",
  "plantuml_viewer.rendering": "Rendering\u2026",
  "plantuml_viewer.engine_no_render_to_string": "Engine did not export renderToString",
  "plantuml_viewer.output_not_svg": "Engine output is not valid SVG",
  "plantuml_viewer.svg_over_limit": "SVG output exceeds the limit of {max} bytes",
  "tabular.prev_sheet_group": "Previous sheet group",
  "tabular.next_sheet_group": "Next sheet group",
  "tabular.first_rows_only": "Showing only the first 100,000 rows (the file has more; total line count not exactly determined)",
  "tabular.not_tabular_file": "not a tabular file: {ext}"
};

// ../../locales/panels/preview/zh-CN.json
var zh_CN_default = {
  "loading.profiling": "\u6B63\u5728\u8BC6\u522B\u6587\u4EF6\u2026",
  "loading.reserving": "\u6B63\u5728\u51C6\u5907\u9884\u89C8\u8D44\u6E90\u2026",
  "loading.creating_host": "\u6B63\u5728\u542F\u52A8\u9884\u89C8\u2026",
  "loading.reading": "\u6B63\u5728\u8BFB\u53D6\u6587\u4EF6\u2026",
  "loading.indexing": "\u6B63\u5728\u7D22\u5F15\u5927\u6587\u4EF6\u2026",
  "loading.loading_window": "\u6B63\u5728\u52A0\u8F7D\u6587\u4EF6\u5185\u5BB9\u2026",
  "loading.parsing": "\u6B63\u5728\u89E3\u6790\u6587\u4EF6\u2026",
  "loading.searching": "\u6B63\u5728\u641C\u7D22\u6574\u4E2A\u6587\u4EF6\u2026",
  "loading.switching_mode": "\u6B63\u5728\u5207\u6362\u9884\u89C8\u6A21\u5F0F\u2026",
  "route.tabular_extension": "\u8868\u683C\u6269\u5C55\u540D",
  "route.json_tree_extension": "JSON \u6811\u6269\u5C55\u540D",
  "route.json_lines_extension": "JSON Lines \u6269\u5C55\u540D",
  "route.rendered_extension": "\u6E32\u67D3\u6269\u5C55\u540D .{e}",
  "route.code_extension": "\u4EE3\u7801\u6269\u5C55\u540D",
  "route.filename_rule": "\u6587\u4EF6\u540D\u89C4\u5219 {rule}",
  "route.media_document_extension": "\u5A92\u4F53/\u6587\u6863\u6269\u5C55\u540D",
  "route.archive": "\u538B\u7F29\u5305",
  "route.unknown_binary": "\u672A\u77E5\u4E8C\u8FDB\u5236",
  "route.unknown_text_probe": "\u672A\u77E5\u6587\u672C(\u5185\u5BB9\u63A2\u6D4B)",
  "route.empty_file": "\u7A7A\u6587\u4EF6",
  "route.persisted_mode": "\u7528\u6237\u6301\u4E45\u5316\u6A21\u5F0F",
  "policy.max_line_force_windowed": "\u6700\u957F\u884C {max_line} \u8D85\u8FC7 5MiB,\u5F3A\u5236\u7A97\u53E3\u5316(\u6587\u4EF6 {size} \u5B57\u8282)",
  "policy.editable_in_budget": "\u53EF\u7F16\u8F91\u5DE5\u4F5C\u96C6\u5728\u9884\u7B97\u5185\u4E14 <= 30MiB",
  "policy.readonly_highlight_in_budget": "\u53EF\u7F16\u8F91\u8D85\u9884\u7B97/\u8D85 30MiB,\u53EA\u8BFB\u9AD8\u4EAE\u5728\u9884\u7B97\u5185\u4E14 <= 64MiB",
  "policy.readonly_plain_in_budget": "\u9AD8\u4EAE\u8D85\u9884\u7B97/\u8D85 64MiB,\u53EA\u8BFB\u7EAF\u6587\u672C\u5728\u9884\u7B97\u5185\u4E14 <= 128MiB",
  "policy.over_single_editor_budget": "\u6587\u4EF6 {size} \u5B57\u8282\u6216\u5DE5\u4F5C\u96C6\u8D85\u8FC7\u5355\u7F16\u8F91\u5668\u9884\u7B97 {budget} \u5B57\u8282,\u7A97\u53E3\u5316",
  "policy.reason_detail": "{base_reason}(\u6587\u4EF6 {size} \u5B57\u8282, \u6700\u957F\u884C {max_line} \u5B57\u8282)",
  "policy.wrap_off": ";\u6700\u957F\u884C >100KiB,\u5173\u95ED\u81EA\u52A8\u6362\u884C",
  "policy.highlight_off": ";\u6700\u957F\u884C >1MiB,\u5173\u95ED\u9AD8\u4EAE/\u6298\u53E0",
  "plantuml.root_too_large": "\u6839\u6587\u4EF6\u8FC7\u5927({bytes} \u5B57\u8282,\u4E0A\u9650 {limit})",
  "plantuml.root_unreadable": "\u65E0\u6CD5\u8BFB\u53D6\u6839\u6587\u4EF6",
  "plantuml.root_not_utf8": "\u6839\u6587\u4EF6\u4E0D\u662F UTF-8 \u6587\u672C",
  "plantuml.include_nested_too_deep": "include \u5D4C\u5957\u8FC7\u6DF1({depth} \u5C42,\u4E0A\u9650 {limit})",
  "plantuml.include_too_many": "include \u6587\u4EF6\u8FC7\u591A({count} \u4E2A,\u4E0A\u9650 {limit})",
  "plantuml.include_too_large": "include \u5185\u5BB9\u8FC7\u5927({bytes} \u5B57\u8282,\u4E0A\u9650 {limit})",
  "plantuml.include_cycle": "include \u5B58\u5728\u73AF",
  "plantuml.include_remote_disabled": "\u8FDC\u7A0B include \u5DF2\u7981\u7528",
  "plantuml.include_file_url_disabled": "file:// include \u5DF2\u7981\u7528",
  "plantuml.include_absolute_disabled": "\u7EDD\u5BF9\u8DEF\u5F84 include \u5DF2\u7981\u7528",
  "plantuml.include_escapes_root": "include \u8D8A\u51FA\u9879\u76EE\u6839",
  "plantuml.include_unreadable": "\u65E0\u6CD5\u8BFB\u53D6 include \u6587\u4EF6",
  "plantuml.include_not_plain_file": "include \u76EE\u6807\u4E0D\u662F\u666E\u901A\u6587\u4EF6",
  "plantuml.include_not_utf8": "include \u6587\u4EF6\u4E0D\u662F UTF-8 \u6587\u672C",
  "plantuml.include_unparseable": "\u65E0\u6CD5\u9759\u6001\u89E3\u6790\u8BE5 include",
  "resources.single_viewer_over_budget": "\u5355 viewer {bytes} \u5B57\u8282\u8D85\u8FC7\u603B\u9884\u89C8\u9884\u7B97 {budget}",
  "resources.no_evictable_viewer": "\u8D85\u9884\u7B97\u4E14\u6CA1\u6709\u53EF\u6DD8\u6C70\u7684 viewer",
  "resources.still_over_after_evict": "\u5373\u4F7F\u6DD8\u6C70\u5168\u90E8\u53EF\u6DD8\u6C70 viewer \u4ECD\u8D85\u9884\u7B97",
  "view.disk_conflict_notice": "\u78C1\u76D8\u6587\u4EF6\u5DF2\u88AB\u5916\u90E8\u4FEE\u6539,\u8BF7\u9009\u62E9\u300C\u4FDD\u7559\u6211\u7684\u4FEE\u6539\u300D\u6216\u300C\u91CD\u8F7D\u78C1\u76D8\u300D\u3002",
  "view.blank": "\u7A7A\u767D",
  "view.tab_missing": "tab \u4E0D\u5B58\u5728",
  "view.no_text_editor": "\u8BE5 tab \u65E0\u6587\u672C\u7F16\u8F91\u5668",
  "view.no_table": "\u8BE5 tab \u4E0D\u662F\u8868\u683C",
  "view.sheet_not_loaded": "\u76EE\u6807 sheet \u5C1A\u672A\u52A0\u8F7D",
  "view.readonly_or_not_writable": "\u53EA\u8BFB\u6216\u4E0D\u53EF\u5199 backend",
  "view.replace_needs_revision": "replace \u5FC5\u987B\u643A\u5E26 expected_revision",
  "text_save.temp_create_failed": "\u65E0\u6CD5\u521B\u5EFA\u4FDD\u5B58\u4E34\u65F6\u6587\u4EF6",
  "editor.not_utf8_readonly": "\u8BE5\u6587\u4EF6\u4E0D\u662F\u6709\u6548\u7684 UTF-8 \u6587\u672C,\u4EC5\u53EA\u8BFB\u663E\u793A;\u4FDD\u5B58\u5DF2\u7981\u7528\u4EE5\u514D\u7834\u574F\u539F\u6587\u4EF6",
  "editor.utf16_readonly": "\u8BE5\u6587\u4EF6\u662F UTF-16 \u7F16\u7801,\u5DF2\u8F6C\u7801\u4E3A\u53EA\u8BFB\u6587\u672C;\u4FDD\u5B58\u5DF2\u7981\u7528\u4EE5\u514D\u6539\u53D8\u539F\u7F16\u7801",
  "editor.global_scroll_label": "\u5168\u5C40\u6587\u4EF6\u6EDA\u52A8\u4F4D\u7F6E",
  "editor.long_line_truncated": "\u672C\u884C\u8FC7\u957F,\u4EC5\u663E\u793A\u5F00\u5934\u90E8\u5206",
  "editor.read_failed": "\u8BFB\u53D6\u6587\u4EF6\u5931\u8D25: {status}",
  "editor.read_error": "\u8BFB\u53D6\u6587\u4EF6\u5F02\u5E38: {error}",
  "editor.json_parse_failed": "JSON \u89E3\u6790\u5931\u8D25: {error}",
  "plantuml_viewer.view_source": "\u67E5\u770B\u6E90\u7801",
  "plantuml_viewer.render_failed": "\u6E32\u67D3\u5931\u8D25",
  "plantuml_viewer.nothing_to_render": "\u6682\u65E0\u53EF\u6E32\u67D3\u5185\u5BB9",
  "plantuml_viewer.empty_file": "\u8BE5 PlantUML \u6587\u4EF6\u4E3A\u7A7A\u3002",
  "plantuml_viewer.rendering": "\u6B63\u5728\u6E32\u67D3\u2026",
  "plantuml_viewer.engine_no_render_to_string": "\u5F15\u64CE\u672A\u5BFC\u51FA renderToString",
  "plantuml_viewer.output_not_svg": "\u5F15\u64CE\u8F93\u51FA\u4E0D\u662F\u5408\u6CD5 SVG",
  "plantuml_viewer.svg_over_limit": "SVG \u8F93\u51FA\u8D85\u8FC7\u4E0A\u9650 {max} \u5B57\u8282",
  "tabular.prev_sheet_group": "\u4E0A\u4E00\u7EC4 sheet",
  "tabular.next_sheet_group": "\u4E0B\u4E00\u7EC4 sheet",
  "tabular.first_rows_only": "\u4EC5\u663E\u793A\u524D 100,000 \u884C(\u6587\u4EF6\u8FD8\u6709\u66F4\u591A,\u672A\u7CBE\u786E\u7EDF\u8BA1\u603B\u884C\u6570)",
  "tabular.not_tabular_file": "\u975E\u8868\u683C\u6587\u4EF6: {ext}"
};

// src/i18n.ts
var PREVIEW_OWNER = "dozer.preview";
var DEFAULT_LOCALE = "en";
var bundles = {
  en: en_default,
  "zh-CN": zh_CN_default
};
var set = {
  owner: PREVIEW_OWNER,
  defaultLocale: DEFAULT_LOCALE,
  bundles
};
var resolver = new Resolver();
resolver.insert(set);
var currentLocale = DEFAULT_LOCALE;
function setLocale(locale) {
  currentLocale = locale;
}
function t(key, fallback, args) {
  return resolver.resolve({ owner: PREVIEW_OWNER, key, fallback, args }, currentLocale);
}

// src/index.ts
function qs() {
  return new URLSearchParams(location.search);
}
setLocale(qs().get("locale") || "en");
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
  showStatus(
    t("plantuml_viewer.render_failed", "Render failed"),
    line !== null ? `${message} (line ${line})` : message,
    true
  );
  postEvent({ kind: "failed", failure_kind: kind, message, line }, revision);
}
async function setDocument(cmd) {
  currentRevision = cmd.revision;
  applyTheme(cmd.theme);
  setLocalIncludes(cmd.includes);
  content.innerHTML = "";
  diagnostics.textContent = "";
  if (cmd.source.trim() === "") {
    showStatus(
      t("plantuml_viewer.nothing_to_render", "Nothing to render yet"),
      t("plantuml_viewer.empty_file", "This PlantUML file is empty.")
    );
    postEvent({ kind: "rendered", width: 0, height: 0, duration_ms: 0 }, cmd.revision);
    return;
  }
  showStatus(t("plantuml_viewer.rendering", "Rendering\u2026"));
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
    diagnostics.textContent = `${width}\xD7${height} \xB7 ${duration}ms${removed ? ` \xB7 cleaned ${removed}` : ""}`;
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
