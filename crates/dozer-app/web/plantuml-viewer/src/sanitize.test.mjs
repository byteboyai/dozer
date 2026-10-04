// Unit tests for the SVG sanitizer (spec §6.4). Run under jsdom so DOMParser
// and DOM element APIs exist. `node --test src/*.test.mjs` executes these.

import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>");
globalThis.DOMParser = dom.window.DOMParser;
globalThis.XMLSerializer = dom.window.XMLSerializer;
globalThis.document = dom.window.document;

const { sanitizeSvg, MAX_SVG_BYTES } = await import("./sanitize.ts");

test("strips <script> elements", () => {
  const { svg, removed } = sanitizeSvg(
    `<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script><rect/></svg>`,
  );
  assert.equal(svg.querySelector("script"), null);
  assert.ok(removed >= 1);
});

test("strips on* event handlers", () => {
  const { svg } = sanitizeSvg(
    `<svg xmlns="http://www.w3.org/2000/svg"><rect onclick="alert(1)" width="10"/></svg>`,
  );
  assert.equal(svg.querySelector("rect").getAttribute("onclick"), null);
});

test("strips foreignObject", () => {
  const { svg } = sanitizeSvg(
    `<svg xmlns="http://www.w3.org/2000/svg"><foreignObject><body>x</body></foreignObject></svg>`,
  );
  assert.equal(svg.querySelector("foreignObject"), null);
});

test("removes external hrefs but keeps fragment refs", () => {
  const { svg } = sanitizeSvg(
    `<svg xmlns="http://www.w3.org/2000/svg"><a href="https://evil.example/x"><rect/></a></svg>`,
  );
  assert.equal(svg.querySelector("a").getAttribute("href"), null);
});

test("rejects non-SVG output", () => {
  assert.throws(() => sanitizeSvg(`<html><body>nope</body></html>`));
});

test("rejects oversized SVG", () => {
  const big = `<svg xmlns="http://www.w3.org/2000/svg">${"a".repeat(MAX_SVG_BYTES + 1)}</svg>`;
  assert.throws(() => sanitizeSvg(big), /上限/);
});
