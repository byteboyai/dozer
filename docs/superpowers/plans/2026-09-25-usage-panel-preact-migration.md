# 用量面板内容侧转 Preact WebView Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把用量面板内容侧(`extensions/usage/view.rs::content_pane` 现有五态里的四态——空/该 agent 无数据/单 agent 趋势/全部 agent 汇总,"统计中…"留原生——+ `chart.rs` 997 行手绘 `iced_widget::canvas`)换成 Preact + esbuild 离线打包的长驻单槽 webview,Rust 主动推送数据(不整页重载),右侧 agent 筛选栏继续原生 iced 不动。

**Architecture:** 新建 `crates/dozer-app/web/usage-content/`(Preact + esbuild,照抄 `web/review-trace` 构建模式)。数据流是"推送"而非 review-trace 的"fetch 快照":`content_pane` 现有四态(不含"统计中…")判定逻辑搬进纯函数 `extensions/usage/protocol.rs::current_view_payload`,产出 `UsageViewPayload` 并 `Serialize`;Rust 侧仿 Git Log diff pane 先例(`app.rs` `preview_desired` 的 `PanelKind::GitLog` 分支、`take_git_log_diff_script`、`git_log::State::pending_diff_push`)——固定单槽 webview(新 `USAGE_CONTENT_ID_OFFSET`),App 级 `usage::WebviewPushState`(`ready`/`last_sent` 两个纯状态位)判定"要不要推、推什么",每帧 `evaluate_script` 注入 `window.__dozer.dispatch(JSON)`。前端只吃 `UsageViewPayload` 渲染,不做任何聚合计算。

**Tech Stack:** Preact 10.29.8、esbuild 0.28.2(jsx automatic runtime)、TypeScript 5.9.3、Node 内置 `node:test`(同 `web/review-trace`/`web/editor` 现有跑法)。

**Spec:** `docs/superpowers/specs/2026-09-25-usage-panel-preact-migration-design.md`

## Global Constraints

- 范围:只转内容侧四态(空/该 agent 无数据/单 agent 趋势/全部 agent 汇总,`chart.rs` 全部图表随之进 webview)。"统计中…"(`math_curve` 动画)、右侧 agent 筛选栏(`view.rs::list_pane`/`agent_filter_sidebar`)、面板头(图标+"用量"标题+收起按钮,`content_pane` 里 `head` 那部分)继续原生 iced,不进 webview。
- 不改 `aggregate.rs` 任何聚合计算逻辑与已有单测断言的业务规则(1% 份额剔除、V8agent 覆盖、7/15 天窗口等)——`current_view_payload` 只是把这些函数的**输出**序列化,不重算。
- 长驻单槽 + 推送指令,不整页重载:webview 只创建一次,数据变化(刷新完成/切筛选)靠 `evaluate_script` 注入更新,不重新 `navigate`。
- 不复用 `preview::webview_protocol::EditorCommand`/`EditorEvent`/`EditorHostBinding`(CodeMirror 专属、绑定多 tab/document 模型)——Usage 是固定单槽面板,新写一套更轻的协议,但复用同一条 `window.__dozer.dispatch(JSON)` 注入约定与 `dispatch_script` 工具函数(该函数已是"任意 JSON 字符串 → `evaluate_script` 脚本",与payload 类型无关)。
- CSP:`default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'`(无 `connect-src`——没有 fetch 端点,数据全靠推送)。
- 离线打包:无 CDN、无运行时 Node,esbuild `format:'iife'`、`sourcemap:false`、`legalComments:'none'`,同 `web/review-trace`/`web/editor`。
- 主题:webview 要跟随当前明暗主题(`dozer://usage-content/host.html?theme=` 同 `flyfish_url`/`html_url` 的 `scheme_query_value()` 约定),不是只支持暗色——现有 `chart.rs`/`view.rs` 全部走 `byteui::theme::color::current()`,是主题响应式的,不能在迁移中退化成恒暗色。
- 视觉与交互逐像素对齐:汇总卡片、环图/饼图配色与图例、逐日柱状图(含斑马纹条带)、趋势折线(含悬浮竖线)、hover tooltip 内容与触发方式,全部 1:1 迁移,不新增不删减。

## Review Focus

- **多 agent 筛选切换的"最新覆盖旧"语义:** agent 筛选栏可能被快速连续点击,`WebviewPushState` 若不能正确判定"当前该推什么"就可能让 webview 短暂显示上一个 agent 的数据——Task 14 的 `pending_push` 单测要覆盖"desired 变了两次、中间那次从未真正 evaluate_script 过"的场景。
- **切换项目/workspace 后 webview 显示的是旧项目数据:** webview 是 App 级单槽(不按项目分,同 Git Log 先例),`last_sent` 若只在"同一 workspace 内容变化"时失效,切到另一个仍显示 Usage 面板的项目 tab 不会触发重推——Task 14 的测试要覆盖"desired payload 来自不同 workspace、与上次 sent 不同"必须判定为待推送。
- **agent 份额饼图角度累加误差:** `PieChart::draw`(chart.rs:821-860)用浮点累加 `angle`,多个 agent 相邻切片累计误差可能让最后一片跟第一片之间出现可见缝隙——Task 7 的 `pieSlices.ts` 测试要覆盖 3+ agent 且份额不能整除 360° 的情形,断言全部切片角度之和等于 `2π - n*PIE_GAP_RAD`(gap 之间的缝隙是设计的,累积误差不是)。
- **`daily_totals_by_agent` 的 `totals` 跨天 agent 顺序不一致会让柱子颜色错位:** `DailyUsageChart.agents` 是从**第一天**的 `totals` 顺序取的(Task 2 `current_view_payload` 实现里),若某次改动让不同天的 agent 顺序不一致,颜色会跟错 agent——Task 2 补一条测试锁定"所有天 agent 顺序一致"这个 `aggregate.rs` 已有的不变量。
- **超小份额/超小数值下的刻度与角度退化:** `nice_tick_step`/`grid_ticks` 在 `max_total` 极小(如 1)或为 0 时的兜底(Task 4 已用 Rust 原有单测断言逐条迁移),以及饼图 `total == 0` 时不渲染任何切片——Task 7 要显式测这个边界,不能只测"正常有数据"的路径。

---

## 文件结构总览

```text
crates/dozer-app/web/usage-content/          # 新增
├── package.json
├── tsconfig.json
├── build.mjs
└── src/
    ├── types.ts                             # UsageViewPayload 及全部子类型,与 Rust protocol.rs 逐字段对应
    ├── format.ts / format.test.ts           # formatCount/niceTickStep/gridTicks(移植自 chart.rs)
    ├── theme.ts / theme.test.ts             # AGENT_COLORS(dark/light)、SERIES_META
    ├── pieSlices.ts / pieSlices.test.ts     # 饼图切片角度纯函数
    ├── host.html                            # 严格 CSP,主题感知(data-theme 属性)
    ├── styles.css
    ├── main.tsx                             # window.__dozer.dispatch 监听 + ipc.postMessage("ready") + 主题解析
    └── components/
        ├── App.tsx                          # 按 payload.kind 分发五态
        ├── EmptyStates.tsx                  # empty/agent_empty 两态文案(统计中…留在原生 iced,不进 webview)
        ├── ProjectSummaryBoxes.tsx / StatBox.tsx
        ├── PieChart.tsx / ChartStatList.tsx / AgentMetricsSection.tsx
        ├── GridLines.tsx                    # 柱状图/趋势图共用的 SVG 网格线+刻度
        ├── DailyUsageChart.tsx              # 逐日分组柱状图+斑马纹+tooltip
        └── TrendLineChart.tsx / TrendSection.tsx  # 折线图+悬浮竖线+tooltip,SessionRound/IoTokens/CacheTokens/Behavior 四种共用

crates/dozer-app/assets/usage-content/       # 新增,构建产物,提交到仓库
├── host.html
├── usage-content.js
└── usage-content.css

crates/dozer-app/src/extensions/usage/protocol.rs   # 新增:UsageViewPayload 及子类型、current_view_payload、UsageWebviewEvent/parse_usage_event
crates/dozer-app/src/extensions/usage/mod.rs        # 修改:挂载 protocol 子模块;新增 WebviewPushState
crates/dozer-app/src/extensions/usage/view.rs       # 修改:content_pane 删掉四态 body 渲染(保留"统计中…"原生分支),chart.rs 全部调用点一起删
crates/dozer-app/src/extensions/usage/chart.rs      # 删除(有效纯函数 has_any_value/trend_total 迁到 protocol.rs)
crates/dozer-app/src/assets.rs                      # 修改:usage-content 路由(无 data.json)
crates/dozer-app/src/theme/geometry.rs              # 修改:usage_content_chrome_top_px
crates/dozer-app/src/webview_geometry.rs            # 修改:usage_content_pane_bounds_for
crates/dozer-app/src/app/app.rs                     # 修改:USAGE_CONTENT_ID_OFFSET、App::usage_webview 字段、preview_desired 分支、take_usage_content_script
crates/dozer-app/src/app/message.rs                 # 修改:Message::UsageContentWebviewEvent
crates/dozer-app/src/app/update.rs                  # 修改:处理该消息
crates/dozer-app/src/platform/window_events.rs      # 修改:apply_pending_editor_commands 消费新指令队列
crates/dozer-app/src/runtime.rs                     # 修改:ipc_handler 识别 usage webview id、解析事件
```

---

### Task 1: 前端脚手架(package.json / build.mjs / host.html / styles.css / types.ts)

**Files:**
- Create: `crates/dozer-app/web/usage-content/package.json`
- Create: `crates/dozer-app/web/usage-content/tsconfig.json`
- Create: `crates/dozer-app/web/usage-content/build.mjs`
- Create: `crates/dozer-app/web/usage-content/src/types.ts`
- Create: `crates/dozer-app/web/usage-content/src/host.html`
- Create: `crates/dozer-app/web/usage-content/src/styles.css`
- Create: `crates/dozer-app/web/usage-content/src/main.tsx`(占位版,Task 11 换成真实实现)

**Interfaces:**
- Produces: `types.ts` 导出 `AgentKind`/`ProjectSummary`/`TrendChartKind`/`TrendDay`/`TrendChart`/`TokenTrendSection`/`AgentShare`/`AgentMetrics`/`DailyUsageDay`/`DailyUsageChart`/`UsageViewPayload`,供全部后续前端任务使用。字段名/形状与 Task 2 的 Rust `protocol.rs` 逐一对应(`#[serde(tag="kind", rename_all="snake_case")]` 外部标记枚举)。
- Produces: 构建脚本产出 `crates/dozer-app/assets/usage-content/{host.html,usage-content.js,usage-content.css}`。

- [ ] **Step 1: 创建 package.json**

```json
{
  "name": "dozer-usage-content",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "description": "Dozer 用量面板内容侧(usage-content),Preact 离线打包,无 CDN/无运行时 Node。",
  "scripts": {
    "build": "node build.mjs",
    "typecheck": "tsc --noEmit",
    "test": "node --test src/*.test.ts"
  },
  "dependencies": {
    "preact": "10.29.8"
  },
  "devDependencies": {
    "@types/node": "^24",
    "esbuild": "0.28.2",
    "typescript": "5.9.3"
  }
}
```

- [ ] **Step 2: 创建 tsconfig.json**

```json
{
  "compilerOptions": {
    "target": "ES2020",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noEmit": true,
    "skipLibCheck": true,
    "allowImportingTsExtensions": true,
    "jsx": "react-jsx",
    "jsxImportSource": "preact",
    "types": ["node"],
    "lib": ["ES2020", "DOM", "DOM.Iterable"]
  },
  "include": ["src/**/*.ts", "src/**/*.tsx"]
}
```

- [ ] **Step 3: 创建 build.mjs**

```js
// 生产构建:把用量面板内容侧打包成离线、无 CDN、无运行时 Node 的确定性
// 产物到 `crates/dozer-app/assets/usage-content/`。不输出 source map;
// minify 后去掉所有 legal comments。

import { build } from 'esbuild';
import { cp, mkdir, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const outdir = path.resolve(here, '../../assets/usage-content');

await rm(outdir, { recursive: true, force: true });
await mkdir(outdir, { recursive: true });

await build({
  entryPoints: [path.join(here, 'src/main.tsx')],
  bundle: true,
  format: 'iife',
  platform: 'browser',
  target: ['es2020'],
  minify: true,
  sourcemap: false,
  legalComments: 'none',
  charset: 'utf8',
  jsx: 'automatic',
  jsxImportSource: 'preact',
  loader: { '.css': 'css' },
  outfile: path.join(outdir, 'usage-content.js'),
  logLevel: 'info',
});

await cp(path.join(here, 'src/host.html'), path.join(outdir, 'host.html'));

console.log('built ->', outdir);
```

- [ ] **Step 4: 创建 src/types.ts**

```ts
export type AgentKind =
  | 'unknown'
  | 'claude'
  | 'codebuddy'
  | 'opencode'
  | 'codex'
  | 'goose'
  | 'aider'
  | 'v8agent';

export interface ProjectSummary {
  conversation_count: number;
  turns: number;
  tool_calls: number;
  files_touched: number;
  tokens_in: number;
  tokens_out: number;
  tokens_cache_read: number;
  tokens_cache_write: number;
}

export type TrendChartKind = 'session_round' | 'io_tokens' | 'cache_tokens' | 'behavior';

export interface TrendDay {
  label: string;
  values: number[];
}

export interface TrendChart {
  kind: TrendChartKind;
  days: TrendDay[];
}

export interface TokenTrendSection {
  io: TrendChart | null;
  io_total: number;
  cache: TrendChart | null;
  cache_total: number;
}

export interface AgentShare {
  agent: AgentKind;
  value: number;
}

export interface AgentMetrics {
  session_share: AgentShare[];
  turn_share: AgentShare[];
  io_share: AgentShare[];
  cache_share: AgentShare[];
  total_tokens: number;
}

export interface DailyUsageDay {
  label: string;
  /** 下标与外层 `DailyUsageChart.agents` 一一对应。 */
  totals: number[];
}

export interface DailyUsageChart {
  agents: AgentKind[];
  days: DailyUsageDay[];
}

// 注意:没有 `loading` 态——"统计中…"用的是 `byteui::feedback::math_curve`
// 动画组件(ByteBoy2077 品牌化的自绘曲线,多个面板共用),不值得单独在
// webview 里重做一套等价动画。加载中时 Rust 侧根本不挂载/推送这个
// webview,原生 iced 继续显示那个动画(同 Git Log diff pane"不可渲染
// 时回落原生占位"的先例,见 Task 13)。
export type UsageViewPayload =
  | { kind: 'empty' }
  | { kind: 'agent_empty'; agent: AgentKind }
  | {
      kind: 'single_agent';
      agent: AgentKind;
      project: ProjectSummary;
      git_commits: number;
      session_trend: TrendChart | null;
      token_trend: TokenTrendSection | null;
    }
  | {
      kind: 'all_agents';
      project: ProjectSummary;
      git_commits: number;
      agent_metrics: AgentMetrics | null;
      daily_usage: DailyUsageChart | null;
      daily_behavior: TrendChart | null;
    };
```

- [ ] **Step 5: 创建 src/styles.css**(明暗双主题变量,基色取自 `crates/byteui/src/theme/color.rs` dark/light 两套面板;结构参照 `web/review-trace/src/styles.css` 但主题可切换)

