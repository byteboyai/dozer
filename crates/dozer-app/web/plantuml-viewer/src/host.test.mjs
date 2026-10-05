import { test } from "node:test";
import assert from "node:assert/strict";
import { build } from "esbuild";
import { JSDOM } from "jsdom";

test("empty document acknowledges completion and applies the requested theme", async () => {
  const result = await build({
    entryPoints: [new URL("./index.ts", import.meta.url).pathname],
    bundle: true, write: false, format: "iife", loader: { ".css": "empty" },
  });
  const dom = new JSDOM('<div id="stage"><div id="content"></div></div><div id="status"></div><div id="diagnostics"></div>', {
    url: "https://preview.invalid/?proj=1&panel=files&tab=2&doc=d",
    runScripts: "outside-only",
  });
  const events = [];
  dom.window.ipc = { postMessage: raw => events.push(JSON.parse(raw)) };
  dom.window.eval(result.outputFiles[0].text);
  dom.window.__dozer.dispatch(JSON.stringify({ payload: {
    kind: "set_document", revision: 7, path: "empty.puml", source: " \n",
    includes: [], theme: "light",
  } }));
  assert.equal(events.at(-1).payload.kind, "rendered");
  assert.equal(events.at(-1).revision, 7);
  assert.equal(events.at(-1).payload.width, 0);
  assert.match(dom.window.document.getElementById("status").textContent, /暂无可渲染内容/);
  assert.ok(dom.window.document.documentElement.classList.contains("theme-light"));
  dom.window.close();
});
