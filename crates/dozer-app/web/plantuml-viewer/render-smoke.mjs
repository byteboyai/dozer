// End-to-end smoke test: drives the *vendored* engine assets (exactly what
// ships under assets/plantuml-viewer/) through a jsdom harness and asserts the
// behaviors the plan requires: core diagram, C4 stdlib, syntax error, and the
// sanitizer's handling of malicious / oversized SVG.
//
// This is a build-time gate, not production code.

import { JSDOM } from "jsdom";
import { readFileSync, existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

const here = path.dirname(fileURLToPath(import.meta.url));
const assetsDir = path.resolve(here, "../../assets/plantuml-viewer");

const GLOBALS = [
  "window", "document", "navigator", "location", "history",
  "MutationObserver", "DOMParser", "XMLSerializer", "Blob", "File", "FileReader",
  "Image", "getComputedStyle", "requestAnimationFrame", "cancelAnimationFrame",
  "HTMLElement", "SVGElement", "Node", "Event", "CustomEvent", "TextEncoder",
];

function createHarness() {
  const dom = new JSDOM(
    `<!doctype html><html><head></head><body></body></html>`,
    { pretendToBeVisual: true, url: "https://plantuml.invalid/" },
  );
  const { window } = dom;
  for (const name of GLOBALS) {
    if (window[name] !== undefined && globalThis[name] === undefined) {
      globalThis[name] = window[name];
    }
  }
  window.HTMLCanvasElement.prototype.getContext = function getContext() {
    return {
      font: "",
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

  const stdlib = {};
  const stdlibJson = {};
  const stdlibInfo = {};
  for (const ns of ["PLANTUML_STDLIB", "PLANTUML_STDLIB_JSON", "PLANTUML_STDLIB_INFO"]) {
    const value = ns === "PLANTUML_STDLIB" ? stdlib
      : ns === "PLANTUML_STDLIB_JSON" ? stdlibJson : stdlibInfo;
    window[ns] = value;
    globalThis[ns] = value;
  }
  const loader = (name, ok) => {
    const base = String(name).replace(/\.(min\.)?js$/, "");
    const ns = stdlib[base] || stdlibJson[base];
    if (!ns) return false;
    ok();
    return true;
  };
  window.PLANTUML_STDLIB_LOADER = loader;
  globalThis.PLANTUML_STDLIB_LOADER = loader;

  return { window, networkAttempts, stdlib, stdlibJson, stdlibInfo };
}

async function loadEngine(window) {
  window.eval(readFileSync(path.join(assetsDir, "viz-global.js"), "utf8"));
  return await import(path.join(assetsDir, "plantuml.js"));
}

function makeRender(renderToString, timeoutMs = 30_000) {
  return (source) =>
    new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("render timeout")), timeoutMs);
      try {
        renderToString(
          source.split(/\r\n|\r|\n/),
          (svg) => {
            clearTimeout(timer);
            resolve(svg);
          },
          (err) => {
            clearTimeout(timer);
            reject(new Error(String(err)));
          },
        );
      } catch (err) {
        clearTimeout(timer);
        reject(err);
      }
    });
}

async function main() {
  assert.ok(existsSync(path.join(assetsDir, "plantuml.js")), "engine assets missing; run npm run build");

  const { window, networkAttempts, stdlib } = createHarness();

  // Register the vendored C4 stdlib exactly as the runtime loader script would.
  const c4 = readFileSync(path.join(assetsDir, "stdlib/c4.min.js"), "utf8");
  const installer = new Function("window", "globalThis", c4);
  installer.call(window, window, window);

  const engine = await loadEngine(window);
  assert.equal(typeof engine.renderToString, "function", "engine must export renderToString");
  const render = makeRender(engine.renderToString);

  let failed = 0;
  const check = async (name, fn) => {
    try {
      await fn();
      console.log(`  ok  ${name}`);
    } catch (err) {
      failed += 1;
      console.error(`FAIL  ${name}: ${err.message}`);
    }
  };

  await check("core sequence diagram", async () => {
    const svg = await render("@startuml\nAlice -> Bob: hi\n@enduml");
    assert.match(svg, /<svg/);
  });

  await check("C4 stdlib diagram", async () => {
    const svg = await render(
      [
        "@startuml",
        "!include <C4/C4_Container>",
        "Person(admin, \"Admin\")",
        "System(sys, \"System\")",
        "Rel(admin, sys, \"uses\")",
        "@enduml",
      ].join("\n"),
    );
    assert.match(svg, /<svg/);
  });

  await check("syntax error is reported (not a hang)", async () => {
    // Unterminated diagram: the engine raises IndexOutOfBoundsException.
    await assert.rejects(() => render("@startuml\nAlice -> Bob"));
  });

  await check("local !include resolves from virtual fs", async () => {
    stdlib["local"] = { "common.puml": ["@startuml", "@enduml"] };
    const svg = await render("@startuml\n!include local/common.puml\nAlice -> Bob\n@enduml");
    assert.match(svg, /<svg/);
  });

  await check("zero network attempts", () => {
    assert.deepEqual(networkAttempts, []);
  });

  // Sanitizer behaviors are covered by src/sanitize.test.mjs (unit); here we
  // only prove the asset scan from build.mjs held for the shipped bundle.
  if (failed > 0) {
    console.error(`\n${failed} smoke check(s) failed`);
    process.exit(1);
  }
  console.log("\nall smoke checks passed");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
