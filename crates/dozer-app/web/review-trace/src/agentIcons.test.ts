import test from 'node:test';
import assert from 'node:assert/strict';
import { ICONS, AGENT_STYLE } from './agentIcons.ts';

test('AGENT_STYLE covers all eight known agent labels with non-empty icon markup', () => {
  const labels = ['claude', 'codebuddy', 'opencode', 'codex', 'aider', 'goose', 'v8agent', 'shell'] as const;
  for (const label of labels) {
    const style = AGENT_STYLE[label];
    assert.ok(style, `missing AGENT_STYLE for ${label}`);
    assert.ok(style.icon.startsWith('<svg'), `${label} icon must be an <svg> string`);
  }
});

test('claude/codebuddy/opencode use a theme color; codex/aider/goose keep their own brand colors (color: null)', () => {
  assert.equal(AGENT_STYLE.claude.color, '#47DEF0');
  assert.equal(AGENT_STYLE.codebuddy.color, '#9580FF');
  assert.equal(AGENT_STYLE.opencode.color, '#1AD585');
  assert.equal(AGENT_STYLE.codex.color, null);
  assert.equal(AGENT_STYLE.aider.color, null);
  assert.equal(AGENT_STYLE.goose.color, null);
});

test('v8agent and shell fall back to the generic bot icon', () => {
  assert.equal(AGENT_STYLE.v8agent.icon, ICONS.bot);
  assert.equal(AGENT_STYLE.shell.icon, ICONS.bot);
});

test('ICONS.user is a distinct icon from ICONS.bot', () => {
  assert.notEqual(ICONS.user, ICONS.bot);
  assert.ok(ICONS.user.startsWith('<svg'));
});
