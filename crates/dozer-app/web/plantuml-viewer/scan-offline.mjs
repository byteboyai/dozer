// Offline asset guard: scans every shipped file under assets/plantuml-viewer/
// for *load-bearing* external references that would fetch from the network at
// runtime. Inert URLs inside strings/comments (theme attribution notes, SVG
// namespace declarations, license links) are intentionally ignored — the goal
// is to catch actual load constructs, not arbitrary text.
//
// A match is an offense only if an absolute http(s) URL appears as part of a
// load construct:
//   - script/link/img element src/href, dynamic import(), fetch(), XHR.open()
//   - new Worker(new URL(...)), sourceMappingURL
//
// sourceMappingURL is always rejected: sourcemaps are never emitted offline.

import { readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const assetsDir = path.resolve(here, "../../assets/plantuml-viewer");

const LOAD_PATTERNS = [
  /sourceMappingURL\s*=/,
  /\bsrc\s*[:=]\s*["'`]https?:\/\//,
  /\bhref\s*[:=]\s*["'`]https?:\/\//,
  /\bimport\s*\(\s*["'`]https?:\/\//,
  /\bfetch\s*\(\s*["'`]https?:\/\//,
  /XMLHttpRequest[\s\S]{0,80}?\.open\s*\([^)]*https?:\/\//,
  /new\s+Worker\s*\(\s*new\s+URL\s*\(\s*["'`]https?:\/\//,
  /\bfrom\s*["'`]https?:\/\//,
];

function walk(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const full = path.join(dir, name);
    const st = statSync(full);
    if (st.isDirectory()) {
      out.push(...walk(full));
    } else {
      out.push(full);
    }
  }
  return out;
}

const offenders = [];
for (const file of walk(assetsDir)) {
  const rel = path.relative(assetsDir, file);
  const text = readFileSync(file, "utf8");
  for (const re of LOAD_PATTERNS) {
    const m = text.match(re);
    if (m) {
      offenders.push(`${rel}: ${m[0].slice(0, 120)}`);
    }
  }
}

if (offenders.length > 0) {
  console.error("offline asset scan failed; load-bearing external references found:");
  for (const o of offenders) {
    console.error(`  ${o}`);
  }
  process.exit(1);
}
console.log(`[plantuml] offline asset scan passed (${walk(assetsDir).length} files)`);
