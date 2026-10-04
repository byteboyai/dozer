// Task 0 spike step 1: baseline. Verify the five core diagram types render
// offline in a DOM environment, rendering is deterministic, and local
// `!include` files resolve through the PLANTUML_STDLIB virtual file system.
//
// Run: node run.mjs

import { readFileSync } from 'node:fs';
import path from 'node:path';
import {
  createHarness, loadEngine, makeRender, fixturesDir,
} from './harness.mjs';

const { window, networkAttempts } = createHarness();
const { renderToString } = await loadEngine(window);
const render = makeRender(renderToString);

async function main() {
  const results = [];
  for (const name of ['sequence', 'class', 'component', 'deployment', 'state']) {
    const src = readFileSync(path.join(fixturesDir, `${name}.puml`), 'utf8');
    const t0 = Date.now();
    const svg = await render(src);
    const ms = Date.now() - t0;
    const ok = svg.includes('<svg');
    results.push({ name, ok, ms, bytes: svg.length });
    console.log(`[${ok ? 'PASS' : 'FAIL'}] ${name}: ${svg.length} bytes in ${ms}ms`);
  }

  // Determinism: same input twice, normalized away the volatile plantuml-src
  // data attribute, must be byte-identical.
  const src = readFileSync(path.join(fixturesDir, 'sequence.puml'), 'utf8');
  const a = await render(src);
  const b = await render(src);
  const norm = (s) => s.replace(/plantuml-src="[^"]*"/g, '');
  console.log(`[${norm(a) === norm(b) ? 'PASS' : 'FAIL'}] determinism (normalized)`);
  console.log(`raw byte-identical: ${a === b}`);

  // Local !include: prove a project file can be injected via the stdlib
  // namespace contract (the production viewer extends this with the actual
  // project directory's files).
  window.PLANTUML_STDLIB['local'] = {
    'included.puml': readFileSync(path.join(fixturesDir, 'included.puml'), 'utf8'),
  };
  try {
    const inc = await render(
      '@startuml\n!include local/included.puml\nAlice -> Bob\n@enduml',
    );
    console.log(`[PASS] stdlib-shaped local include: ${inc.length} bytes`);
  } catch (err) {
    console.log(`[FAIL] stdlib-shaped local include: ${err.message}`);
  }

  // Repeated serial renders: production must serialize renders in one JS
  // context. Measure 20 consecutive renders (cold caches warm after first).
  const seq = readFileSync(path.join(fixturesDir, 'sequence.puml'), 'utf8');
  const times = [];
  for (let i = 0; i < 20; i += 1) {
    const t0 = Date.now();
    await render(seq);
    times.push(Date.now() - t0);
  }
  const sum = times.reduce((a, b) => a + b, 0);
  console.log(
    `[INFO] 20x sequence render: total ${sum}ms, min ${Math.min(...times)}ms, `
    + `max ${Math.max(...times)}ms, avg ${(sum / times.length).toFixed(1)}ms`,
  );
  const rss = process.memoryUsage();
  console.log(
    `[INFO] node RSS ${(rss.rss / 1024 / 1024).toFixed(1)} MiB, `
    + `heapUsed ${(rss.heapUsed / 1024 / 1024).toFixed(1)} MiB`,
  );

  console.log(`\nnetwork attempts: ${networkAttempts.length}`);
  console.log('Summary:', JSON.stringify(results));
}

main().then(() => {
  console.log('spike complete');
  process.exit(0);
}).catch((err) => {
  console.error('SPIKE FAILED:', err);
  process.exit(1);
});