```css
:root[data-theme="dark"] {
  --bg: #0d131c;
  --panel: #0a0e16;
  --card: #12202a;
  --border: #1c3440;
  --cream: #FFE5B4;
  --dim: #6B7F8F;
  --gold: #F2D94E;
  --cyan: #47DEF0;
  --green: #1AD585;
  --purple: #9580FF;
  --lime: #A3E635;
  --orange: #FF9B4D;
  --blue: #4D8CFF;
  --magenta: #FF6EC7;
}
:root[data-theme="light"] {
  --bg: #fffdf6;
  --panel: #fef2e4;
  --card: #fefdfb;
  --border: #d7dfe5;
  --cream: #16232e;
  --dim: #4c5c68;
  --gold: #118b96;
  --cyan: #0e8a9e;
  --green: #128f5a;
  --purple: #6a4fdb;
  --lime: #6fa813;
  --orange: #c97a1b;
  --blue: #2f6fe0;
  --magenta: #c93b93;
}
* { box-sizing: border-box; }
html, body {
  margin: 0; height: 100%;
  background: var(--panel); color: var(--cream);
  font: 13px/1.5 -apple-system, "PingFang SC", sans-serif;
}
#root { height: 100%; overflow-y: auto; padding: 14px; }

.usage-empty { color: var(--dim); font-size: 13px; }

.usage-section { margin-bottom: 24px; }
.usage-section:last-child { margin-bottom: 0; }
.usage-section-head {
  display: flex; align-items: center; gap: 6px;
  font-size: 13px; color: var(--cream); margin-bottom: 24px;
}
.usage-section-head::before {
  content: ""; width: 6px; height: 6px; border-radius: 50%; background: var(--cream);
}
.usage-subsection-gap { margin-top: 32px; }

.usage-stat-row { display: flex; gap: 16px; width: 100%; margin-bottom: 24px; }
.usage-stat-box {
  flex: 1; display: flex; gap: 24px; padding: 12px;
  background: var(--card); border: 1px solid var(--border); border-radius: 10px;
}
.usage-stat { display: flex; flex-direction: column; gap: 2px; }
.usage-stat-label { font-size: 11px; color: var(--dim); }
.usage-stat-value { font-size: 15px; }

.usage-metric-banner { display: flex; align-items: center; gap: 8px; margin-bottom: 12px; }
.usage-metric-banner .label { font-size: 13px; color: var(--cream); }
.usage-metric-banner .total { font-size: 12px; color: var(--cream); }

.usage-pair { display: flex; gap: 16px; width: 100%; margin-bottom: 24px; }
.usage-pair > * { flex: 1; }
.usage-metric-group { display: flex; gap: 16px; align-items: center; width: 100%; }

.usage-legend-title { font-size: 12px; color: var(--dim); margin-bottom: 8px; }
.usage-legend-row { display: flex; align-items: center; gap: 6px; margin-bottom: 6px; }
.usage-legend-dot { width: 8px; height: 8px; border-radius: 50%; flex: none; }
.usage-legend-text { font-size: 11px; color: var(--dim); }

.usage-inline-legend { display: flex; gap: 12px; }
.usage-inline-legend .usage-legend-row { margin-bottom: 0; }

.usage-chart-wrap { position: relative; width: 100%; }
.usage-tooltip {
  position: absolute; z-index: 1; padding: 6px 8px; border-radius: 6px;
  background: var(--card); border: 1px solid var(--border);
  font-size: 11px; white-space: nowrap; pointer-events: none;
  transform: translate(-50%, -100%); margin-top: -6px;
}
.usage-tooltip .day-label { font-size: 11px; color: var(--cream); margin-bottom: 4px; }
```

- [ ] **Step 6: 创建 src/host.html**(严格 CSP,主题由 URL `?theme=` 参数注入,见 Task 11 main.tsx)

```html
<!doctype html>
<html lang="zh">
<head>
<meta charset="utf-8" />
<meta
  http-equiv="Content-Security-Policy"
  content="default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'"
/>
<title>usage</title>
<link rel="stylesheet" href="usage-content.css" />
</head>
<body>
<div id="root"></div>
<script src="usage-content.js"></script>
</body>
</html>
```

- [ ] **Step 7: 创建占位版 src/main.tsx**(证明构建管线打通;Task 11 换成真实实现)

```tsx
import { render } from 'preact';
import './styles.css';

document.documentElement.dataset.theme = 'dark';
render(<div>加载中…</div>, document.getElementById('root')!);
```

- [ ] **Step 8: 安装依赖并构建,验证产物**

```bash
cd crates/dozer-app/web/usage-content
npm install
npm run build
```

Expected: `esbuild` 输出 `usage-content.js` 体积日志,无报错;随后检查产物:

```bash
ls -la ../../assets/usage-content
grep -c "default-src 'none'" ../../assets/usage-content/host.html
```

Expected: `host.html`、`usage-content.js`、`usage-content.css` 三个文件都存在且非空;`grep` 命中 1。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/web/usage-content crates/dozer-app/assets/usage-content
git commit -m "feat(usage-content): 脚手架 Preact + esbuild 离线打包管线"
```

---

### Task 2: Rust protocol.rs(UsageViewPayload 及子类型、current_view_payload 纯函数)

**Files:**
- Create: `crates/dozer-app/src/extensions/usage/protocol.rs`
- Modify: `crates/dozer-app/src/extensions/usage/mod.rs`

**Interfaces:**
- Consumes: `WorkspaceState`(本文件 `mod.rs`,已有)、`aggregate.rs` 全部聚合函数(经 `mod.rs` 的 `pub(crate) use aggregate::*` 可见)、`chart.rs` 的 `has_any_value`/`trend_total` 两个纯函数(Task 17 删除 `chart.rs` 时会把它们搬进本文件,当前先直接引用,不重复实现)。
- Produces: `UsageViewPayload`、`ProjectSummary`、`TrendChartKind`、`TrendDay`、`TrendChart`、`TokenTrendSection`、`AgentShare`、`AgentMetrics`、`DailyUsageDay`、`DailyUsageChart`(全部 `pub(crate)`,`#[derive(Serialize)]`)、`current_view_payload(ws_state: &WorkspaceState) -> UsageViewPayload`、`UsageWebviewEvent`/`parse_usage_event`、`UsagePushEnvelope`/`encode_usage_push`,供 Task 14/15 使用。

- [ ] **Step 1: 写失败的测试**(先写测试,再写实现——`current_view_payload` 是纯函数,可以完全脱离 webview/iced 单测)

```rust
// 追加到 crates/dozer-app/src/extensions/usage/protocol.rs 底部
#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::ConversationMeta;
    use std::path::PathBuf;

    fn meta_at(agent: AgentKind, ms: u64) -> ConversationMeta {
        ConversationMeta {
            path: PathBuf::from(format!("/{ms}.jsonl")),
            title: String::new(),
            modified_ms: ms,
            agent,
        }
    }

    fn usage(turns: u32) -> ConversationUsage {
        ConversationUsage {
            turns,
            ..Default::default()
        }
    }

    #[test]
    fn empty_rows_yields_empty_payload() {
        let ws = WorkspaceState::default();
        assert_eq!(current_view_payload(&ws), UsageViewPayload::Empty);
    }

    #[test]
    fn agent_filter_with_no_matching_rows_yields_agent_empty() {
        let mut ws = WorkspaceState::default();
        update(
            &mut ws,
            Message::Loaded(1, vec![(meta_at(AgentKind::Claude, 0), usage(1))], 0, Default::default()),
        );
        update(&mut ws, Message::AgentFilterSet(Some(AgentKind::Codebuddy)));
        assert_eq!(
            current_view_payload(&ws),
            UsageViewPayload::AgentEmpty {
                agent: AgentKind::Codebuddy
            }
        );
    }

    #[test]
    fn single_agent_state_carries_project_summary_and_git_commits() {
        let mut ws = WorkspaceState::default();
        update(
            &mut ws,
            Message::Loaded(
                1,
                vec![(meta_at(AgentKind::Claude, 0), usage(3))],
                7,
                Default::default(),
            ),
        );
        update(&mut ws, Message::AgentFilterSet(Some(AgentKind::Claude)));
        match current_view_payload(&ws) {
            UsageViewPayload::SingleAgent {
                agent,
                project,
                git_commits,
                ..
            } => {
                assert_eq!(agent, AgentKind::Claude);
                assert_eq!(project.turns, 3);
                assert_eq!(git_commits, 7);
            }
            other => panic!("expected SingleAgent, got {other:?}"),
        }
    }

    #[test]
    fn all_agents_state_is_default_when_no_filter_set() {
        let mut ws = WorkspaceState::default();
        update(
            &mut ws,
            Message::Loaded(1, vec![(meta_at(AgentKind::Claude, 0), usage(3))], 0, Default::default()),
        );
        assert!(matches!(
            current_view_payload(&ws),
            UsageViewPayload::AllAgents { .. }
        ));
    }

    /// `daily_totals_by_agent` 每天 `totals` 的 agent 顺序必须一致(否则不同
    /// 天的柱子颜色会错位)——`aggregate.rs` 的实现已保证这一点(`project_agents`
    /// 只算一次、逐天 zip 复用同一份顺序),这里锁定 `current_view_payload`
    /// 把 `agents` 只从第一天取出、不逐天重新推导这个简化是安全的。
    #[test]
    fn daily_usage_agents_order_matches_every_day() {
        let mut ws = WorkspaceState::default();
        update(
            &mut ws,
            Message::Loaded(
                1,
                vec![
                    (meta_at(AgentKind::Codebuddy, 0), usage(1)),
                    (meta_at(AgentKind::Claude, 0), usage(1)),
                    (meta_at(AgentKind::Claude, 86_400_000), usage(1)),
                ],
                0,
                Default::default(),
            ),
        );
        match current_view_payload(&ws) {
            UsageViewPayload::AllAgents {
                daily_usage: Some(chart),
                ..
            } => {
                assert_eq!(chart.days.len(), 2);
                for d in &chart.days {
                    assert_eq!(d.totals.len(), chart.agents.len());
                }
            }
            other => panic!("expected AllAgents with daily_usage, got {other:?}"),
        }
    }

    #[test]
    fn single_agent_token_trend_none_when_no_io_or_cache_activity() {
        let mut ws = WorkspaceState::default();
        update(
            &mut ws,
            Message::Loaded(1, vec![(meta_at(AgentKind::Claude, 0), usage(1))], 0, Default::default()),
        );
        update(&mut ws, Message::AgentFilterSet(Some(AgentKind::Claude)));
        match current_view_payload(&ws) {
            UsageViewPayload::SingleAgent { token_trend, .. } => assert!(token_trend.is_none()),
            other => panic!("expected SingleAgent, got {other:?}"),
        }
    }

    #[test]
    fn encode_usage_push_embeds_protocol_version_and_payload() {
        let json = encode_usage_push(UsageViewPayload::Empty);
        assert!(json.contains(r#""protocol_version":1"#));
        assert!(json.contains(r#""kind":"empty""#));
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozer-app usage::protocol
```

Expected: FAIL,`current_view_payload`/`UsageViewPayload` 等尚未定义。

- [ ] **Step 3: 创建 src/extensions/usage/protocol.rs 实现**

```rust
//! Usage 内容侧 webview 推送协议:`content_pane`(view.rs)现有五态判定
//! 逻辑的纯函数版本,产出要序列化推给 webview 的 `UsageViewPayload`。前端
//! 只管照着渲染,不做任何聚合计算——聚合逻辑本身仍在 `aggregate.rs`,一处
//! 不动。

use super::*;
use dozer_core::protocol::AgentKind;
use serde::{Deserialize, Serialize};

/// 没有 `Loading` 变体——"统计中…"是 `byteui::feedback::math_curve` 动画
/// 组件,加载中时这个 webview 根本不挂载(见 `webview_geometry.rs::
/// usage_content_pane_bounds_for` 的 `content_desired` 参数、Task 13),
/// 原生 iced 继续画那个动画,不进这份序列化契约。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UsageViewPayload {
    Empty,
    AgentEmpty {
        agent: AgentKind,
    },
    SingleAgent {
        agent: AgentKind,
        project: ProjectSummary,
        git_commits: u64,
        session_trend: Option<TrendChart>,
        token_trend: Option<TokenTrendSection>,
    },
    AllAgents {
        project: ProjectSummary,
        git_commits: u64,
        agent_metrics: Option<AgentMetrics>,
        daily_usage: Option<DailyUsageChart>,
        daily_behavior: Option<TrendChart>,
    },
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ProjectSummary {
    pub conversation_count: u32,
    pub turns: u32,
    pub tool_calls: u32,
    pub files_touched: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cache_read: u64,
    pub tokens_cache_write: u64,
}

