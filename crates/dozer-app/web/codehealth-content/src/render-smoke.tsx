import test from 'node:test';
import assert from 'node:assert/strict';
import { render } from 'preact-render-to-string';
import { App } from './components/App.tsx';
import { emptyFixture, scanFailedFixture, scanningWithOldResultFixture } from './fixtures.ts';
import {
  overviewFixture,
  overviewFirstScanFixture,
  overviewLegacyFixture,
  structureFixture,
  uiFixture,
  uiNotApplicableFixture,
  scopeFixture,
} from './fixtures.ts';
import {
  archFixture,
  archNotApplicableFixture,
  archPartialFixture,
  archTruncatedFixture,
} from './fixtures.ts';
import type { ViewPayload } from './types.ts';

const html = (p: ViewPayload) => render(<App payload={p} />);

test('empty: never scanned shows message and a scan button', () => {
  const out = html(emptyFixture);
  assert.match(out, /这个项目还没有扫描过。/);
  assert.match(out, /扫描/);
});

test('empty: scan error shows banner', () => {
  const out = html(scanFailedFixture);
  assert.match(out, /扫描失败,请重试:boom/);
});

test('scanning disables the button', () => {
  const out = html(scanningWithOldResultFixture);
  assert.match(out, /disabled/);
  assert.match(out, /扫描中…/);
});

test('overview: tier, summary, change card, priorities, scope line', () => {
  const out = html(overviewFixture);
  assert.match(out, /需要关注/);
  assert.match(out, /核心代码 100 行/);
  assert.match(out, /代码行 \+12/);
  assert.match(out, /函数数 -1/);
  assert.match(out, /新增风险 \+2/);
  assert.match(out, /已解决风险 1/);
  assert.match(out, /交给 Agent 分析/);
  assert.match(out, /本轮新增/);
  assert.match(out, /已分析 3 个文件/);
  assert.match(out, /main · abc123 · 有未提交改动/);
});

test('overview: first scan never fakes zero change', () => {
  const out = html(overviewFirstScanFixture);
  assert.match(out, /首次扫描,暂无历史可比对/);
  assert.doesNotMatch(out, /新增风险/);
  assert.match(out, /暂无优先处理项。/);
});

test('overview: legacy report note is shown', () => {
  assert.match(html(overviewLegacyFixture), /旧版报告,重新扫描可查看变化与范围/);
});

test('structure: default filter is new-only; severity and change use text labels', () => {
  const out = html(structureFixture);
  assert.match(out, /指标口径:控制流信号/);
  assert.match(out, /run 控制流信号 12/);
  assert.doesNotMatch(out, /old 控制流信号 9/, '默认只看本轮新增');
  assert.match(out, /警戒/);
  assert.match(out, /本轮新增/);
  assert.match(out, /src\/a\.rs:7/);
});

test('ui: groups with counts and empty groups say 无发现', () => {
  const out = html(uiFixture);
  assert.match(out, /颜色硬编码:1 处/);
  assert.match(out, /边距硬编码:0 处/);
  assert.match(out, /无发现/);
});

test('ui: not applicable is not rendered as healthy', () => {
  const out = html(uiNotApplicableFixture);
  assert.match(out, /UI 一致性检测适用于 iced\/Rust/);
  assert.doesNotMatch(out, /无发现/);
});

test('scope: status, counts, skipped details', () => {
  const out = html(scopeFixture);
  assert.match(out, /扫描状态:部分完成/);
  assert.match(out, /已分析 3 个文件 · 排除 1 个 · 跳过 1 个/);
  assert.match(out, /Rust\(3 个文件,结构分析\)/);
  assert.match(out, /扫描耗时:420 ms/);
  assert.match(out, /报告版本:schema v3/);
  assert.match(out, /Git 基准:main @ abc123/);
  assert.match(out, /src\/bad\.rs\(解析失败\)/);
});

test('architecture: toolbar, layer switch, risk list with text labels', () => {
  const out = html(archFixture);
  assert.match(out, /架构地图/);
  assert.match(out, /crate/);
  assert.match(out, /module/);
  assert.match(out, /仅看风险/);
  assert.match(out, /适配窗口/);
  assert.match(out, /循环依赖/);
  assert.match(out, /graph-canvas/);
});

// Review Focus 4:没有架构数据时不能显示成零风险。
test('architecture: not applicable shows the note and no canvas', () => {
  const out = html(archNotApplicableFixture);
  assert.match(out, /请重新扫描/);
  assert.doesNotMatch(out, /graph-canvas/);
  assert.doesNotMatch(out, /无风险/);
});

test('architecture: partial shows warning and errors', () => {
  const out = html(archPartialFixture);
  assert.match(out, /不能据此判断没有风险/);
  assert.match(out, /cargo metadata 失败/);
});

test('architecture: truncated shows the truncation note', () => {
  assert.match(html(archTruncatedFixture), /仅显示 crate 层/);
});

import { resetViewState, writeViewState } from './viewState.ts';

// 修复回归:分类切换会卸载页面,视图状态必须从组件外的存储恢复。
test('structure: filter chosen earlier is restored when the page is mounted again', () => {
  resetViewState();
  writeViewState('structure.mode', 'all');
  const out = html(structureFixture);
  assert.match(out, /old 控制流信号 9/, '「全部」筛选应被恢复,持续存在的发现可见');
  resetViewState();
});

test('architecture: layer and risk-only chosen earlier are restored', () => {
  resetViewState();
  writeViewState('arch.layer', 'module');
  writeViewState('arch.riskOnly', true);
  const out = html(archFixture);
  assert.match(out, /class="active"[^>]*>\s*module/);
  assert.match(out, /checked/);
  resetViewState();
});
