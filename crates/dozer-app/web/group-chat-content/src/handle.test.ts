import test from 'node:test';
import assert from 'node:assert/strict';
import {
  validateHandle, handleKey, suggestHandle, mentionContext, filterMembers, applyMention,
} from './handle.ts';

test('validateHandle accepts normal handles incl. chinese', () => {
  assert.equal(validateHandle('claude', []), null);
  assert.equal(validateHandle('架构师', []), null);
});

test('validateHandle rejects empty, too long, whitespace, @, terminator punctuation', () => {
  assert.match(validateHandle('', [])!, /不能为空/);
  assert.match(validateHandle('x'.repeat(33), [])!, /32/);
  assert.ok(validateHandle('a b', []));
  assert.ok(validateHandle('a@b', []));
  assert.ok(validateHandle('评审，员', []));
  assert.ok(validateHandle('a.b', []));
});

test('validateHandle rejects duplicates ignoring case', () => {
  assert.match(validateHandle('Claude', ['claude'])!, /已有/);
  assert.equal(validateHandle('claude2', ['claude']), null);
});

test('handleKey lowercases', () => {
  assert.equal(handleKey('ClAuDe'), 'claude');
});

test('suggestHandle prefers the agent name, then numbers', () => {
  assert.equal(suggestHandle('claude', []), 'claude');
  assert.equal(suggestHandle('claude', ['claude']), 'claude2');
  assert.equal(suggestHandle('codex', ['codex', 'codex2']), 'codex3');
});

// ---- @ 补全 ----

test('mentionContext finds the @query before the caret', () => {
  assert.deepEqual(mentionContext('你好 @cla', 7), { start: 3, query: 'cla' });
  assert.deepEqual(mentionContext('@', 1), { start: 0, query: '' });
  assert.deepEqual(mentionContext('看下@架构', 5), { start: 2, query: '架构' });
});

test('mentionContext is null when caret is not inside a mention', () => {
  assert.equal(mentionContext('你好 world', 8), null);
  assert.equal(mentionContext('@claude 你好', 10), null); // 光标在空格之后的普通文字里
  assert.equal(mentionContext('foo@bar.com', 7), null);    // 邮箱不算
  assert.deepEqual(mentionContext('@abc', 99), { start: 0, query: 'abc' }); // 越界光标按末尾处理
});

test('mentionContext stops at terminator punctuation', () => {
  assert.equal(mentionContext('@claude，然后', 10), null);
});

test('filterMembers matches by case-insensitive prefix then substring', () => {
  const ms = [{ handle: 'Claude' }, { handle: 'codex' }, { handle: '审阅者' }];
  assert.deepEqual(filterMembers(ms, 'c').map((m) => m.handle), ['Claude', 'codex']);
  assert.deepEqual(filterMembers(ms, 'ode').map((m) => m.handle), ['codex']);
  assert.deepEqual(filterMembers(ms, '').map((m) => m.handle), ['Claude', 'codex', '审阅者']);
  assert.deepEqual(filterMembers(ms, 'zzz'), []);
});

test('applyMention replaces the partial mention and adds a trailing space', () => {
  const ctx = { start: 3, query: 'cla' };
  assert.deepEqual(applyMention('你好 @cla', ctx, 7, 'claude'), { text: '你好 @claude ', caret: 11 });
  // 光标后还有文字时不吞掉后面的内容
  assert.deepEqual(applyMention('你好 @cla 你呢', ctx, 7, 'claude'), { text: '你好 @claude  你呢', caret: 11 });
});
