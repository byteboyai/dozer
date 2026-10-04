// Task 0 spike step 2 (the hard gate): prove the official @plantuml/core JS
// engine renders sequence/class/component/deployment/state AND a C4 diagram
// that pulls in the vendored standard library, with ZERO network access.
//
// Run: node run-c4.mjs

import { readFileSync } from 'node:fs';
import path from 'node:path';
import {
  createHarness, loadEngine, makeRender, here, fixturesDir,
} from './harness.mjs';

const { window, networkAttempts, stdlib } = createHarness();

// Vendor the real C4 stdlib bundle (fetched once at spike time, committed under
// stdlib/). Loading it populates PLANTUML_STDLIB.c4 with 37 files. This is the
// same artifact the production viewer will ship and register via the loader.
window.eval(readFileSync(path.join(here, 'stdlib/c4.min.js'), 'utf8'));
console.log(`stdlib namespaces: ${Object.keys(stdlib).join(', ')}`);
console.log(`c4 files registered: ${Object.keys(stdlib.c4 || {}).length}`);

const { renderToString } = await loadEngine(window);
const render = makeRender(renderToString);

let failed = false;
const results = [];
for (const name of ['sequence', 'class', 'component', 'deployment', 'state', 'c4']) {
  const src = readFileSync(path.join(fixturesDir, `${name}.puml`), 'utf8');
  const t0 = Date.now();
  try {
    const svg = await render(src);
    const ok = svg.includes('<svg');
    if (!ok) failed = true;
    const ms = Date.now() - t0;
    results.push({ name, ok, ms, bytes: svg.length });
    console.log(`[${ok ? 'PASS' : 'FAIL'}] ${name}: ${svg.length} bytes in ${ms}ms`);
  } catch (err) {
    failed = true;
    results.push({ name, ok: false, error: err.message });
    console.log(`[FAIL] ${name}: ${err.message}`);
  }
}

console.log(`network attempts: ${networkAttempts.length}`);
if (networkAttempts.length) {
  console.log(networkAttempts.join('\n'));
  failed = true;
}
console.log('\nSummary:', JSON.stringify(results));
console.log(failed ? 'SPIKE-C4 FAILED' : 'SPIKE-C4 PASSED (fully offline, includes C4 stdlib)');
process.exit(failed ? 1 : 0);
