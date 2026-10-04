// Production build for the PlantUML viewer host.
//
// - Bundles the thin TS glue (`src/index.ts`) with esbuild into a single
//   `assets/plantuml-viewer/bundle.js` plus `bundle.css`.
// - Copies the official `@plantuml/core@<pinned>` engine release artifacts
//   (`plantuml.js`, `viz-global.js`, `themes.js`, `emoji.js`, `openiconic.js`)
//   verbatim as static assets. Per Task 0 §12.1 the engine is NOT bundled into
//   esbuild — it is a large TeaVM artifact that depends on browser globals.
// - Copies vendored stdlib packages (C4) from `stdlib/`.
//
// Determinism: no source maps, no legal comments, no build-machine paths, no
// CDN references. The build refuses to write if a banned token sneaks in.

import { build } from "esbuild";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const assetsDir = path.resolve(here, "../../assets/plantuml-viewer");
const coreDir = path.join(here, "node_modules/@plantuml/core");
const stdlibDir = path.join(here, "stdlib");

/** Engine artifacts copied verbatim (Task 0 §12.1 / resource table). */
const ENGINE_FILES = [
  "plantuml.js",
  "viz-global.js",
  "themes.js",
  "emoji.js",
  "openiconic.js",
];

function sha256(buf) {
  return createHash("sha256").update(buf).digest("hex");
}

async function bundleGlue() {
  const result = await build({
    entryPoints: [path.join(here, "src/index.ts")],
    outdir: path.join(here, ".build"),
    entryNames: "bundle",
    bundle: true,
    format: "esm",
    target: "es2022",
    platform: "browser",
    write: false,
    sourcemap: false,
    legalComments: "none",
    logLevel: "info",
    // The engine is fetched at runtime as a static asset, not bundled.
    external: ["dozer://plantuml-viewer/*"],
    loader: { ".css": "css" },
  });
  const outputs = result.outputFiles;
  const js = outputs.find((o) => o.path.endsWith(".js"));
  const css = outputs.find((o) => o.path.endsWith(".css"));
  if (!js || !css) {
    throw new Error(`expected a .js and a .css output, got ${outputs.map((o) => o.path).join(", ")}`);
  }
  return { js: Buffer.from(js.contents), css: Buffer.from(css.contents) };
}

async function copyEngine() {
  for (const name of ENGINE_FILES) {
    const src = path.join(coreDir, name);
    const dst = path.join(assetsDir, name);
    const buf = await readFile(src).catch(() => null);
    if (!buf) {
      throw new Error(`missing engine artifact ${name} (run npm ci)`);
    }
    await writeFile(dst, buf);
  }
}

async function copyStdlib() {
  const entries = await readdir(stdlibDir).catch(() => []);
  for (const name of entries) {
    if (!name.endsWith(".js")) {
      continue;
    }
    const buf = await readFile(path.join(stdlibDir, name));
    await writeFile(path.join(assetsDir, "stdlib", name), buf);
  }
}

/** Refuse to ship artifacts that would trigger a runtime network request. */
function assertOffline(name, text) {
  const banned = [
    "sourceMappingURL",
    "cdn.jsdelivr.net",
    "unpkg.com",
    "cdnjs.cloudflare.com",
    "registry.npmjs.org",
    here, // build-machine absolute path
  ];
  for (const token of banned) {
    if (text.includes(token)) {
      throw new Error(`refusing to write ${name}: found banned token ${JSON.stringify(token)}`);
    }
  }
}

async function main() {
  await mkdir(path.join(assetsDir, "stdlib"), { recursive: true });
  const { js, css } = await bundleGlue();
  assertOffline("bundle.js", js.toString("utf8"));
  assertOffline("bundle.css", css.toString("utf8"));
  await writeFile(path.join(assetsDir, "bundle.js"), js);
  await writeFile(path.join(assetsDir, "bundle.css"), css);
  await copyEngine();
  await copyStdlib();
  console.log(`[plantuml] bundle.js  sha256 ${sha256(js)}`);
  console.log(`[plantuml] bundle.css sha256 ${sha256(css)}`);
  console.log(`[plantuml] assets -> ${assetsDir}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
