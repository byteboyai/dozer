// Shared jsdom harness for the Task 0 spike. The @plantuml/core engine (TeaVM
// output) expects a browser global environment: `document`, `window`, canvas
// metrics, DOMParser, etc. This module builds a jsdom window, mirrors the
// needed globals onto `globalThis` (a real browser has window === globalThis;
// Node ESM does not, and the engine reads some globals via globalThis), and
// returns helpers.
//
// Throwaway validation code, not production.

import { JSDOM } from 'jsdom';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const here = path.dirname(fileURLToPath(import.meta.url));
export const coreDir = path.join(here, 'node_modules/@plantuml/core');
export const fixturesDir = path.join(here, 'fixtures');

const GLOBALS = [
  'window', 'document', 'navigator', 'location', 'history',
  'MutationObserver', 'DOMParser', 'XMLSerializer', 'Blob', 'File', 'FileReader',
  'Image', 'getComputedStyle', 'requestAnimationFrame', 'cancelAnimationFrame',
  'HTMLElement', 'SVGElement', 'Node', 'Event', 'CustomEvent',
];

export function createHarness({ blockNetwork = true } = {}) {
  const dom = new JSDOM(
    `<!doctype html><html><head></head><body><div id="out"></div></body></html>`,
    { pretendToBeVisual: true, url: 'https://plantuml.invalid/' },
  );
  const { window } = dom;
  for (const name of GLOBALS) {
    if (window[name] !== undefined && globalThis[name] === undefined) {
      globalThis[name] = window[name];
    }
  }

  // canvas: jsdom has no 2d context. Mock the metrics + image-data API the
  // engine and viz-global touch. A real WKWebView provides these natively.
  window.HTMLCanvasElement.prototype.getContext = function getContext() {
    return {
      font: '',
      measureText(text) {
        const s = 12;
        return {
          width: String(text).length * s * 0.6,
          actualBoundingBoxAscent: s * 0.8,
          actualBoundingBoxDescent: s * 0.2,
          fontBoundingBoxAscent: s * 0.8,
          fontBoundingBoxDescent: s * 0.2,
        };
      },
      createImageData(w, h) {
        return { width: w, height: h, data: new Uint8ClampedArray(w * h * 4) };
      },
      getImageData(x, y, w, h) {
        return { width: w, height: h, data: new Uint8ClampedArray(w * h * 4) };
      },
      putImageData() {},
      drawImage() {},
    };
  };
  window.SVGElement.prototype.getBBox = () => ({ x: 0, y: 0, width: 10, height: 10 });

  const networkAttempts = [];
  if (blockNetwork) {
    class BlockedXHR {
      open(method, url) {
        networkAttempts.push(`${method} ${url}`);
        throw new Error(`NETWORK BLOCKED: ${method} ${url}`);
      }
    }
    globalThis.XMLHttpRequest = BlockedXHR;
    window.XMLHttpRequest = BlockedXHR;
    const blockedFetch = (url) => {
      networkAttempts.push(`fetch ${url}`);
      return Promise.reject(new Error(`NETWORK BLOCKED: fetch ${url}`));
    };
    globalThis.fetch = blockedFetch;
    window.fetch = blockedFetch;
  }

  // Register the stdlib virtual file system namespaces on BOTH window and
  // globalThis. The engine reads them via `globalThis ?? self ?? window`.
  const stdlib = {};
  const stdlibJson = {};
  const stdlibInfo = {};
  for (const ns of ['PLANTUML_STDLIB', 'PLANTUML_STDLIB_JSON', 'PLANTUML_STDLIB_INFO']) {
    const value = ns === 'PLANTUML_STDLIB' ? stdlib
      : ns === 'PLANTUML_STDLIB_JSON' ? stdlibJson : stdlibInfo;
    window[ns] = value;
    globalThis[ns] = value;
  }

  // The engine's default stdlib loader script-injects `${base}${name}`. Override
  // it to satisfy loads synchronously from the vendored namespaces. Contract
  // (engine `EK_`): return anything !== false after calling ok() to claim
  // handling; return false to fall through to the (here blocked) script path.
  // `name` is the script filename, e.g. "c4.min.js"; the namespace key is the
  // base name, e.g. "c4".
  const loader = (name, ok, fail) => {
    const base = String(name).replace(/\.(min\.)?js$/, '');
    const ns = stdlib[base] || stdlibJson[base];
    if (!ns) return false;
    ok();
    return true;
  };
  window.PLANTUML_STDLIB_LOADER = loader;
  globalThis.PLANTUML_STDLIB_LOADER = loader;

  return { window, networkAttempts, stdlib, stdlibJson, stdlibInfo };
}

// Loads viz-global (classic script, defines Graphviz) into the jsdom window,
// then imports the engine ESM module. Returns the `renderToString` API.
export async function loadEngine(window) {
  window.eval(readFileSync(path.join(coreDir, 'viz-global.js'), 'utf8'));
  const mod = await import(path.join(coreDir, 'plantuml.js'));
  return mod;
}

export function makeRender(renderToString, timeoutMs = 30_000) {
  return (source) => new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('render timeout')), timeoutMs);
    try {
      renderToString(source.split(/\r?\n/), (svg) => {
        clearTimeout(timer);
        resolve(svg);
      }, (err) => {
        clearTimeout(timer);
        reject(new Error(String(err)));
      });
    } catch (err) {
      clearTimeout(timer);
      reject(err);
    }
  });
}
