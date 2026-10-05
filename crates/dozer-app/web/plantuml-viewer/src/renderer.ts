//! Adapter over the vendored official `@plantuml/core` (TeaVM) engine.
//!
//! The engine is loaded as static assets served from `dozer://plantuml-viewer/`:
//! `viz-global.js` must run as a classic script first (it defines the Graphviz
//! globals), then the ESM `plantuml.js` is imported. This module hides the
//! engine's global-namespace contract and TeaVM callback style behind a small
//! promise API so the UI never touches engine symbols directly.

import type { PlantUmlInclude, PlantUmlFailureKind } from "./types.ts";

/** `renderToString` from the engine's ESM entry (typed locally, no .d.ts upstream). */
type RenderToString = (
  lines: string[],
  onSuccess: (svg: string) => void,
  onError: (err: unknown) => void,
) => void;

interface EngineModule {
  renderToString: RenderToString;
}

/**
 * Engine global namespace contract (Task 0 §12.3). The engine reads these via
 * `globalThis ?? self ?? window`. In a browser `globalThis === window`, so a
 * single assignment on `globalThis` is enough.
 */
interface PlantUmlGlobals {
  PLANTUML_STDLIB?: Record<string, Record<string, string | string[]>>;
  PLANTUML_STDLIB_JSON?: Record<string, Record<string, string>>;
  PLANTUML_STDLIB_INFO?: Record<string, unknown>;
  PLANTUML_STDLIB_LOADER?: (
    name: string,
    ok: () => void,
    fail: (err?: unknown) => void,
  ) => unknown;
}

export class RenderError extends Error {
  readonly kind: PlantUmlFailureKind;
  readonly line: number | null;
  constructor(kind: PlantUmlFailureKind, message: string, line: number | null = null) {
    super(message);
    this.name = "RenderError";
    this.kind = kind;
    this.line = line;
  }
}

/** Base URL the vendored assets are served from. */
const ASSET_BASE = "dozer://plantuml-viewer/";

/**
 * Vendored stdlib packages to register at engine bootstrap (spec §12.3). Each is
 * a classic script under `stdlib/` that self-assigns the `PLANTUML_STDLIB*`
 * namespaces on `window`; loaded same-origin so `script-src 'self'` permits it.
 */
const VENDORED_STDLIB_SCRIPTS = ["c4.min.js"] as const;

/** Namespaces registered by vendored stdlib packages keyed by library base. */
const stdlibStore: Record<string, Record<string, string | string[]>> = {};
const stdlibJsonStore: Record<string, Record<string, string>> = {};
const stdlibInfoStore: Record<string, unknown> = {};

let enginePromise: Promise<EngineModule> | null = null;
/** Renders are serialized: the engine is async and shares global state. */
let renderQueue: Promise<unknown> = Promise.resolve();

function globals(): PlantUmlGlobals {
  return globalThis as unknown as PlantUmlGlobals;
}

/** Ensure the virtual-file-system namespaces exist before the engine loads. */
function ensureNamespaces(): void {
  const g = globals();
  g.PLANTUML_STDLIB = stdlibStore;
  g.PLANTUML_STDLIB_JSON = stdlibJsonStore;
  g.PLANTUML_STDLIB_INFO = stdlibInfoStore;
  // Override the engine's default script-injecting loader. `name` is the
  // script filename (e.g. "c4.min.js"); the namespace key is the base name
  // (e.g. "c4"). We satisfy loads synchronously from the vendored namespaces,
  // and return false to refuse any load we do not have vendored (no network).
  g.PLANTUML_STDLIB_LOADER = (name, ok /*, fail */) => {
    const base = String(name).replace(/\.(min\.)?js$/, "");
    const ns = stdlibStore[base] ?? stdlibJsonStore[base];
    if (!ns) {
      return false;
    }
    ok();
    return true;
  };
}

function loadScript(src: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const el = document.createElement("script");
    el.src = src;
    el.onload = () => resolve();
    el.onerror = () => reject(new RenderError("engine", `无法加载引擎资源 ${src}`));
    document.head.appendChild(el);
  });
}

/** Lazily bootstrap the engine exactly once. */
export function loadEngine(): Promise<EngineModule> {
  if (enginePromise) {
    return enginePromise;
  }
  enginePromise = (async () => {
    ensureNamespaces();
    // viz-global.js must run first and as a classic script (defines Graphviz).
    await loadScript(`${ASSET_BASE}viz-global.js`);
    // Register the vendored stdlib packages (e.g. C4) by loading their classic
    // scripts same-origin. They self-assign `window.PLANTUML_STDLIB[_JSON/_INFO]`,
    // so the engine's `PLANTUML_STDLIB_LOADER` can satisfy `!include <C4/...>`
    // synchronously without any network. `script-src 'self'` allows this; a
    // `new Function` installer would be rejected (no `unsafe-eval`).
    for (const name of VENDORED_STDLIB_SCRIPTS) {
      await loadScript(`${ASSET_BASE}stdlib/${name}`);
    }
    const mod = (await import(
      /* @vite-ignore */ `${ASSET_BASE}plantuml.js`
    )) as EngineModule;
    if (typeof mod.renderToString !== "function") {
      throw new RenderError("engine", "引擎未导出 renderToString");
    }
    return mod;
  })();
  return enginePromise;
}

/**
 * Inject project-local `!include` files as virtual files under the `local/`
 * stdlib base. Rust has already rewritten every project-local include in the
 * source and in each include's content to `!include <local/<project-relative>>`
 * (the browser engine only consults `PLANTUML_STDLIB` for the angle-bracket
 * stdlib form; plain relative includes are silently dropped). So the key here
 * is exactly `inc.path`, the project-relative normalized path.
 */
export function setLocalIncludes(includes: PlantUmlInclude[]): void {
  const local: Record<string, string[]> = {};
  for (const inc of includes) {
    const key = inc.path.replace(/\\/g, "/");
    local[key] = inc.content.split(/\r\n|\r|\n/);
  }
  stdlibStore["local"] = local;
}

/** Normalize raw engine error text into a failure kind + optional line. */
export function classifyEngineError(err: unknown): RenderError {
  if (err instanceof RenderError) {
    return err;
  }
  const message = err instanceof Error ? err.message : String(err);
  // Engine syntax errors carry the offending 1-based line in their text.
  const lineMatch = message.match(/line\s+(\d+)/i) ?? message.match(/第\s*(\d+)\s*行/);
  const line = lineMatch ? Number.parseInt(lineMatch[1]!, 10) : null;
  return new RenderError("syntax", message, line);
}

/** Render one diagram to an SVG string. Serialized against all other renders. */
export function renderToString(source: string): Promise<string> {
  const run = renderQueue.then(async () => {
    const engine = await loadEngine();
    const lines = source.split(/\r\n|\r|\n/);
    return await new Promise<string>((resolve, reject) => {
      try {
        engine.renderToString(
          lines,
          (svg) => resolve(svg),
          (err) => reject(classifyEngineError(err)),
        );
      } catch (err) {
        reject(classifyEngineError(err));
      }
    });
  });
  // Keep the chain alive even after a rejection so the next render can run.
  renderQueue = run.catch(() => undefined);
  return run;
}