impl From<&ProjectUsageTotals> for ProjectSummary {
    fn from(t: &ProjectUsageTotals) -> Self {
        Self {
            conversation_count: t.conversation_count,
            turns: t.turns,
            tool_calls: t.tool_calls,
            files_touched: t.files_touched,
            tokens_in: t.tokens_in,
            tokens_out: t.tokens_out,
            tokens_cache_read: t.tokens_cache_read,
            tokens_cache_write: t.tokens_cache_write,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TrendChartKind {
    SessionRound,
    IoTokens,
    CacheTokens,
    Behavior,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TrendDay {
    pub label: String,
    pub values: Vec<u64>,
}

impl From<&DaySeries> for TrendDay {
    fn from(d: &DaySeries) -> Self {
        Self {
            label: d.label.clone(),
            values: d.values.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TrendChart {
    pub kind: TrendChartKind,
    pub days: Vec<TrendDay>,
}

fn trend_chart(kind: TrendChartKind, days: &[DaySeries]) -> Option<TrendChart> {
    has_any_value(days).then(|| TrendChart {
        kind,
        days: days.iter().map(TrendDay::from).collect(),
    })
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TokenTrendSection {
    pub io: Option<TrendChart>,
    pub io_total: u64,
    pub cache: Option<TrendChart>,
    pub cache_total: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AgentShare {
    pub agent: AgentKind,
    pub value: u64,
}

fn shares(v: &[(AgentKind, u64)]) -> Vec<AgentShare> {
    v.iter()
        .map(|&(agent, value)| AgentShare { agent, value })
        .collect()
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AgentMetrics {
    pub session_share: Vec<AgentShare>,
    pub turn_share: Vec<AgentShare>,
    pub io_share: Vec<AgentShare>,
    pub cache_share: Vec<AgentShare>,
    /// "Tokens" 横幅用的四项 token 总和,直接对 `filtered_rows` 求和
    /// (口径同 `view.rs:141-149` 现状的 `total_tokens`),**不是**
    /// `io_share`/`cache_share` 两个已剔除 <1% agent 份额口径的相加——
    /// 两者在"存在被 1% 阈值剔除的 agent"时会有微小差异,横幅要用更大的
    /// 那个"全项目未筛选"口径,所以单独算一份,不让前端从 share 猜。
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DailyUsageDay {
    pub label: String,
    /// 下标与 `DailyUsageChart.agents` 一一对应。
    pub totals: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DailyUsageChart {
    pub agents: Vec<AgentKind>,
    pub days: Vec<DailyUsageDay>,
}

/// `view.rs::content_pane` 剩余四态(不含"统计中…",见上面 `UsageViewPayload`
/// 文档)判定逻辑的纯函数版本,不碰 iced,直接产出要推给 webview 的
/// payload。**调用方必须先确认 `!ws_state.loading()`**(`take_usage_content_
/// script`——Task 15——只在 `content_desired` 为真时才调用这个函数);本函数
/// 自身不重复判定 loading,避免"两处都写一遍判据、以后改漏一处"。
/// `content_pane` 改造后不再自己重算这些分支,这里是唯一权威实现。
pub fn current_view_payload(ws_state: &WorkspaceState) -> UsageViewPayload {
    let rows = ws_state.rows();
    if rows.is_empty() {
        return UsageViewPayload::Empty;
    }
    let filtered_rows = filter_rows_by_agent(rows, ws_state.agent_filter);
    if filtered_rows.is_empty() {
        // `filter_rows_by_agent(rows, None)` 原样返回 `rows`(非空),所以
        // 走到这个分支时 `agent_filter` 必然是 `Some`。
        let agent = ws_state
            .agent_filter
            .expect("filtered_rows 为空且 rows 非空时 agent_filter 必为 Some");
        return UsageViewPayload::AgentEmpty { agent };
    }
    let usages: Vec<ConversationUsage> = filtered_rows.iter().map(|(_, u)| u.clone()).collect();
    let project = ProjectSummary::from(&aggregate(&usages));

    match ws_state.agent_filter {
        Some(agent) => {
            let today = today_day_index();
            let session_trend = trend_chart(
                TrendChartKind::SessionRound,
                &session_round_trend(rows, agent, today),
            );
            let token_trend = single_agent_token_trend(agent, rows, today);
            UsageViewPayload::SingleAgent {
                agent,
                project,
                git_commits: ws_state.git_commits(),
                session_trend,
                token_trend,
            }
        }
        None => {
            let session_share = agent_session_share(&filtered_rows);
            let turn_share = agent_turn_share(&filtered_rows);
            let io_share = agent_io_token_share(&filtered_rows);
            let cache_share = agent_cache_token_share(&filtered_rows);
            let agent_metrics = (!session_share.is_empty()
                || !turn_share.is_empty()
                || !io_share.is_empty()
                || !cache_share.is_empty())
            .then(|| AgentMetrics {
                session_share: shares(&session_share),
                turn_share: shares(&turn_share),
                io_share: shares(&io_share),
                cache_share: shares(&cache_share),
                total_tokens: filtered_rows
                    .iter()
                    .map(|(_, u)| {
                        u.tokens_in + u.tokens_out + u.tokens_cache_read + u.tokens_cache_write
                    })
                    .sum(),
            });

            let days = daily_totals_by_agent(&filtered_rows);
            let daily_usage = (!days.is_empty()).then(|| {
                let agents: Vec<AgentKind> = days
                    .first()
                    .map(|d| d.totals.iter().map(|(a, _)| *a).collect())
                    .unwrap_or_default();
                DailyUsageChart {
                    agents,
                    days: days
                        .iter()
                        .map(|d| DailyUsageDay {
                            label: d.label.clone(),
                            totals: d.totals.iter().map(|(_, v)| *v).collect(),
                        })
                        .collect(),
                }
            });

            let behavior = behavior_series(
                &filtered_rows,
                ws_state.git_commits_by_day(),
                today_day_index(),
            );
            let daily_behavior = trend_chart(TrendChartKind::Behavior, &behavior);

            UsageViewPayload::AllAgents {
                project,
                git_commits: ws_state.git_commits(),
                agent_metrics,
                daily_usage,
                daily_behavior,
            }
        }
    }
}

fn single_agent_token_trend(
    agent: AgentKind,
    rows: &[(crate::conversation::ConversationMeta, ConversationUsage)],
    today_index: i64,
) -> Option<TokenTrendSection> {
    let io = io_trend(rows, agent, today_index);
    let cache = cache_trend(rows, agent, today_index);
    if !has_any_value(&io) && !has_any_value(&cache) {
        return None;
    }
    Some(TokenTrendSection {
        io: trend_chart(TrendChartKind::IoTokens, &io),
        io_total: trend_total(&io),
        cache: trend_chart(TrendChartKind::CacheTokens, &cache),
        cache_total: trend_total(&cache),
    })
}

/// webview 发回的事件——只有 `Ready` 一种(JS 端 `window.__dozer.dispatch`
/// 已注册)。不复用 `preview::webview_protocol::EditorEvent`(那个枚举带
/// `DocumentLoaded`/`SelectionChanged` 等 CodeMirror 专属变体);Usage 是
/// 固定单槽、只读渲染,不需要那一整套多 tab/document 协议。
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UsageWebviewEvent {
    Ready,
}

pub fn parse_usage_event(body: &str) -> Result<UsageWebviewEvent, String> {
    serde_json::from_str(body).map_err(|e| e.to_string())
}

/// Rust → webview 推送 envelope。刻意比 `preview::webview_protocol::
/// WebviewEnvelope` 精简(没有 `tab_id`/`document_id`/`revision`/
/// `request_id`)——固定单槽面板,没有多 tab/document 身份需要携带。
/// `preview::webview_protocol::dispatch_script` 只吃"任意 JSON 字符串",
/// 与这里的 payload 类型无关,可以直接复用来包成 `evaluate_script` 脚本
/// (见 `App::take_usage_content_script`,Task 15)。
#[derive(Debug, Clone, Serialize)]
pub struct UsagePushEnvelope {
    pub protocol_version: u32,
    pub payload: UsageViewPayload,
}

pub const USAGE_PROTOCOL_VERSION: u32 = 1;

pub fn encode_usage_push(payload: UsageViewPayload) -> String {
    serde_json::to_string(&UsagePushEnvelope {
        protocol_version: USAGE_PROTOCOL_VERSION,
        payload,
    })
    .unwrap_or_else(|_| "null".to_string())
}
```

- [ ] **Step 4: 在 mod.rs 挂载新模块**

```rust
// crates/dozer-app/src/extensions/usage/mod.rs 顶部 `mod` 声明处新增
mod protocol;
pub(crate) use protocol::*;
```

- [ ] **Step 5: 运行测试确认通过**

```bash
cargo test -p dozer-app usage::protocol
```

Expected: 6 个测试全部 PASS。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/usage/protocol.rs crates/dozer-app/src/extensions/usage/mod.rs
git commit -m "feat(usage): 新增内容侧推送协议 UsageViewPayload + current_view_payload"
```

---

### Task 3: assets.rs 路由(usage-content 静态资源服务,无 data.json)

**Files:**
- Modify: `crates/dozer-app/src/assets.rs`

**Interfaces:**
- Consumes: Task 1 产出的 `crates/dozer-app/assets/usage-content/{host.html,usage-content.js,usage-content.css}`。
- Produces: `dozer://usage-content/*` 路由,供 Task 13(webview 创建)使用。

- [ ] **Step 1: 写失败的测试**

```rust
// 追加到 crates/dozer-app/src/assets.rs 的 `#[cfg(test)] mod tests` 块内
/// 提交的 usage-content 产物必须齐全(防止忘记 `npm run build` 就提交)。
#[test]
fn usage_content_bundle_assets_are_present() {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/usage-content"));
    for f in ["host.html", "usage-content.js", "usage-content.css"] {
        let p = root.join(f);
        assert!(p.is_file(), "缺少 usage-content 产物 {f}: {}", p.display());
        assert!(
            std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 0,
            "usage-content 产物为空: {f}"
        );
    }
}

/// 严格 CSP、无 connect-src(没有 fetch 端点,数据全靠推送)、无网络。
#[test]
fn usage_content_host_has_strict_csp_and_no_external_refs() {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/usage-content"));
    let html = std::fs::read_to_string(root.join("host.html")).expect("读 host.html");
    assert!(html.contains("default-src 'none'"));
    assert!(html.contains("script-src 'self'"));
    assert!(
        !html.contains("connect-src"),
        "usage-content 没有 fetch 端点,不应声明 connect-src"
    );
    assert!(!html.contains("http://") && !html.contains("https://"));
    assert!(html.contains("usage-content.js") && html.contains("usage-content.css"));
}

#[test]
fn usage_content_serves_vendored_files() {
    let root = scratch();
    std::fs::create_dir_all(root.with_file_name("usage-content")).unwrap();
    std::fs::write(root.with_file_name("usage-content").join("host.html"), b"<html>u</html>").unwrap();
    let r = handle_protocol(&root, &HashSet::new(), None, "dozer://usage-content/host.html");
    assert_eq!((r.status, r.mime), (200, "text/html"));
    assert_eq!(r.body, b"<html>u</html>");
}

#[test]
fn usage_content_unknown_subpath_404() {
    let root = scratch();
    std::fs::create_dir_all(root.with_file_name("usage-content")).unwrap();
    let r = handle_protocol(&root, &HashSet::new(), None, "dozer://usage-content/nope");
    assert_eq!(r.status, 404);
}
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozer-app assets::tests::usage_content
```

Expected: FAIL(前两个测试此时应已通过,因为 Task 1 已产出真实文件;后两个 FAIL,因为路由尚未接上,404 而不是命中)。

- [ ] **Step 3: 实现路由**

```rust
// crates/dozer-app/src/assets.rs,紧挨着 review_trace_root_for 添加
/// usage-content host(用量面板内容侧图表)静态资源根 = flyfish 根的兄弟
/// 目录 `usage-content`。同 `editor_root_for`/`review_trace_root_for`。
fn usage_content_root_for(flyfish_root: &Path) -> PathBuf {
    flyfish_root.with_file_name("usage-content")
}
```

```rust
// handle_protocol 内,review-trace 分支之后新增(顺序不影响正确性,放在
// review-trace 分支旁边便于对照——两者都是"仿 review-trace 磁盘服务"）:
// 没有 data.json 特判:数据全靠 evaluate_script 推送,不走 fetch。
if let Some(path) = rest.strip_prefix("usage-content/") {
    return serve_vendored(&usage_content_root_for(assets_root), path);
}
```

同时把 `handle_protocol` 顶部注释"只服务 flyfish/review-trace/editor 三个命名空间"更新为包含 `usage-content`。

- [ ] **Step 4: 运行测试确认通过**

```bash
cargo test -p dozer-app assets::tests
```

Expected: 全部 PASS(含既有 review-trace/editor/json-editor 测试,确认没有破坏)。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/assets.rs
git commit -m "feat(usage-content): assets.rs 接入 dozer://usage-content 路由"
```

---

### Task 4: format.ts(数字格式化 + 刻度算法,移植自 chart.rs)

**Files:**
- Create: `crates/dozer-app/web/usage-content/src/format.ts`
- Test: `crates/dozer-app/web/usage-content/src/format.test.ts`

**Interfaces:**
- Produces: `formatCount(n: number): string`、`niceTickStep(maxValue: number, targetTicks: number): number`、`gridTicks(maxTotal: number): number[]`,供 Task 6/7/8/9/10 全部图表组件使用。

> `niceTickStep`/`gridTicks` 的测试用例直接照抄 `chart.rs` 已有的 `nice_tick_step_rounds_to_1_2_5_family`/`grid_ticks_stops_at_max_and_never_empty` 单测断言(`usage/mod.rs:215-231`)——这两组数字在 Rust 侧已经过验证,原样迁过来保证行为一致,不是随手挑的用例。

- [ ] **Step 1: 写失败的测试**

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { formatCount, niceTickStep, gridTicks } from './format.ts';

test('formatCount below 1000 returns the plain integer', () => {
  assert.equal(formatCount(0), '0');
  assert.equal(formatCount(999), '999');
});

test('formatCount uses k-suffix with one decimal in the [1000, 1e6) range', () => {
  assert.equal(formatCount(1000), '1.0k');
  assert.equal(formatCount(1234), '1.2k');
  assert.equal(formatCount(123_456), '123.5k');
  assert.equal(formatCount(999_999), '1000.0k');
});

test('formatCount uses m-suffix with one decimal at/above 1e6', () => {
  assert.equal(formatCount(1_000_000), '1.0m');
  assert.equal(formatCount(1_234_567), '1.2m');
});

// 与 chart.rs::tests::nice_tick_step_rounds_to_1_2_5_family 逐条对应。
test('niceTickStep rounds the raw step to the 1/2/5 family', () => {
  assert.equal(niceTickStep(7, 4), 2);
  assert.equal(niceTickStep(42, 4), 20);
  assert.equal(niceTickStep(1, 4), 1);
});

// 与 chart.rs::tests::grid_ticks_stops_at_max_and_never_empty 逐条对应。
test('gridTicks stops at max and never returns empty for a positive total', () => {
  assert.deepEqual(gridTicks(0), []);
  assert.deepEqual(gridTicks(7), [2, 4, 6]);
  assert.deepEqual(gridTicks(1), [1]);
});
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cd crates/dozer-app/web/usage-content
node --test src/format.test.ts
```

Expected: FAIL,`Cannot find module './format.ts'`。

- [ ] **Step 3: 创建 src/format.ts**(逐行对照 `chart.rs:67-103`/`chart.rs:772-781` 迁移)

```ts
// 用量面板数字的统一样式,移植自 chart.rs::format_count:
// - < 1000:原样。
// - [1000, 1e6):除以 1000,1 位小数,'k' 后缀(1000 直接进 1.0k,不出现
//   1000.0k 这种中间档)。
// - >= 1e6:除以 1e6,1 位小数,'m' 后缀。
export function formatCount(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'm';
  if (n >= 1000) return (n / 1000).toFixed(1) + 'k';
  return String(n);
}

// "nice numbers" 刻度步长算法,移植自 chart.rs::nice_tick_step:按数量级
// 取 1/2/5/10 里最接近 raw_step 的一档,让刻度总落在整数上(不是简单
// max/target 等分,那样步长会是 733 这种没法一眼读的零头)。
export function niceTickStep(maxValue: number, targetTicks: number): number {
  const rawStep = maxValue / Math.max(targetTicks, 1);
  if (rawStep <= 0) return 1;
  const magnitude = Math.pow(10, Math.floor(Math.log10(rawStep)));
  const residual = rawStep / magnitude;
  let niceResidual: number;
  if (residual <= 1) niceResidual = 1;
  else if (residual <= 2) niceResidual = 2;
  else if (residual <= 5) niceResidual = 5;
  else niceResidual = 10;
  return Math.max(Math.round(niceResidual * magnitude), 1);
}

const GRID_TARGET_TICKS = 4;

// 从一个步长的整数倍往上数,数到 maxTotal 为止的刻度值(不含 0 基线),
// 移植自 chart.rs::grid_ticks。
export function gridTicks(maxTotal: number): number[] {
  if (maxTotal === 0) return [];
  const step = niceTickStep(maxTotal, GRID_TARGET_TICKS);
  const ticks: number[] = [];
  for (let v = step; v <= maxTotal; v += step) ticks.push(v);
  if (ticks.length === 0) ticks.push(maxTotal);
  return ticks;
}
```

- [ ] **Step 4: 运行测试确认通过**

```bash
node --test src/format.test.ts
```

Expected: 8 个测试全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/web/usage-content/src/format.ts crates/dozer-app/web/usage-content/src/format.test.ts
git commit -m "feat(usage-content): 移植数字格式化与刻度算法"
```

---

### Task 5: theme.ts(agent 配色 + 趋势序列配色表)

**Files:**
- Create: `crates/dozer-app/web/usage-content/src/theme.ts`
- Test: `crates/dozer-app/web/usage-content/src/theme.test.ts`

**Interfaces:**
- Consumes: `AgentKind`、`TrendChartKind` from `./types.ts`(Task 1)。
- Produces: `agentColor(agent: AgentKind): string`(读当前 `document.documentElement.dataset.theme`)、`SERIES_META: Record<TrendChartKind, { label: string; colorVar: string }[]>`,供 Task 7(饼图/图例)、Task 9(柱状图)、Task 10(趋势图)使用。

> 颜色表与 `workspace/hook.rs::agent_dot_color`(dark/light 两套十六进制取自 `crates/byteui/src/theme/color.rs`)、`chart.rs::session_trend_series`/`io_trend_series`/`cache_trend_series` 与 `view.rs` 的"触达文件/Git提交"配色逐一对应。用 CSS 变量名(`--cyan` 等,已在 Task 1 `styles.css` 定义)而不是直接写死十六进制,明暗切换靠 CSS 自动生效,不需要 JS 侧重新计算。

- [ ] **Step 1: 写失败的测试**

```ts
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
```

- [ ] **Step 2: 运行测试确认失败**

```bash
node --test src/theme.test.ts
```

Expected: FAIL,`Cannot find module './theme.ts'`。

- [ ] **Step 3: 创建 src/theme.ts**

```ts
import type { AgentKind, TrendChartKind } from './types.ts';

// 移植自 workspace/hook.rs::agent_dot_color——每个 agent 固定映射到一个
// 主题 CSS 变量名(见 Task 1 styles.css 的 :root[data-theme] 定义),不是
// 写死的十六进制,明暗切换时自动跟随。
export const AGENT_COLOR_VAR: Record<AgentKind, string> = {
  claude: '--cyan',
  codebuddy: '--purple',
  opencode: '--green',
  codex: '--orange',
  goose: '--blue',
  aider: '--magenta',
  v8agent: '--lime',
  unknown: '--dim',
};

export function agentColorVar(agent: AgentKind): string {
  return `var(${AGENT_COLOR_VAR[agent]})`;
}

export interface SeriesMeta {
  label: string;
  colorVar: string;
}

// 移植自 chart.rs::session_trend_series/io_trend_series/cache_trend_series
// 与 view.rs 里"触达文件/Git提交"的配色(c.cream/c.green)。下标与
// Rust 侧 `TrendDay.values[i]` 一一对应,不能重排。
export const SERIES_META: Record<TrendChartKind, SeriesMeta[]> = {
  session_round: [
    { label: '会话', colorVar: '--cream' },
    { label: '回合', colorVar: '--green' },
  ],
  io_tokens: [
    { label: 'Input', colorVar: '--cyan' },
    { label: 'Output', colorVar: '--purple' },
  ],
  cache_tokens: [
    { label: '读', colorVar: '--lime' },
    { label: '写', colorVar: '--green' },
  ],
  behavior: [
    { label: '触达文件', colorVar: '--cream' },
    { label: 'Git提交', colorVar: '--green' },
  ],
};
```

- [ ] **Step 4: 运行测试确认通过**

```bash
node --test src/theme.test.ts
```

Expected: 4 个测试全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/web/usage-content/src/theme.ts crates/dozer-app/web/usage-content/src/theme.test.ts
git commit -m "feat(usage-content): 迁移 agent 配色与趋势序列配色表"
```

---

### Task 6: ProjectSummaryBoxes.tsx / StatBox.tsx(项目汇总卡片)

**Files:**
- Create: `crates/dozer-app/web/usage-content/src/components/StatBox.tsx`
- Create: `crates/dozer-app/web/usage-content/src/components/ProjectSummaryBoxes.tsx`

**Interfaces:**
- Consumes: `ProjectSummary` from `../types.ts`(Task 1)、`formatCount` from `../format.ts`(Task 4)。
- Produces: `StatBox({ stats }: { stats: { label: string; value: string; colorVar: string }[] })`、`ProjectSummaryBoxes({ project, gitCommits }: { project: ProjectSummary; gitCommits: number })`,供 Task 11(`App.tsx`)使用。

> 纯结构组件,没有可脱离浏览器断言的逻辑分支(字段↔文案映射由 Task 11 人工视觉核对确认),同 review-trace 计划 Task 5 对这类组件的验证口径——`npm run typecheck` 保正确性,不额外写单测。

- [ ] **Step 1: 创建 src/components/StatBox.tsx**(对应 `view.rs::stat`/`stat_box`)

```tsx
interface Stat {
  label: string;
  value: string;
  colorVar: string;
}

export function StatBox({ stats }: { stats: Stat[] }) {
  return (
    <div class="usage-stat-box">
      {stats.map((s) => (
        <div class="usage-stat" key={s.label}>
          <span class="usage-stat-label">{s.label}</span>
          <span class="usage-stat-value" style={{ color: `var(${s.colorVar})` }}>
            {s.value}
          </span>
        </div>
      ))}
    </div>
  );
}
```

- [ ] **Step 2: 创建 src/components/ProjectSummaryBoxes.tsx**(对应 `view.rs::project_summary_boxes`)

```tsx
import type { ProjectSummary } from '../types.ts';
import { formatCount } from '../format.ts';
import { StatBox } from './StatBox.tsx';

export function ProjectSummaryBoxes({
  project,
  gitCommits,
}: {
  project: ProjectSummary;
  gitCommits: number;
}) {
  return (
    <div class="usage-stat-row">
      <StatBox
        stats={[
          { label: '会话', value: formatCount(project.conversation_count), colorVar: '--cream' },
          { label: '回合', value: formatCount(project.turns), colorVar: '--cream' },
          { label: '工具调用', value: formatCount(project.tool_calls), colorVar: '--cream' },
          { label: '触达文件', value: formatCount(project.files_touched), colorVar: '--cream' },
          { label: 'Git提交', value: formatCount(gitCommits), colorVar: '--cream' },
        ]}
      />
      <StatBox
        stats={[
          { label: 'Input', value: formatCount(project.tokens_in), colorVar: '--cyan' },
          { label: 'Output', value: formatCount(project.tokens_out), colorVar: '--cyan' },
          { label: 'cache 读', value: formatCount(project.tokens_cache_read), colorVar: '--cyan' },
          { label: 'cache 写', value: formatCount(project.tokens_cache_write), colorVar: '--cyan' },
        ]}
      />
    </div>
  );
}
```

- [ ] **Step 3: 类型检查**

```bash
npm run typecheck
```

Expected: 无输出、退出码 0。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/web/usage-content/src/components/StatBox.tsx crates/dozer-app/web/usage-content/src/components/ProjectSummaryBoxes.tsx
git commit -m "feat(usage-content): 迁移项目汇总卡片组件"
```

---

### Task 7: 饼图数学 + PieChart / ChartStatList / AgentMetricsSection

**Files:**
- Create: `crates/dozer-app/web/usage-content/src/pieSlices.ts`
- Test: `crates/dozer-app/web/usage-content/src/pieSlices.test.ts`
- Create: `crates/dozer-app/web/usage-content/src/components/PieChart.tsx`
- Create: `crates/dozer-app/web/usage-content/src/components/ChartStatList.tsx`
- Create: `crates/dozer-app/web/usage-content/src/components/AgentMetricsSection.tsx`

**Interfaces:**
- Consumes: `AgentShare`/`AgentMetrics` from `../types.ts`(Task 1)、`agentColorVar` from `../theme.ts`(Task 5)、`formatCount` from `../format.ts`(Task 4)。
- Produces: `pieSlices(share: AgentShare[]): PieSlice[]`、`PieChart`、`ChartStatList`、`AgentMetricsSection({ metrics }: { metrics: AgentMetrics })`,供 Task 11 使用。

> `pieSlices` 是这个任务里唯一有真实数学复杂度的部分,逐行对照 `chart.rs::PieChart::draw`(chart.rs:821-860)迁移:12 点钟方向起(`-PI/2`)、顺时针累加,每片扣掉 `PIE_GAP_RAD` 的一半留缝。渲染(角度 → SVG `<path>` 弧线)是纯几何转换,不在这次风险清单里。

- [ ] **Step 1: 写失败的测试**

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { pieSlices, PIE_GAP_RAD } from './pieSlices.ts';

test('empty share list produces no slices', () => {
  assert.deepEqual(pieSlices([]), []);
});

test('all-zero share (total 0) produces no slices', () => {
  assert.deepEqual(pieSlices([{ agent: 'claude', value: 0 }]), []);
});

test('a single 100% slice starts at -PI/2 and sweeps nearly a full circle minus one gap', () => {
  const [s] = pieSlices([{ agent: 'claude', value: 10 }]);
  const near = (a: number, b: number) => Math.abs(a - b) < 1e-9;
  assert.ok(near(s.startAngle, -Math.PI / 2 + PIE_GAP_RAD / 2));
  assert.ok(near(s.endAngle, -Math.PI / 2 + 2 * Math.PI - PIE_GAP_RAD / 2));
});

// 对应本计划 Review Focus:多切片累积误差不应产生可见缝隙(gap 之外的
// 累加误差)——全部切片角宽之和必须精确等于 2π - n*PIE_GAP_RAD。
test('slice angular widths sum to 2*PI minus one gap per slice, regardless of share count', () => {
  const share = [
    { agent: 'claude' as const, value: 7 },
    { agent: 'codebuddy' as const, value: 13 },
    { agent: 'opencode' as const, value: 5 },
    { agent: 'v8agent' as const, value: 41 },
  ];
  const slices = pieSlices(share);
  const totalWidth = slices.reduce((sum, s) => sum + (s.endAngle - s.startAngle), 0);
  const near = (a: number, b: number) => Math.abs(a - b) < 1e-9;
  assert.ok(near(totalWidth, 2 * Math.PI - slices.length * PIE_GAP_RAD));
});

test('slices appear in the same order as the input share list', () => {
  const share = [
    { agent: 'claude' as const, value: 1 },
    { agent: 'codebuddy' as const, value: 1 },
  ];
  const slices = pieSlices(share);
  assert.deepEqual(
    slices.map((s) => s.agent),
    ['claude', 'codebuddy'],
  );
});
```

- [ ] **Step 2: 运行测试确认失败**

```bash
node --test src/pieSlices.test.ts
```

Expected: FAIL,`Cannot find module './pieSlices.ts'`。

- [ ] **Step 3: 创建 src/pieSlices.ts**(逐行对照 `chart.rs:817-860` 迁移)

```ts
import type { AgentKind, AgentShare } from './types.ts';

export const PIE_GAP_RAD = 0.035;

export interface PieSlice {
  agent: AgentKind;
  startAngle: number;
  endAngle: number;
}

// 12 点钟方向起(-PI/2),顺时针(角度递增)累加每片的角度,每片两端各扣
// 半个 PIE_GAP_RAD 留缝——逐行对照 chart.rs::PieChart::draw。
export function pieSlices(share: AgentShare[]): PieSlice[] {
  const total = share.reduce((sum, s) => sum + s.value, 0);
  if (total === 0) return [];
  let angle = -Math.PI / 2;
  const slices: PieSlice[] = [];
  for (const { agent, value } of share) {
    const sweep = (2 * Math.PI * value) / total;
    const startAngle = angle + PIE_GAP_RAD / 2;
    const endAngle = angle + sweep - PIE_GAP_RAD / 2;
    slices.push({ agent, startAngle, endAngle });
    angle += sweep;
  }
  return slices;
}

// 角度 → 圆上一点(屏幕坐标系,y 向下,角度递增=顺时针,与 chart.rs 的
// iced Radians 约定一致)。
export function pointOnCircle(cx: number, cy: number, r: number, angle: number): [number, number] {
  return [cx + r * Math.cos(angle), cy + r * Math.sin(angle)];
}

// 单个扇形切片的 SVG path `d` 属性。
export function pieSlicePath(cx: number, cy: number, r: number, slice: PieSlice): string {
  const [x1, y1] = pointOnCircle(cx, cy, r, slice.startAngle);
  const [x2, y2] = pointOnCircle(cx, cy, r, slice.endAngle);
  const largeArc = slice.endAngle - slice.startAngle > Math.PI ? 1 : 0;
  return `M ${cx} ${cy} L ${x1} ${y1} A ${r} ${r} 0 ${largeArc} 1 ${x2} ${y2} Z`;
}
```

- [ ] **Step 4: 运行测试确认通过**

```bash
node --test src/pieSlices.test.ts
```

Expected: 5 个测试全部 PASS。

- [ ] **Step 5: 创建 src/components/PieChart.tsx**(对应 `chart.rs::pie_chart`,`PIE_RADIUS=52.0`)

```tsx
import type { AgentShare } from '../types.ts';
import { pieSlices, pieSlicePath } from '../pieSlices.ts';
import { agentColorVar } from '../theme.ts';

const PIE_RADIUS = 52;

export function PieChart({ share }: { share: AgentShare[] }) {
  const size = PIE_RADIUS * 2 + 8;
  const c = size / 2;
  const slices = pieSlices(share);
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} class="usage-pie">
      {slices.map((s) => (
        <path key={s.agent} d={pieSlicePath(c, c, PIE_RADIUS, s)} fill={agentColorVar(s.agent)} />
      ))}
    </svg>
  );
}
```

- [ ] **Step 6: 创建 src/components/ChartStatList.tsx**(对应 `chart.rs::chart_stat_list`)

```tsx
import type { AgentShare } from '../types.ts';
import { formatCount } from '../format.ts';
import { agentColorVar } from '../theme.ts';

// agent 展示名——同 `AgentKind::label()`(dozer_core::protocol,
// crates/dozer-core/src/protocol.rs),与 Rust 侧逐一对应。
const AGENT_LABEL: Record<string, string> = {
  claude: 'claude',
  codebuddy: 'codebuddy',
  opencode: 'opencode',
  codex: 'codex',
  goose: 'goose',
  aider: 'aider',
  v8agent: 'v8agent',
  unknown: 'unknown',
};

export function ChartStatList({ title, share }: { title: string; share: AgentShare[] }) {
  const total = share.reduce((sum, s) => sum + s.value, 0);
  return (
    <div>
      <div class="usage-legend-title">
        {title}({formatCount(total)})
      </div>
      {share.map((s) => {
        const pct = total > 0 ? Math.floor((s.value * 100) / total) : 0;
        return (
          <div class="usage-legend-row" key={s.agent}>
            <span class="usage-legend-dot" style={{ background: agentColorVar(s.agent) }} />
            <span class="usage-legend-text">
              {AGENT_LABEL[s.agent]} - {formatCount(s.value)}({pct}%)
            </span>
          </div>
        );
      })}
    </div>
  );
}
```

- [ ] **Step 7: 创建 src/components/AgentMetricsSection.tsx**(对应 `view.rs` "Agent 用量统计"分支 + `chart.rs::agent_metric_group`/`pair_metric_cells`/`metric_group_banner`)

```tsx
import type { AgentMetrics, AgentShare } from '../types.ts';
import { formatCount } from '../format.ts';
import { PieChart } from './PieChart.tsx';
import { ChartStatList } from './ChartStatList.tsx';

function shareTotal(share: AgentShare[]): number {
  return share.reduce((sum, s) => sum + s.value, 0);
}

function MetricBanner({ label, total }: { label: string; total: number }) {
  return (
    <div class="usage-metric-banner">
      <span class="label">{label}</span>
      <span class="total">{formatCount(total)} total</span>
    </div>
  );
}

function MetricGroup({ title, share }: { title: string; share: AgentShare[] }) {
  return (
    <div class="usage-metric-group">
      <PieChart share={share} />
      <ChartStatList title={title} share={share} />
    </div>
  );
}

// 某一侧没有数据时只放有数据那一节、让它吃满整行——对应
// chart.rs::pair_metric_cells 的 (Some,None)/(None,Some) 分支。
function PairCells({
  left,
  right,
}: {
  left: [string, AgentShare[]] | null;
  right: [string, AgentShare[]] | null;
}) {
  if (left && right) {
    return (
      <div class="usage-pair">
        <MetricGroup title={left[0]} share={left[1]} />
        <MetricGroup title={right[0]} share={right[1]} />
      </div>
    );
  }
  const only = left ?? right;
  if (!only) return null;
  return (
    <div class="usage-pair">
      <MetricGroup title={only[0]} share={only[1]} />
    </div>
  );
}

export function AgentMetricsSection({ metrics }: { metrics: AgentMetrics }) {
  const sessTotal = shareTotal(metrics.session_share);
  const hasSessionGroup = metrics.session_share.length > 0 || metrics.turn_share.length > 0;
  const hasTokenGroup = metrics.io_share.length > 0 || metrics.cache_share.length > 0;
  return (
    <>
      {hasSessionGroup && (
        <>
          <MetricBanner label="Session" total={sessTotal} />
          <PairCells
            left={metrics.session_share.length > 0 ? ['Session', metrics.session_share] : null}
            right={metrics.turn_share.length > 0 ? ['Round', metrics.turn_share] : null}
          />
        </>
      )}
      {hasTokenGroup && (
        <>
          <MetricBanner label="Tokens" total={metrics.total_tokens} />
          <PairCells
            left={metrics.io_share.length > 0 ? ['Input/Output', metrics.io_share] : null}
            right={metrics.cache_share.length > 0 ? ['Cache Read/Write', metrics.cache_share] : null}
          />
        </>
      )}
    </>
  );
}
```

- [ ] **Step 8: 类型检查**

```bash
npm run typecheck
```

Expected: 无输出、退出码 0。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/web/usage-content/src/pieSlices.ts crates/dozer-app/web/usage-content/src/pieSlices.test.ts crates/dozer-app/web/usage-content/src/components/PieChart.tsx crates/dozer-app/web/usage-content/src/components/ChartStatList.tsx crates/dozer-app/web/usage-content/src/components/AgentMetricsSection.tsx
git commit -m "feat(usage-content): 迁移饼图数学与 Agent 用量统计组件"
```

---

### Task 8: GridLines.tsx(柱状图/趋势图共用的网格线+刻度)

**Files:**
- Create: `crates/dozer-app/web/usage-content/src/components/GridLines.tsx`
- Modify: `crates/dozer-app/web/usage-content/src/styles.css`

**Interfaces:**
- Consumes: `gridTicks`/`formatCount` from `../format.ts`(Task 4)。
- Produces: `GridLines({ maxTotal }: { maxTotal: number })`、导出常量 `BAR_MAX_HEIGHT`/`BAR_LABEL_GAP`/`GRID_CANVAS_HEIGHT`/`GRID_LABEL_GUTTER`/`BAR_WIDTH`/`DAY_BAND_HEIGHT`(移植自 `chart.rs:14-25`),供 Task 9(`DailyUsageChart`)、Task 10(`TrendLineChart`)共用同一套版式常量与 y 轴换算,不能各自重复定义出现偏差。

- [ ] **Step 1: 创建 src/components/GridLines.tsx**(对应 `chart.rs::GridLines::draw`/`grid_lines_canvas`,常量原样迁自 `chart.rs:14-31`)

```tsx
import { gridTicks, formatCount } from '../format.ts';

export const BAR_MAX_HEIGHT = 72;
export const BAR_WIDTH = 14;
export const BAR_LABEL_GAP = 14;
export const GRID_CANVAS_HEIGHT = BAR_MAX_HEIGHT + BAR_LABEL_GAP; // 86
export const DAY_BAND_HEIGHT = GRID_CANVAS_HEIGHT + 4 + 12; // 102
export const GRID_LABEL_GUTTER = 26;

// value → 网格画布内的 y 坐标(0 基线落在画布最底部),同 chart.rs 的
// GridLines::draw / TrendLines::draw 共用换算。
export function yFor(value: number, maxTotal: number): number {
  if (maxTotal === 0) return GRID_CANVAS_HEIGHT;
  return GRID_CANVAS_HEIGHT - (value / maxTotal) * BAR_MAX_HEIGHT;
}

export function GridLines({ maxTotal }: { maxTotal: number }) {
  if (maxTotal === 0) return null;
  return (
    <>
      {gridTicks(maxTotal).map((tick) => (
        <div class="usage-grid-line" style={{ top: `${yFor(tick, maxTotal)}px` }} key={tick}>
          <span class="usage-grid-label">{formatCount(tick)}</span>
        </div>
      ))}
    </>
  );
}
```

- [ ] **Step 2: 追加 styles.css 规则**(网格线绝对定位,左侧留 `GRID_LABEL_GUTTER` 给刻度数字,不与柱顶数字重叠——同 `chart.rs` 注释里的理由)

```css
/* 追加到 crates/dozer-app/web/usage-content/src/styles.css 末尾 */
.usage-chart-wrap { position: relative; height: 86px; }
.usage-grid-line {
  position: absolute; left: 26px; right: 0; height: 0;
  border-top: 1px solid var(--border);
}
.usage-grid-label {
  position: absolute; right: calc(100% + 4px); top: -6px;
  font-size: 7px; color: var(--dim); white-space: nowrap;
}
```

- [ ] **Step 3: 类型检查**

```bash
npm run typecheck
```

Expected: 无输出、退出码 0。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/web/usage-content/src/components/GridLines.tsx crates/dozer-app/web/usage-content/src/styles.css
git commit -m "feat(usage-content): 迁移共用网格线组件"
```

---

### Task 9: DailyUsageChart.tsx(逐日分组柱状图 + 斑马纹 + tooltip)

**Files:**
- Create: `crates/dozer-app/web/usage-content/src/components/DailyUsageChart.tsx`
- Modify: `crates/dozer-app/web/usage-content/src/styles.css`

**Interfaces:**
- Consumes: `DailyUsageChart` type from `../types.ts`(Task 1)、`GridLines`/`BAR_MAX_HEIGHT`/`BAR_WIDTH`/`GRID_CANVAS_HEIGHT`/`DAY_BAND_HEIGHT`/`GRID_LABEL_GUTTER` from `./GridLines.tsx`(Task 8)、`agentColorVar` from `../theme.ts`(Task 5)、`formatCount` from `../format.ts`(Task 4)。
- Produces: `DailyUsageChart({ chart }: { chart: DailyUsageChartType })`,供 Task 11 使用。

> hover tooltip 用纯 CSS `:hover` 触发(`.usage-day-band:hover .usage-tooltip { display: block }`),不需要 JS 事件监听——对应 `chart.rs::bar_chart` 用 iced `Tooltip` widget 挂在每天条带上的效果,行为一致(悬停哪天显示哪天的明细气泡)。

- [ ] **Step 1: 创建 src/components/DailyUsageChart.tsx**(对应 `chart.rs::bar_chart`,chart.rs:234-339)

```tsx
import type { DailyUsageChart as DailyUsageChartType } from '../types.ts';
import { formatCount } from '../format.ts';
import { agentColorVar } from '../theme.ts';
import { GridLines, BAR_MAX_HEIGHT, BAR_WIDTH, GRID_CANVAS_HEIGHT, GRID_LABEL_GUTTER } from './GridLines.tsx';

// agent 展示名,同 ChartStatList.tsx 的 AGENT_LABEL(逐一对应
// AgentKind::label())。
const AGENT_LABEL: Record<string, string> = {
  claude: 'claude',
  codebuddy: 'codebuddy',
  opencode: 'opencode',
  codex: 'codex',
  goose: 'goose',
  aider: 'aider',
  v8agent: 'v8agent',
  unknown: 'unknown',
};

export function DailyUsageChart({ chart }: { chart: DailyUsageChartType }) {
  const maxTotal = Math.max(1, ...chart.days.flatMap((d) => d.totals));
  const n = chart.days.length;
  return (
    <div class="usage-chart-wrap" style={{ height: '102px', marginLeft: `${GRID_LABEL_GUTTER}px` }}>
      <div class="usage-grid-overlay" style={{ marginLeft: `-${GRID_LABEL_GUTTER}px` }}>
        <GridLines maxTotal={maxTotal} />
      </div>
      <div class={`usage-bar-groups ${n === 1 ? 'usage-bar-groups-center' : ''}`}>
        {chart.days.map((day, i) => (
          <div
            class="usage-day-band"
            style={{ background: i % 2 === 0 ? 'var(--card)' : 'transparent' }}
            key={day.label}
          >
            <div class="usage-tooltip">
              <div class="day-label">{day.label}</div>
              {chart.agents.map(
                (agent, ai) =>
                  day.totals[ai] > 0 && (
                    <div class="usage-legend-row" key={agent}>
                      <span class="usage-legend-dot" style={{ background: agentColorVar(agent) }} />
                      <span class="usage-legend-text">
                        {AGENT_LABEL[agent]} {formatCount(day.totals[ai])}
                      </span>
                    </div>
                  ),
              )}
            </div>
            <div class="usage-bar-row">
              {chart.agents.map((agent, ai) => {
                const value = day.totals[ai];
                const height = Math.max((value / maxTotal) * BAR_MAX_HEIGHT, 1);
                return (
                  <div class="usage-bar-col" key={agent}>
                    <span class="usage-bar-value">{formatCount(value)}</span>
                    <div
                      class="usage-bar"
                      style={{ height: `${height}px`, width: `${BAR_WIDTH}px`, background: agentColorVar(agent) }}
                    />
                  </div>
                );
              })}
            </div>
            <span class="usage-day-label">{day.label}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: 追加 styles.css 规则**(分组柱状图版式,对应 `chart.rs:257-338` 的 `Fill` 空位分布/斑马纹/tooltip 挂载)

```css
/* 追加到 styles.css 末尾 */
.usage-grid-overlay { position: absolute; inset: 0; height: 86px; }
.usage-bar-groups {
  display: flex; justify-content: space-between; align-items: flex-end;
  width: 100%; height: 102px; position: relative;
}
.usage-bar-groups-center { justify-content: center; gap: 8px; }
.usage-day-band {
  position: relative; padding: 0 4px; border-radius: 4px;
  display: flex; flex-direction: column; align-items: center; justify-content: flex-end;
  height: 102px;
}
.usage-day-band .usage-tooltip { display: none; bottom: 100%; left: 50%; }
.usage-day-band:hover .usage-tooltip { display: block; }
.usage-bar-row { display: flex; gap: 3px; align-items: flex-end; height: 86px; }
.usage-bar-col { display: flex; flex-direction: column; align-items: center; gap: 2px; justify-content: flex-end; height: 100%; }
.usage-bar-value { font-size: 8px; color: var(--dim); }
.usage-bar { border-radius: 4px 4px 0 0; }
.usage-day-label { font-size: 8px; color: var(--dim); margin-top: 4px; }
```

- [ ] **Step 3: 类型检查**

```bash
npm run typecheck
```

Expected: 无输出、退出码 0。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/web/usage-content/src/components/DailyUsageChart.tsx crates/dozer-app/web/usage-content/src/styles.css
git commit -m "feat(usage-content): 迁移逐日分组柱状图"
```

---

### Task 10: TrendLineChart.tsx / TrendSection.tsx(趋势折线图 + 悬浮竖线 + tooltip)

**Files:**
- Create: `crates/dozer-app/web/usage-content/src/components/TrendLineChart.tsx`
- Create: `crates/dozer-app/web/usage-content/src/components/TrendSection.tsx`
- Modify: `crates/dozer-app/web/usage-content/src/styles.css`

**Interfaces:**
- Consumes: `TrendChart`/`TokenTrendSection` from `../types.ts`(Task 1)、`SERIES_META` from `../theme.ts`(Task 5)、`GridLines`/`yFor`/`GRID_CANVAS_HEIGHT`/`GRID_LABEL_GUTTER` from `./GridLines.tsx`(Task 8)、`formatCount` from `../format.ts`(Task 4)。
- Produces: `TrendLineChart({ chart }: { chart: TrendChart })`、`TrendSection({ title, chart }: { title: string; chart: TrendChart })`(标题+图例+图表的组合,对应 `chart.rs::trend_chart_section`)、`TokenTrendSectionView({ section }: { section: TokenTrendSection })`(对应 `chart.rs::token_trend_section`),供 Task 11 使用。

> 折线/圆点的 x 轴用 SVG `viewBox` 0-100 单位 + `preserveAspectRatio="none"` 水平拉伸;圆点若也放进这个被非等比拉伸的 SVG 坐标系会被拉成椭圆,所以圆点改用固定像素宽高的 CSS 定位 div(`left` 用百分比、`top` 用像素),折线本身用 `vector-effect="non-scaling-stroke"` 保证描边粗细不被拉伸——两者结合才能在响应式宽度下还原 `chart.rs::TrendLines::draw` 里"等比像素圆点 + 抗拉伸描边"的观感。悬浮竖线与 tooltip 改用一排 `flex:1` 的透明命中格子(每天一格)+ CSS `:hover`,不需要 JS 监听鼠标移动——对应原实现"按 `pitch` 换算光标落在哪天"的效果(不必真的算光标像素位置,离散到"格子"粒度即可,原实现视觉上也是整格高亮)。

- [ ] **Step 1: 创建 src/components/TrendLineChart.tsx**(对应 `chart.rs::TrendLines::draw`/`trend_line_chart`,chart.rs:424-586)

```tsx
import type { TrendChart } from '../types.ts';
import { formatCount } from '../format.ts';
import { SERIES_META } from '../theme.ts';
import { GridLines, yFor, GRID_CANVAS_HEIGHT, GRID_LABEL_GUTTER } from './GridLines.tsx';

export function TrendLineChart({ chart }: { chart: TrendChart }) {
  const days = chart.days;
  const n = Math.max(days.length, 1);
  const series = SERIES_META[chart.kind];
  const maxTotal = Math.max(1, ...days.flatMap((d) => d.values));

  return (
    <div class="usage-chart-wrap" style={{ height: '86px' }}>
      <GridLines maxTotal={maxTotal} />
      <div class="usage-trend-svg-wrap" style={{ marginLeft: `${GRID_LABEL_GUTTER}px` }}>
        <svg
          width="100%"
          height={GRID_CANVAS_HEIGHT}
          viewBox={`0 0 100 ${GRID_CANVAS_HEIGHT}`}
          preserveAspectRatio="none"
        >
          {series.map((s, idx) => (
            <polyline
              key={s.label}
              points={days
                .map((d, i) => `${(100 * (i + 0.5)) / n},${yFor(d.values[idx], maxTotal)}`)
                .join(' ')}
              fill="none"
              stroke={`var(${s.colorVar})`}
              stroke-width="2"
              vector-effect="non-scaling-stroke"
            />
          ))}
        </svg>
        {series.flatMap((s, idx) =>
          days.map((d, i) => (
            <div
              key={`${s.label}-${i}`}
              class="usage-trend-dot"
              style={{
                left: `${((i + 0.5) / n) * 100}%`,
                top: `${yFor(d.values[idx], maxTotal)}px`,
                background: `var(${s.colorVar})`,
              }}
            />
          )),
        )}
        <div class="usage-trend-cells">
          {days.map((d, i) => (
            <div class="usage-trend-cell" key={i}>
              <div class="usage-hover-line" />
              <div class="usage-tooltip">
                <div class="day-label">{d.label}</div>
                {series.map(
                  (s, idx) =>
                    d.values[idx] > 0 && (
                      <div class="usage-legend-row" key={s.label}>
                        <span class="usage-legend-dot" style={{ background: `var(${s.colorVar})` }} />
                        <span class="usage-legend-text">
                          {s.label}: {formatCount(d.values[idx])}
                        </span>
                      </div>
                    ),
                )}
              </div>
            </div>
          ))}
        </div>
      </div>
      <div class="usage-trend-day-labels" style={{ marginLeft: `${GRID_LABEL_GUTTER}px` }}>
        {days.map((d) => (
          <span class="usage-day-label" key={d.label}>
            {d.label}
          </span>
        ))}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: 创建 src/components/TrendSection.tsx**(对应 `chart.rs::trend_chart_section`——标题 + 右侧图例 + 图表)

```tsx
import type { TrendChart, TokenTrendSection as TokenTrendSectionType } from '../types.ts';
import { formatCount } from '../format.ts';
import { SERIES_META } from '../theme.ts';
import { TrendLineChart } from './TrendLineChart.tsx';

function InlineLegend({ chartKind }: { chartKind: TrendChart['kind'] }) {
  return (
    <div class="usage-inline-legend">
      {SERIES_META[chartKind].map((s) => (
        <div class="usage-legend-row" key={s.label}>
          <span class="usage-legend-dot" style={{ background: `var(${s.colorVar})` }} />
          <span class="usage-legend-text">{s.label}</span>
        </div>
      ))}
    </div>
  );
}

export function TrendSection({ title, chart }: { title: string; chart: TrendChart }) {
  return (
    <div class="usage-section">
      <div class="usage-section-head-row">
        <div class="usage-section-head">{title}</div>
        <InlineLegend chartKind={chart.kind} />
      </div>
      <TrendLineChart chart={chart} />
    </div>
  );
}

// 对应 chart.rs::token_trend_section:一个"Token 趋势"标题下,IO 与 Cache
// 两张独立子图各自独立纵轴 max,任一缺失只画另一张,都缺失时上层
// (App.tsx)不渲染这个 section。
export function TokenTrendSectionView({ section }: { section: TokenTrendSectionType }) {
  return (
    <div class="usage-section">
      <div class="usage-section-head">Token 趋势</div>
      {section.io && (
        <>
          <div class="usage-section-head-row">
            <span class="usage-trend-window-tag">
              Input/Output({formatCount(section.io_total)}/{section.io.days.length}days)
            </span>
            <InlineLegend chartKind="io_tokens" />
          </div>
          <TrendLineChart chart={section.io} />
        </>
      )}
      {section.cache && (
        <div class={section.io ? 'usage-subsection-gap' : ''}>
          <div class="usage-section-head-row">
            <span class="usage-trend-window-tag">
              Cache read/write({formatCount(section.cache_total)}/{section.cache.days.length}days)
            </span>
            <InlineLegend chartKind="cache_tokens" />
          </div>
          <TrendLineChart chart={section.cache} />
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 3: 追加 styles.css 规则**

```css
/* 追加到 styles.css 末尾 */
.usage-trend-svg-wrap { position: relative; height: 86px; }
.usage-trend-dot {
  position: absolute; width: 6px; height: 6px; margin-left: -3px; margin-top: -3px;
  border-radius: 50%;
}
.usage-trend-cells { position: absolute; inset: 0; display: flex; }
.usage-trend-cell { position: relative; flex: 1; }
.usage-hover-line {
  display: none; position: absolute; left: 50%; top: 0; bottom: 0; width: 1px;
  background: var(--border);
}
.usage-trend-cell:hover .usage-hover-line { display: block; }
.usage-trend-cell .usage-tooltip { display: none; bottom: 100%; left: 50%; }
.usage-trend-cell:hover .usage-tooltip { display: block; }
.usage-trend-day-labels { display: flex; margin-top: 4px; }
.usage-trend-day-labels .usage-day-label { flex: 1; text-align: center; }
.usage-section-head-row { display: flex; align-items: center; gap: 8px; margin-bottom: 24px; }
.usage-section-head-row .usage-section-head { margin-bottom: 0; flex: none; }
.usage-section-head-row .usage-inline-legend { margin-left: auto; }
.usage-trend-window-tag { font-size: 12px; color: var(--cream); }
```

- [ ] **Step 4: 类型检查**

```bash
npm run typecheck
```

Expected: 无输出、退出码 0。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/web/usage-content/src/components/TrendLineChart.tsx crates/dozer-app/web/usage-content/src/components/TrendSection.tsx crates/dozer-app/web/usage-content/src/styles.css
git commit -m "feat(usage-content): 迁移趋势折线图与图例组合"
```

---

### Task 11: EmptyStates.tsx / App.tsx / 真实 main.tsx(主题解析 + dispatch 监听 + ready 上报)

**Files:**
- Create: `crates/dozer-app/web/usage-content/src/components/EmptyStates.tsx`
- Create: `crates/dozer-app/web/usage-content/src/components/App.tsx`
- Modify: `crates/dozer-app/web/usage-content/src/main.tsx`(替换 Task 1 占位版)
- Modify: `crates/dozer-app/web/usage-content/src/styles.css`

**Interfaces:**
- Consumes: 全部前置组件(Task 6/7/9/10)、`UsageViewPayload` from `../types.ts`(Task 1)。
- Produces: 完整可跑的前端;`window.__dozer.dispatch(json)` 全局入口,`main.tsx` 收到后 `setState` 触发重渲染;页面加载完成后 `window.ipc.postMessage('{"kind":"ready"}')`。

- [ ] **Step 1: 创建 src/components/EmptyStates.tsx**(对应 `view.rs::content_pane` 里"还没有对话记录"/"该 agent 还没有用量数据"两条空态文案,`view.rs:47-62`;**不含**"统计中…",见 Task 2 顶部说明)

```tsx
export function EmptyRows() {
  return <div class="usage-empty">这个项目还没有 agent 对话记录</div>;
}

export function EmptyAgentRows() {
  return <div class="usage-empty">这个 agent 在当前项目还没有用量数据</div>;
}
```

- [ ] **Step 2: 创建 src/components/App.tsx**(顶层按 `payload.kind` 分发;"项目用量统计"卡片在 `single_agent`/`all_agents` 两态**都**渲染,对应 `view.rs` 现状里 `project_summary_boxes` 调用点在 `match ws_state.agent_filter` **之前**,不是只属于 all_agents 一态)

```tsx
import type { UsageViewPayload, ProjectSummary } from '../types.ts';
import { ProjectSummaryBoxes } from './ProjectSummaryBoxes.tsx';
import { AgentMetricsSection } from './AgentMetricsSection.tsx';
import { DailyUsageChart } from './DailyUsageChart.tsx';
import { TrendSection, TokenTrendSectionView } from './TrendSection.tsx';
import { EmptyRows, EmptyAgentRows } from './EmptyStates.tsx';

function ProjectSection({ project, gitCommits }: { project: ProjectSummary; gitCommits: number }) {
  return (
    <div class="usage-section">
      <div class="usage-section-head">项目用量统计</div>
      <ProjectSummaryBoxes project={project} gitCommits={gitCommits} />
    </div>
  );
}

export function App({ payload }: { payload: UsageViewPayload }) {
  switch (payload.kind) {
    case 'empty':
      return <EmptyRows />;
    case 'agent_empty':
      return <EmptyAgentRows />;
    case 'single_agent':
      return (
        <>
          <ProjectSection project={payload.project} gitCommits={payload.git_commits} />
          {payload.session_trend && <TrendSection title="Session 趋势" chart={payload.session_trend} />}
          {payload.token_trend && <TokenTrendSectionView section={payload.token_trend} />}
        </>
      );
    case 'all_agents':
      return (
        <>
          <ProjectSection project={payload.project} gitCommits={payload.git_commits} />
          {payload.agent_metrics && (
            <div class="usage-section">
              <div class="usage-section-head">Agent 用量统计</div>
              <AgentMetricsSection metrics={payload.agent_metrics} />
            </div>
          )}
          {payload.daily_usage && (
            <div class="usage-section">
              <div class="usage-section-head">每日用量统计</div>
              <DailyUsageChart chart={payload.daily_usage} />
            </div>
          )}
          {payload.daily_behavior && <TrendSection title="每日行为统计" chart={payload.daily_behavior} />}
        </>
      );
  }
}
```

- [ ] **Step 3: 替换 src/main.tsx 为真实实现**

```tsx
import { render } from 'preact';
import { useState, useEffect } from 'preact/hooks';
import './styles.css';
import type { UsageViewPayload } from './types.ts';
import { App } from './components/App.tsx';

declare global {
  interface Window {
    ipc?: { postMessage(body: string): void };
    __dozer?: { dispatch(json: string): void };
  }
}

// 主题由 URL `?theme=light|dark` 注入(同 dozer://flyfish、dozer://html
// 的 `scheme_query_value()` 约定),缺省/未知值回落 dark。
function applyThemeFromUrl() {
  const theme = new URLSearchParams(location.search).get('theme');
  document.documentElement.dataset.theme = theme === 'light' ? 'light' : 'dark';
}

function Root() {
  const [payload, setPayload] = useState<UsageViewPayload | null>(null);

  useEffect(() => {
    window.__dozer = {
      dispatch(json: string) {
        try {
          const envelope = JSON.parse(json) as { payload?: UsageViewPayload };
          if (envelope.payload) setPayload(envelope.payload);
        } catch {
          // 忽略无法解析的推送——Rust 侧 `dispatch_script` 只在
          // `__dozer.dispatch` 已注册时才会注入,理论上不该收到坏 JSON。
        }
      },
    };
    // 页面初始化完成、`window.__dozer.dispatch` 已可用,报回 Rust——
    // Rust 侧收到后才开始推送(见 protocol.rs::UsageWebviewEvent::Ready、
    // Task 15 的 `WebviewPushState`)。
    window.ipc?.postMessage(JSON.stringify({ kind: 'ready' }));
    return () => {
      delete window.__dozer;
    };
  }, []);

  if (!payload) {
    return null;
  }
  return <App payload={payload} />;
}

applyThemeFromUrl();
render(<Root />, document.getElementById('root')!);
```

- [ ] **Step 4: 构建 + 用假数据人工冒烟(浏览器打开产物,确认四态各自渲染正常、控制台零报错)**

```bash
cd crates/dozer-app/web/usage-content
npm run build
npm run typecheck
```

用一个最小 HTML 片段本地起服务(如 `python3 -m http.server` 指向 `../../assets/usage-content`),浏览器打开 `host.html`,在控制台手动执行:

```js
window.__dozer.dispatch(JSON.stringify({
  payload: {
    kind: 'all_agents',
    project: { conversation_count: 12, turns: 40, tool_calls: 88, files_touched: 15,
      tokens_in: 12000, tokens_out: 3400, tokens_cache_read: 900, tokens_cache_write: 200 },
    git_commits: 23,
    agent_metrics: {
      session_share: [{ agent: 'claude', value: 8 }, { agent: 'codebuddy', value: 4 }],
      turn_share: [{ agent: 'claude', value: 30 }, { agent: 'codebuddy', value: 10 }],
      io_share: [{ agent: 'claude', value: 12000 }],
      cache_share: [{ agent: 'claude', value: 900 }],
      total_tokens: 16500,
    },
    daily_usage: { agents: ['claude', 'codebuddy'], days: [
      { label: '09/24', totals: [120, 30] }, { label: '09/25', totals: [200, 0] },
    ] },
    daily_behavior: { kind: 'behavior', days: [
      { label: '09/24', values: [3, 1] }, { label: '09/25', values: [5, 2] },
    ] },
  }
}));
```

Expected: 汇总卡片、饼图+图例、逐日柱状图(含斑马纹,hover 出 tooltip)、趋势折线(hover 出竖线+tooltip)全部渲染,控制台零报错、零 CSP 违规。再分别 dispatch `{kind:'empty'}`/`{kind:'agent_empty', agent:'claude'}`/`{kind:'single_agent', ...}` 确认对应文案与图表正确。

- [ ] **Step 5: 追加 styles.css 收尾规则**

```css
/* 追加到 styles.css 末尾 */
.usage-section-head { font-size: 13px; color: var(--cream); margin-bottom: 24px; display: flex; align-items: center; gap: 6px; }
.usage-section-head::before { content: ""; width: 6px; height: 6px; border-radius: 50%; background: var(--cream); }
```

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/web/usage-content/src
git commit -m "feat(usage-content): 接上真实 dispatch 监听与顶层五态分发"
```

---

### Task 12: theme/geometry.rs——`usage_content_chrome_top_px`

**Files:**
- Modify: `crates/dozer-app/src/theme/geometry.rs`

**Interfaces:**
- Produces: `usage_content_chrome_top_px() -> f32`,供 Task 13(`usage_content_pane_bounds_for`)使用。

**背景**(不是随手拍的数字,复刻实际渲染堆栈,同文件里 `tree_chrome_top_px`/`git_log_diff_header_h_px` 的方法论):面板头(图标+"用量"标题+收起按钮)继续原生 iced 渲染,webview 只覆盖头部**以下**的内容区。`content_pane`(`view.rs`)现状是 `column![head].spacing(12).padding(14)`,`head` 来自 `home_panel_head_with_actions`(`chrome/homespace.rs:336-382`):`column![head_row(固定高 tab_button_size()), 1px 分割线].spacing(4)`。Task 15 会把 `content_pane` 里除"统计中…"(留在原生 iced,见 Task 2 顶部说明)之外的四态 body 渲染删掉,`head` 之后只留一个占位容器让 webview 覆盖上去,但**这里的几何常量仍按"头部到内容区起点的视觉间距"设计,不是照抄改造后剩下的字面量**——`.spacing(12)` 在改造后可能因为列只剩一两个子元素而不再产生实际 iced 布局间隙,但这段视觉留白本来就该由 webview 矩形的起始 y 来承担,不依赖 iced 是否还渲染这段间隙。

- [ ] **Step 1: 写失败的测试**

```rust
// 追加到 crates/dozer-app/src/theme/geometry.rs 的 `#[cfg(test)] mod tests` 块内
#[test]
fn usage_content_chrome_top_matches_composition() {
    let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
    // 14(column padding.top) + tab_button_size()(head_row 固定高) +
    // 4.0(head 内部 spacing) + 1.0(head 内部 1px 分割线) +
    // 12.0(标题到内容区的设计留白,对应 column 原 `.spacing(12)`)。
    assert!(near(
        usage_content_chrome_top_px(),
        14.0 + byteui::theme::geometry::tab_button_size() + 4.0 + 1.0 + 12.0
    ));
}
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozer-app theme::geometry::tests::usage_content_chrome_top_matches_composition
```

Expected: FAIL,`usage_content_chrome_top_px` 未定义。

- [ ] **Step 3: 实现**

```rust
// 追加到 crates/dozer-app/src/theme/geometry.rs,`git_log_diff_header_h_px`
// 函数之后
/// 用量面板内容侧 webview 之上、面板头(图标+"用量"标题+收起按钮,继续
/// 原生 iced 渲染)占用的固定高度(逻辑像素),已含全局 scale。
/// `usage_content_pane_bounds_for` 用它算 webview 矩形的纵向起点——
/// webview 只覆盖头部**以下**的内容区,不能把头部也盖住。
///
/// 复刻 `chrome/homespace.rs::home_panel_head_with_actions` 的固定堆栈
/// （`column![head_row(固定高 tab_button_size()), 1px 分割线].spacing(4)`）
/// 加 `content_pane` 外层 `column![head].spacing(12).padding(14)` 的
/// `padding.top` 与到内容区的设计留白,不另起字面量。
pub fn usage_content_chrome_top_px() -> f32 {
    let head_h = byteui::theme::geometry::tab_button_size() + 4.0 + 1.0;
    14.0 + head_h + 12.0
}
```

- [ ] **Step 4: 运行测试确认通过**

```bash
cargo test -p dozer-app theme::geometry
```

Expected: 全部 PASS(含既有 `tree_geometry_matches_composition`/`git_log_diff_header_matches_composition`,确认没有破坏)。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/theme/geometry.rs
git commit -m "feat(usage): 新增用量面板内容侧 chrome 高度几何常量"
```

---

### Task 13: webview_geometry.rs——`usage_content_pane_bounds_for`

**Files:**
- Modify: `crates/dozer-app/src/webview_geometry.rs`

**Interfaces:**
- Consumes: `theme::geometry::usage_content_chrome_top_px`(Task 12)、既有 `pair_content_width`/`pair_columns`/`maximized_box_x_range`/`maximized_box_height`(`app.rs` 已 `pub(crate)`,本文件顶部已 `use` 引入,同 `git_log_diff_pane_bounds_for` 现状)。
- Produces: `usage_content_pane_bounds_for(side, window_width, window_height, state, content_desired, list_visible) -> (f32,f32,f32,f32)`,供 Task 15(`preview_desired`)使用。`content_desired`/`list_visible` 由调用方算好传入——本函数只吃 `&ShellState`,够不到 `ws_state.loading()`/`ws_state.has_agent_filter()` 这些 workspace 业务数据。

**结构**:与 `git_log_diff_pane_bounds_for`(chart.rs:386-496)同谱系(zone/镜像/放大态处理一致),但只有一层配对(内容|分隔线|筛选栏)而非 GitLog 的两层嵌套,且顺序与 GitLog 相反——"内容在前、列表在后"(同 `PanelDims::usage_split` 字段文档、`layout.rs:110-112`),`mirrored` 要取反(同 `preview_content_bounds_for` 里 `PanelKind::Conversations` 分支的处理手法,那也是"内容在前")。`content_desired` 为假(该侧 Usage 正在"统计中…",原生 iced 播动画)或 `list_visible` 为假(无筛选栏数据/被手动收起,内容独占整条配对宽)时的两条分支分别对应 Files 的 `files_tree_collapsed`/收起态处理。

- [x] **Step 1: 写失败的测试**

```rust
// 追加到 crates/dozer-app/src/webview_geometry.rs 既有 `#[cfg(test)] mod
// tests` 块内(复用该块顶部已有的 `test_state()` 辅助函数,不新起一份)。
#[test]
fn usage_content_zero_when_content_not_desired() {
    let state = ShellState {
        left_view: PanelKind::Usage,
        ..test_state()
    };
    let (_, _, w, h) =
        usage_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, false, true);
    assert_eq!((w, h), (0.0, 0.0));
}

#[test]
fn usage_content_zero_when_side_collapsed() {
    let state = ShellState {
        left_view: PanelKind::Usage,
        left_collapsed: true,
        ..test_state()
    };
    let (_, _, w, h) = usage_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, true, true);
    assert_eq!((w, h), (0.0, 0.0));
}

#[test]
fn usage_content_zero_when_panel_kind_is_not_usage() {
    let state = test_state(); // left_view: PanelKind::Files
    let (_, _, w, h) = usage_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, true, true);
    assert_eq!((w, h), (0.0, 0.0));
}

#[test]
fn usage_content_full_zone_width_when_list_not_visible() {
    let state = ShellState {
        left_view: PanelKind::Usage,
        ..test_state()
    };
    let (_, _, w_split, _) =
        usage_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, true, true);
    let (_, _, w_full, _) =
        usage_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, true, false);
    assert!(
        w_full > w_split,
        "无筛选栏时内容应独占整条配对宽,比分栏时更宽:{w_full} vs {w_split}"
    );
}

#[test]
fn usage_content_y_starts_below_chrome_top() {
    let state = ShellState {
        left_view: PanelKind::Usage,
        ..test_state()
    };
    let (_, y, _, _) = usage_content_pane_bounds_for(Side::Left, 1600.0, 900.0, &state, true, true);
    let top_of_zone =
        byteui::theme::geometry::top_bar_height() + theme::region::left_zone().margin.top;
    assert!((y - top_of_zone - theme::geometry::usage_content_chrome_top_px()).abs() < 1.0);
}
```

- [x] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozer-app webview_geometry::tests::usage_content
```

Expected: FAIL,`usage_content_pane_bounds_for` 未定义。

- [x] **Step 3: 实现**(紧接着 `git_log_diff_pane_bounds_for` 之后追加)

```rust
/// 用量面板内容侧 Preact webview 矩形(上/左/宽/高,逻辑像素),供 main.rs
/// 摆放固定单槽的图表 webview 用。
///
/// 与 `git_log_diff_pane_bounds_for` 同谱系(zone/镜像/放大态处理一致),
/// 但只有一层配对(内容|分隔线|筛选栏,"内容在前、列表在后",`mirrored`
/// 要取反——同 `preview_content_bounds_for` 里 `PanelKind::Conversations`
/// 分支的处理手法)而非 GitLog 那样的两层嵌套。内容顶部还要再扣掉面板头
/// (图标+"用量"标题+收起按钮,继续留在原生 iced)的固定高度——webview
/// 只覆盖头部**以下**的内容区,见 `theme::geometry::usage_content_chrome_top_px`。
///
/// `content_desired`:该侧当前是否该挂载这个 webview(`!ws_state.loading()`,
/// 由调用方——`preview_desired`——算好传入,统计中时原生 iced 播放
/// `math_curve` 动画,webview 不挂载)。`list_visible`:agent 筛选栏这一列
/// 是否参与分栏(`ws_state.has_agent_filter() && !state.dims.usage_list_
/// collapsed`,同样由调用方算好传入)——两者都依赖 workspace 业务数据,
/// 本函数只吃 `&ShellState` 够不到。`list_visible` 为假时内容独占整条
/// 配对宽,不经 `pair_columns` 分栏(同 `preview_content_bounds_for` 里
/// `files_tree_collapsed`/`project_list_collapsed` 分支的处理手法)。
///
/// 不可摆放(`!content_desired` / 该侧收起 / 不是 Usage / 放大的是另一侧)
/// 时返回零尺寸矩形。
pub fn usage_content_pane_bounds_for(
    side: Side,
    window_width: f32,
    window_height: f32,
    state: &ShellState,
    content_desired: bool,
    list_visible: bool,
) -> (f32, f32, f32, f32) {
    let zero = || (0.0, 0.0, 0.0, 0.0);
    let kind = match side {
        Side::Left => state.left_view,
        Side::Right => state.right_view,
    };
    let collapsed = match side {
        Side::Left => state.left_collapsed,
        Side::Right => state.right_collapsed,
    };
    if !content_desired || collapsed || kind != PanelKind::Usage {
        return zero();
    }
    let mirrored =
        state.layout.rail_layout.side_of(PanelKind::Usage) != PanelKind::Usage.default_side();
    let chrome_top = theme::geometry::usage_content_chrome_top_px();

    // 给定"面板区外框(区)矩形" → 内容矩形。放大态与非放大态只差这个输入
    // 矩形,分块逻辑共用(同 `git_log_diff_pane_bounds_for` 的 `compute` 手法)。
    let compute = |inx: f32, iny: f32, inw: f32, inh: f32| -> (f32, f32, f32, f32) {
        let (x, w) = if list_visible {
            let pair_w = pair_content_width(inw);
            let cols = pair_columns(pair_w, state.dims.usage_split, !mirrored);
            (inx + cols.content_x, cols.content_w.max(0.0))
        } else {
            (inx, inw.max(0.0))
        };
        let y = iny + chrome_top;
        let h = (inh - chrome_top).max(0.0);
        (x, y, w, h)
    };

    if let Some(maximized) = state.maximized {
        let showing_side = match maximized {
            MaximizedPane::Left => Side::Left,
            MaximizedPane::Right => Side::Right,
        };
        if side != showing_side {
            return zero();
        }
        let m = match side {
            Side::Left => theme::region::left_zone().margin,
            Side::Right => theme::region::right_zone().margin,
        };
        let (x0, avail_w) = maximized_box_x_range(window_width);
        let inx = x0 + m.left;
        let inw = (avail_w - m.left - m.right).max(0.0);
        let iny = byteui::theme::geometry::top_bar_height()
            + byteui::theme::geometry::maximize_overlay_padding()
            + m.top;
        let inh = (maximized_box_height(window_height)
            - byteui::theme::geometry::maximize_overlay_padding()
            - byteui::theme::geometry::status_bar_height()
            - m.top
            - m.bottom)
            .max(0.0);
        return compute(inx, iny, inw, inh);
    }

    let m = match side {
        Side::Left => theme::region::left_zone().margin,
        Side::Right => theme::region::right_zone().margin,
    };
    let zone_x0 = match side {
        Side::Left => byteui::theme::geometry::icon_rail_width(),
        Side::Right => {
            window_width
                - byteui::theme::geometry::icon_rail_width()
                - right_zone_width(window_width, state)
        }
    };
    let zone_raw_w = match side {
        Side::Left => left_zone_width(window_width, state),
        Side::Right => right_zone_width(window_width, state),
    };
    let inx = zone_x0 + m.left;
    let iny = byteui::theme::geometry::top_bar_height() + m.top;
    let inw = (zone_raw_w - m.left - m.right).max(0.0);
    let inh =
        (window_height - iny - m.bottom - byteui::theme::geometry::status_bar_height()).max(0.0);
    compute(inx, iny, inw, inh)
}
```

- [x] **Step 4: 运行测试确认通过**

```bash
cargo test -p dozer-app webview_geometry
```

Expected: 全部 PASS(含既有 `git_log_diff_pane_bounds_for` 相关测试,确认没有破坏)。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/src/webview_geometry.rs
git commit -m "feat(usage): 新增内容侧 webview 矩形几何计算"
```

---

### Task 14: WebviewPushState(App 级推送判定)+ Message::UsageContentWebviewEvent

**Files:**
- Modify: `crates/dozer-app/src/extensions/usage/mod.rs`
- Modify: `crates/dozer-app/src/app/message.rs`

**Interfaces:**
- Consumes: `UsageViewPayload`/`UsageWebviewEvent` from `protocol.rs`(Task 2)。
- Produces: `usage::WebviewPushState`(`set_ready`/`pending_push`/`mark_sent`)、`Message::UsageContentWebviewEvent(UsageWebviewEvent)`,供 Task 15(`App::usage_webview` 字段、`take_usage_content_script`)、Task 16(`update.rs`/`runtime.rs`)使用。

**为什么是 App 级而不是 `WorkspaceState` 里的字段**(同 `git_log::State` 的先例,见其文档注释"现在挂在 App(不按项目分)"):这个 webview 是**每侧一个固定单槽**,不按项目分——切换到另一个仍显示用量面板的项目 tab 时,同一个 webview 实例要显示新项目的数据。`last_sent` 若挂在 `WorkspaceState`(每项目一份),`desired` 换了个从没在这个 webview 里出现过的 workspace 时,该 workspace 自己的 `last_sent` 是 `None`,天然会判定"要推"——这个设计已经正确处理了这个真实边界(Review Focus 第二条),不需要额外的"workspace 切换"特判,但要写测试锁定这一点,不能只靠巧合。

- [x] **Step 1: 写失败的测试**

```rust
// 追加到 crates/dozer-app/src/extensions/usage/mod.rs 的 `#[cfg(test)] mod tests` 块内
fn payload_a() -> UsageViewPayload {
    UsageViewPayload::AgentEmpty {
        agent: AgentKind::Claude,
    }
}

fn payload_b() -> UsageViewPayload {
    UsageViewPayload::AgentEmpty {
        agent: AgentKind::Codebuddy,
    }
}

#[test]
fn pending_push_none_when_not_ready() {
    let state = WebviewPushState::default();
    assert!(state.pending_push(&payload_a()).is_none());
}

#[test]
fn pending_push_some_when_ready_and_never_sent() {
    let mut state = WebviewPushState::default();
    state.set_ready(true);
    assert_eq!(state.pending_push(&payload_a()), Some(payload_a()));
}

#[test]
fn pending_push_none_once_marked_sent_and_desired_unchanged() {
    let mut state = WebviewPushState::default();
    state.set_ready(true);
    state.mark_sent(payload_a());
    assert!(state.pending_push(&payload_a()).is_none());
}

/// 对应本计划 Review Focus"多 agent 筛选切换的最新覆盖旧语义":desired
/// 在 A→B→A 之间反复横跳,每次跟上一次真正送达的不一样都要判定为待推,
/// 不能因为"A 以前发过一次"就误判成不用重发。
#[test]
fn pending_push_resends_when_desired_flips_back_to_a_previously_sent_value() {
    let mut state = WebviewPushState::default();
    state.set_ready(true);
    state.mark_sent(payload_a());
    assert_eq!(state.pending_push(&payload_b()), Some(payload_b()));
    state.mark_sent(payload_b());
    assert_eq!(
        state.pending_push(&payload_a()),
        Some(payload_a()),
        "desired 变回 A(即便 A 是更早发过的值)也必须判定为待推"
    );
}

/// 对应本计划 Review Focus"切换项目后 webview 显示旧项目数据":这里用
/// "desired 突然换成另一个 workspace 算出来的、从未出现过的 payload"模拟
/// 项目切换——`last_sent` 是 App 级单槽,不按项目分,天然不会因为
/// "这个项目以前没发过"而漏推。
#[test]
fn pending_push_treats_a_different_workspaces_payload_as_new() {
    let mut state = WebviewPushState::default();
    state.set_ready(true);
    state.mark_sent(payload_a()); // 上一个显示这个 webview 的项目留下的内容
    let other_project_payload = UsageViewPayload::Empty; // 切到的新项目,当前态
    assert_eq!(
        state.pending_push(&other_project_payload),
        Some(UsageViewPayload::Empty)
    );
}

#[test]
fn set_ready_true_clears_last_sent_forcing_a_resend() {
    let mut state = WebviewPushState::default();
    state.set_ready(true);
    state.mark_sent(payload_a());
    assert!(state.pending_push(&payload_a()).is_none());
    // webview 被销毁重建(全新实例),重新 ready——即便 desired 没变,也要
    // 强制重发一次,因为新实例的 JS 端状态是空的(同 git_log 的先例)。
    state.set_ready(false);
    state.set_ready(true);
    assert_eq!(state.pending_push(&payload_a()), Some(payload_a()));
}
```

- [x] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozer-app usage::tests
```

Expected: FAIL,`WebviewPushState` 未定义。

- [x] **Step 3: 实现 `WebviewPushState`**

```rust
// 追加到 crates/dozer-app/src/extensions/usage/mod.rs
/// App 级(不按项目分,同 `git_log::State` 先例)内容侧 webview 推送状态:
/// `ready`(JS `window.__dozer.dispatch` 已注册)+ `last_sent`(最近一次
/// 真正 `evaluate_script` 成功的 payload)。纯状态,可单测,不碰 webview 池
/// ——调用方(`App::take_usage_content_script`,Task 15)拿 `pending_push`
/// 的结果去组 envelope,只有真正 `evaluate_script` 成功才调 `mark_sent`。
#[derive(Default)]
pub struct WebviewPushState {
    ready: bool,
    last_sent: Option<UsageViewPayload>,
}

impl WebviewPushState {
    pub fn set_ready(&mut self, ready: bool) {
        self.ready = ready;
        if ready {
            // 新实例(或重新确认 ready)一律强制重发一次当前内容,同
            // `git_log::State::set_diff_webview_ready` 配
            // `clear_diff_sent_for` 的先例。
            self.last_sent = None;
        }
    }

    pub fn pending_push(&self, desired: &UsageViewPayload) -> Option<UsageViewPayload> {
        if !self.ready {
            return None;
        }
        if self.last_sent.as_ref() == Some(desired) {
            return None;
        }
        Some(desired.clone())
    }

    pub fn mark_sent(&mut self, payload: UsageViewPayload) {
        self.last_sent = Some(payload);
    }
}
```

- [x] **Step 4: 新增 `Message::UsageContentWebviewEvent`**

```rust
// crates/dozer-app/src/app/message.rs,`GitLogDiffWebviewEvent`/
// `FileHistoryDiffWebviewEvent` 变体附近新增。不带 binding——固定单槽、
// 不按项目分,没有 tab/document 身份需要校验(同 `WebviewPushState`
// 不按项目分的理由一致),直接携带已解析事件。
UsageContentWebviewEvent(crate::extensions::usage::UsageWebviewEvent),
```

- [x] **Step 5: 运行测试确认通过**

```bash
cargo test -p dozer-app usage::tests
```

Expected: 全部 PASS(既有 + 新增 6 个)。

- [x] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/usage/mod.rs crates/dozer-app/src/app/message.rs
git commit -m "feat(usage): 新增 App 级 webview 推送判定状态与事件消息"
```

---

### Task 15: app.rs 接线(常量/字段/`preview_desired`/`take_usage_content_script`)+ content_pane 让位

**Files:**
- Modify: `crates/dozer-app/src/app/app.rs`
- Modify: `crates/dozer-app/src/extensions/usage/view.rs`

**Interfaces:**
- Consumes: `usage::WebviewPushState`(Task 14)、`usage::current_view_payload`/`UsageViewPayload`(Task 2)、`webview_geometry::usage_content_pane_bounds_for`(Task 13)、`preview::dispatch_script`(既有,`preview/webview_protocol.rs`)。
- Produces: `USAGE_CONTENT_ID_OFFSET` 常量、`App::usage_webview` 字段、`preview_desired` 新分支、`App::take_usage_content_script`,供 Task 16(`window_events.rs`/`runtime.rs`)使用。

> `chart.rs` 里的函数(`bar_chart`/`pie_chart`/`trend_line_chart` 等)在本任务改完后会暂时变成"没人调用"——这是预期的中间状态,Task 17 才删除 `chart.rs` 并把 `has_any_value`/`trend_total` 两个纯函数搬进 `protocol.rs`(Task 2 已经在用它们,`use super::*` 目前还能从 `chart.rs` 拿到)。中间状态如果 `cargo clippy` 报 `dead_code`,属于预期,不要在本任务里提前删 `chart.rs`——那样会让 Task 2 的 `has_any_value`/`trend_total` 引用失败,两个改动应该保持任务边界清晰、各自可独立 review。

- [x] **Step 1: 新增常量与 `App` 字段**

```rust
// crates/dozer-app/src/app/app.rs,GIT_LOG_DIFF_ID_OFFSET 常量(app.rs:631)
// 之后新增,延续既有 1_000_000 递增序列。
pub(crate) const USAGE_CONTENT_ID_OFFSET: usize = 4_000_000;
```

```rust
// crates/dozer-app/src/app/app.rs,`App` 结构体内 `git_log: git_log::State,`
// 字段(app.rs:453)之后新增。
pub(crate) usage_webview: crate::extensions::usage::WebviewPushState,
```

若 `App` 有手写 `impl Default`/构造函数逐字段初始化(而非 `#[derive(Default)]` 覆盖到底),同步在那里补上 `usage_webview: Default::default(),`。

- [x] **Step 2: `preview_desired` 新增 `PanelKind::Usage` 分支**

```rust
// crates/dozer-app/src/app/app.rs,`preview_desired` 内,`if kind ==
// PanelKind::GitLog { ... continue; }` 分支(app.rs:3171-3204)之后、
// `let (mut specs, id_offset): (Vec<WebviewSpec>, usize) = match kind {`
// 之前插入。`ws` 是函数顶部已经 `let Some(ws) = self.active_workspace()`
// 绑定的当前活跃 workspace。
if kind == PanelKind::Usage {
    // 统计中(`math_curve` 动画)时不挂载——原生 iced 继续播动画。
    let content_desired = !ws.usage.loading();
    // 无筛选栏数据 / 被手动收起,内容独占整条配对宽。
    let list_visible = ws.usage.has_agent_filter() && !self.list_collapsed(PanelKind::Usage);
    let bounds = crate::webview_geometry::usage_content_pane_bounds_for(
        side,
        window_width,
        window_height,
        &self.shell_state(),
        content_desired,
        list_visible,
    );
    if bounds.2 > 0.0 && bounds.3 > 0.0 {
        let spec = WebviewSpec {
            id: USAGE_CONTENT_ID_OFFSET,
            url: format!(
                "dozer://usage-content/host.html?theme={}",
                crate::preview::scheme_query_value()
            ),
            visible: !app_modal_open,
            editor_binding: None,
            loading_generation: None,
        };
        out.push((spec, bounds));
    }
    continue;
}
```

- [x] **Step 3: 新增 `App::take_usage_content_script`**

```rust
// crates/dozer-app/src/app/app.rs,`take_git_log_diff_script` 方法之后新增
/// 用量面板内容侧待下发推送:每帧轮询"当前该显示什么"(`current_view_
/// payload`)与"上次真正送达的是什么"(`usage_webview.pending_push`)是否
/// 一致,不一致且 webview 已 ready 才组 envelope。与 `take_git_log_diff_
/// script` 同一节奏(`window_events.rs::apply_pending_editor_commands`
/// 消费,见 Task 16),但判定逻辑是"声明式比较当前值"而非"事件驱动",因为
/// 这个 webview 固定单槽、不按项目分——项目切换、agent 筛选切换都统一
/// 走"这一帧算出来的 desired 和上次不一样就推"这一条路径,不需要分别处理
/// 每种触发源。
pub fn take_usage_content_script(
    &mut self,
    available_webview_ids: &std::collections::HashSet<usize>,
) -> Vec<(usize, String)> {
    let webview_id = USAGE_CONTENT_ID_OFFSET;
    if !available_webview_ids.contains(&webview_id) {
        // webview 尚未进池/已被销毁:降级 ready,下次真正 ready 事件到达
        // 前不再尝试推送(同 `pending_diff_push` 系列先例的"重试不丢内容"
        // 语义,只是这里改用主动降级而不是等待外部信号)。
        self.usage_webview.set_ready(false);
        return Vec::new();
    }
    let Some(ws) = self.active_workspace() else {
        return Vec::new();
    };
    if ws.usage.loading() {
        return Vec::new();
    }
    let desired = crate::extensions::usage::current_view_payload(&ws.usage);
    let Some(payload) = self.usage_webview.pending_push(&desired) else {
        return Vec::new();
    };
    let envelope = crate::extensions::usage::encode_usage_push(payload.clone());
    self.usage_webview.mark_sent(payload);
    vec![(webview_id, crate::preview::dispatch_script(&envelope))]
}
```

- [x] **Step 4: `content_pane` 让位给 webview(删掉四态 body 渲染,保留"统计中…"原生分支与面板头)**

```rust
// crates/dozer-app/src/extensions/usage/view.rs::content_pane——把
// `if loading { ... } else if rows.is_empty() { ... } else { ... 五态
// body ... }` 整段(view.rs:41-193)删掉,只保留 head 与外层容器。
pub fn content_pane<'a>(
    app: &crate::app::App,
    ws_state: &'a WorkspaceState,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 统计中(`math_curve` 动画)时没有 webview 可挂,原生渲染那个动画——
    // 唯一还留在这里的"内容态"分支,其余四态全部搬进
    // `dozer://usage-content` webview(见 protocol.rs::current_view_payload)。
    let collapse = app.list_collapse_button(
        crate::app::PanelKind::Usage,
        app.list_collapsed(crate::app::PanelKind::Usage),
        crate::app::HoverId::UsageListCollapse,
        "收起列表",
        "展开列表",
        Message::ToggleListCollapse,
        move |hovered| Message::Hover(crate::app::HoverId::UsageListCollapse, hovered),
    );
    let head = crate::chrome::homespace::home_panel_head_with_actions(
        icons::IconKind::BarChart3,
        "用量",
        Some(collapse),
    );
    let mut content = column![head].spacing(12).padding(14).width(Length::Fill);
    if ws_state.loading() {
        content = content.push(byteui::feedback::math_curve::loading_hint(
            byteui::feedback::math_curve::Curve::RoseThree,
            "统计中…",
            64.0,
        ));
    }
    // `rows.is_empty()`/该 agent 无数据/单 agent 趋势/全部 agent 汇总
    // 四态:webview 矩形由 `App::preview_desired`(Task 15 Step 2)按
    // `usage_content_pane_bounds_for` 摆放,原生这里不需要再画任何占位
    // ——`Length::Fill` 的这个 `container` 仍然提供面板背景/边框,
    // webview 盖在它上面(同 Files/Project/GitLog 现状)。
    container(content)
        .width(width)
        .height(Length::Fill)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(byteui::theme::color::current().panel.into()),
                border: outer,
                ..iced_widget::container::Style::default()
            },
        )
        .into()
}
```

- [x] **Step 5: 编译确认(`cargo check`,预期出现 Task 17 才处理的 dead_code 警告,不阻塞)**

```bash
cargo check -p dozer-app
```

Expected: 编译通过;`chart.rs` 里除 `has_any_value`/`trend_total` 外的函数出现 `never used` 警告(预期中的中间态,Task 17 处理)。

- [x] **Step 6: Commit**

```bash
git add crates/dozer-app/src/app/app.rs crates/dozer-app/src/extensions/usage/view.rs
git commit -m "feat(usage): app.rs 接入内容侧 webview 挂载与推送轮询,content_pane 让位"
```

---

### Task 16: update.rs 事件处理 + window_events.rs 消费 + runtime.rs IPC 路由

**Files:**
- Modify: `crates/dozer-app/src/app/update.rs`
- Modify: `crates/dozer-app/src/platform/window_events.rs`
- Modify: `crates/dozer-app/src/runtime.rs`

**Interfaces:**
- Consumes: `Message::UsageContentWebviewEvent`(Task 14)、`App::take_usage_content_script`(Task 15)、`usage::parse_usage_event`(Task 2)。

这是整条推送链路的最后一段:JS `ready` 上报 → `runtime.rs` IPC handler 解析 → `Message::UsageContentWebviewEvent` → `update.rs` 翻 `ready` 状态 → 下一帧 `window_events.rs` 消费 `take_usage_content_script` → `evaluate_script` 真正推送。

- [x] **Step 1: `update.rs` 处理 `UsageContentWebviewEvent`**

```rust
// crates/dozer-app/src/app/update.rs,`Message::GitLogDiffWebviewEvent`
// 分支之后新增
Message::UsageContentWebviewEvent(event) => {
    if matches!(event, crate::extensions::usage::UsageWebviewEvent::Ready) {
        // `set_ready(true)` 内部已经清空 `last_sent`,强制下一帧重发
        // 一次当前内容(覆盖"webview 被销毁重建,新实例第一次 ready"的
        // 场景),不需要在这里再显式清一次。
        self.usage_webview.set_ready(true);
    }
}
```

- [x] **Step 2: `window_events.rs` 消费待推送脚本**

```rust
// crates/dozer-app/src/platform/window_events.rs::apply_pending_editor_commands,
// 紧接着 `take_git_log_diff_script` 那段消费循环之后新增(同一节奏)
// 用量面板内容侧 webview(单固定槽,不在 PreviewPane tab 模型里):
// 内容经声明式比较(`WebviewPushState::pending_push`)推送,与上面的
// Git Log diff 命令同一注入节奏。
for (webview_id, js) in app.take_usage_content_script(&available_webview_ids) {
    if let Some((view, _)) = webviews.get(&webview_id) {
        let _ = view.evaluate_script(&js);
    }
}
```

- [x] **Step 3: `runtime.rs` IPC handler 识别 usage webview id、解析 `ready` 事件**

```rust
// crates/dozer-app/src/runtime.rs,`with_ipc_handler` 闭包内 `_ => { ... }`
// 分支(runtime.rs:499 起),`if let Some(binding) = flyfish_binding.as_ref()
// && looks_like_envelope { ... }` 之后、`else if let Some(binding) =
// editor_binding.as_ref() { ... }` 之前插入一个 `else if` 分支。usage-content
// 不是 CodeMirror/JSON/Flyfish 家族,不复用那三者任何一个 binding,直接
// 按固定 webview id 判断(单槽面板,没有 tab/document 身份需要携带)。
} else if webview_id == crate::app::USAGE_CONTENT_ID_OFFSET && looks_like_envelope {
    match crate::extensions::usage::parse_usage_event(body) {
        Ok(event) => {
            let _ =
                ipc_proxy.send_event(Message::UsageContentWebviewEvent(event));
        }
        Err(error) => {
            tracing::warn!(%error, "无法解析 usage-content IPC");
        }
    }
} else if let Some(binding) = editor_binding.as_ref() {
```

> 插入点提醒:原有代码是 `if let Some(binding) = flyfish_binding.as_ref() && looks_like_envelope { ... } else if let Some(binding) = editor_binding.as_ref() { ... } else { ... 兜底 WebViewFocused }`——上面这段只是在第一个 `else if` 前面**再插一个 `else if`**,不改动前后两段既有分支的内容,`webview_id` 变量在这个闭包作用域内已经存在(同 `find_page`/`find_native` 分支已经在用它),不需要额外捕获。

- [x] **Step 4: 手动集成验证(暂无自动化 headless webview 测试,走真机)**

```bash
cargo build -p dozer-app
cargo run -p dozer-app
```

打开一个有真实 agent 对话记录的项目,切到"用量"面板:

- 确认内容侧图表以 webview 渲染(浏览器 DevTools 风格检查——或先确认视觉与旧版一致,下一 Task 有更系统的核对清单)。
- 连续点击右侧 agent 筛选栏多个 agent,确认每次都是"内容原地更新"、没有整页闪烁。
- 收起/展开右侧筛选栏,确认内容宽度正确联动。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/src/app/update.rs crates/dozer-app/src/platform/window_events.rs crates/dozer-app/src/runtime.rs
git commit -m "feat(usage): 接通 ready 事件回传与推送注入的完整闭环"
```

---

### Task 17: 清理——删除 chart.rs,搬走仍有用的纯函数

**Files:**
- Modify: `crates/dozer-app/src/extensions/usage/protocol.rs`(接收搬入的纯函数)
- Delete: `crates/dozer-app/src/extensions/usage/chart.rs`
- Modify: `crates/dozer-app/src/extensions/usage/mod.rs`(去掉 `mod chart;`/`pub(crate) use chart::*;`)

**Interfaces:**
- Consumes: 无(纯删除+搬迁,不新增对外接口)。

`chart.rs` 997 行里唯二还有实际用处的纯函数是 `has_any_value`/`trend_total`(`protocol.rs` 的 `trend_chart`/`single_agent_token_trend` 已经在用,此前靠 `mod chart` 还没删、`use super::*` 能拿到)——本任务把这两个函数原样搬进 `protocol.rs`,再删掉整个 `chart.rs`,顺带清掉 `mod.rs` 里的模块声明。`format_count`/`nice_tick_step`/`grid_ticks`/`PIE_RADIUS`/`PIE_GAP_RAD`/`BAR_MAX_HEIGHT` 等其余内容在 Rust 侧不再需要——它们已经在 Task 4/7/8 里各自等价迁到 `format.ts`/`pieSlices.ts`/`GridLines.tsx`,前端拥有自己独立的实现与测试,不指望 Rust 那份继续存在。

- [x] **Step 1: 把 `has_any_value`/`trend_total` 搬进 `protocol.rs`**

```rust
// 追加到 crates/dozer-app/src/extensions/usage/protocol.rs,
// `current_view_payload` 定义之前(原样从 chart.rs:588-600 搬来,
// 注释一并保留)
/// `days` 里是否至少一天有非零值——趋势窗口若整段都是 0(目标 agent 最近
/// 这段时间其实没活动),上层就不画这个趋势区,避免白框空难读。
fn has_any_value(days: &[DaySeries]) -> bool {
    days.iter().any(|d| d.values.iter().any(|&v| v > 0))
}

/// 某趋势窗口内所有天、所有子序列值的总和——用于把"标签 + 总量 + 天数"揉
/// 成一行摘要式图例(如 `Input/Output(23.2m/15days)`)。
fn trend_total(days: &[DaySeries]) -> u64 {
    days.iter().flat_map(|d| d.values.iter().copied()).sum()
}
```

- [x] **Step 2: 删除 `chart.rs`**

```bash
git rm crates/dozer-app/src/extensions/usage/chart.rs
```

- [x] **Step 3: 修改 `mod.rs`,去掉 chart 模块声明**

```rust
// crates/dozer-app/src/extensions/usage/mod.rs,删掉这两行:
// mod chart;
// pub(crate) use chart::*;
```

- [x] **Step 4: 编译确认(此时不该再有任何 dead_code 警告,`view.rs` 若还残留对 `chart::*` 任何函数的引用会直接编译失败,能借此确认 Task 15 Step 4 删干净了)**

```bash
cargo check -p dozer-app
cargo clippy -p dozer-app --all-targets
```

Expected: 两者都无 `usage` 相关警告/错误。若 `view.rs` 报缺函数,回到 Task 15 Step 4 确认 `content_pane` 是否还有遗漏的 `chart::` 调用点未删干净。

- [x] **Step 5: 跑用量面板全部现有单测,确认聚合逻辑/协议逻辑均未受影响**

```bash
cargo test -p dozer-app usage::
```

Expected: 全部 PASS(`aggregate.rs` 原有测试 + Task 2/13/14 新增测试)。

- [x] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/usage/protocol.rs crates/dozer-app/src/extensions/usage/mod.rs
git commit -m "refactor(usage): 删除 chart.rs,搬走仍有用的纯函数"
```

---

### Task 18: 全量验证 + 人工视觉核对

**Files:** 无新增/修改,纯验证。

- [x] **Step 1: 全 workspace 构建 + 测试 + lint**

```bash
cargo build
cargo test -p dozer-app
cargo clippy --all-targets
cargo fmt --check
```

Expected: 全部通过,`cargo fmt --check` 无 diff。

- [x] **Step 2: 前端全量测试**

```bash
cd crates/dozer-app/web/usage-content
npm run typecheck
npm test
npm run build
```

Expected: 类型检查/单测全部 PASS,构建产物就绪。

- [x] **Step 3: 人工视觉核对清单(浏览器/真机跑 `cargo run -p dozer-app`,对照旧版本截图或口述记忆,逐条确认)**

- [x] 空态:项目无任何 agent 对话记录时,内容侧显示"这个项目还没有 agent 对话记录",筛选栏不显示(`has_agent_filter()` 为假)。
- [x] 该 agent 无数据态:选中一个当前项目没跑过的 agent,内容侧显示"这个 agent 在当前项目还没有用量数据"。
- [x] 统计中态:切到用量面板瞬间(或人为制造慢查询)看到原生 `math_curve` 动画,不是白屏/webview 空白。
- [x] 单 agent 态:选中某个真实用过的 agent,项目汇总卡片 + Session 趋势折线(会话/回合双线,hover 显示逐日明细竖线+tooltip)+ Token 趋势(Input/Output、Cache read/write 两张独立子图,数据都缺时对应子图不出现)。
- [x] 全部 agent 态:项目汇总卡片 + Agent 用量统计(Session/Round 环图对 + Tokens Input/Output/Cache 环图对,饼图配色与右侧筛选栏色点一致)+ 每日用量统计(分组柱状图,斑马纹条带,hover 显示逐 agent 明细)+ 每日行为统计(触达文件/Git提交双线趋势)。
- [x] agent 筛选栏连续切换多个 agent:每次都是内容"原地更新",无整页白屏闪烁(这是本次改造相对 review-trace 模式的核心验证点)。
- [x] 收起/展开右侧筛选栏(`收起列表`/`展开列表` 按钮):内容侧宽度正确联动放大/收窄,webview 矩形没有滞后一帧或残留旧宽度。
- [x] 明暗主题切换:内容侧配色跟随当前主题(不是恒暗色)。
- [x] 窗口放大(该侧 maximize):内容侧正确占满放大盒子,矩形没有跑到别处。
- [x] 控制台(如可开发者工具核查)零报错、零 CSP 违规。

- [x] **Step 4: 最终 Commit(若 Step 3 发现问题已在前面任务修复,这里只是收尾确认,通常无新改动;如有小修在此提交)**

```bash
git status
# 若有未提交的小修改:
git add -A
git commit -m "fix(usage): 人工视觉核对后的收尾修正"
```
