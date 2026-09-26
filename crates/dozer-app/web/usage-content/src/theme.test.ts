import test from 'node:test';
import assert from 'node:assert/strict';
import { AGENT_COLOR_VAR, SERIES_META } from './theme.ts';

test('AGENT_COLOR_VAR covers all eight AgentKind values', () => {
  const kinds = ['unknown', 'claude', 'codebuddy', 'opencode', 'codex', 'goose', 'aider', 'v8agent'] as const;
  for (const k of kinds) {
    assert.ok(AGENT_COLOR_VAR[k], `missing color for ${k}`);
  }
});

test('AGENT_COLOR_VAR matches workspace/hook.rs::agent_dot_color mapping', () => {
  assert.equal(AGENT_COLOR_VAR.claude, '--cyan');
  assert.equal(AGENT_COLOR_VAR.codebuddy, '--purple');
  assert.equal(AGENT_COLOR_VAR.opencode, '--green');
  assert.equal(AGENT_COLOR_VAR.codex, '--orange');
  assert.equal(AGENT_COLOR_VAR.goose, '--blue');
  assert.equal(AGENT_COLOR_VAR.aider, '--magenta');
  assert.equal(AGENT_COLOR_VAR.v8agent, '--lime');
  assert.equal(AGENT_COLOR_VAR.unknown, '--dim');
});

test('SERIES_META has exactly two series per known TrendChartKind', () => {
  for (const kind of ['session_round', 'io_tokens', 'cache_tokens', 'behavior'] as const) {
    assert.equal(SERIES_META[kind].length, 2, kind);
  }
});

test('SERIES_META labels match chart.rs series constructors', () => {
  assert.deepEqual(
    SERIES_META.session_round.map((s) => s.label),
    ['会话', '回合'],
  );
  assert.deepEqual(
    SERIES_META.io_tokens.map((s) => s.label),
    ['Input', 'Output'],
  );
  assert.deepEqual(
    SERIES_META.cache_tokens.map((s) => s.label),
    ['读', '写'],
  );
  assert.deepEqual(
    SERIES_META.behavior.map((s) => s.label),
    ['触达文件', 'Git提交'],
  );
});
