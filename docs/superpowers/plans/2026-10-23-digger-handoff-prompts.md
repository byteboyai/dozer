# Digger S6 交接提示词(2026-10-23)

按顺序把下面的提示词贴给开发 agent。每份交接说明都在同目录。每个 agent 做完会给报告路径,交审核方在干净快照里重跑后再放行下一步。

依赖顺序:`feat/diggerd` → `feat/agent-sessions`(1b → 1c)→ `feat/agent-chrome`(S6d-3)。S6e-2 是单独一条线,可并行。

## 第 1 步:S6d-2 原 agent,先做 1b

```
S6d-2 审核完成,有修改。请先读这个文件并执行:
/Users/chrischiang/Projects/CoralProjects/byteboy/dozer/docs/superpowers/plans/2026-10-23-digger-handoff-1b-s6d2-review-fixes.md

在 ../digger-s6d2 的 feat/agent-sessions 上追加提交,不改写已有提交。
做完把报告路径告诉我,不要继续下一步。
```

## 第 1 步续:1b 复核通过后,同一个 agent 做 1c

```
1b 复核通过。输入法探针用户已真机验证全部通过。
请读并执行:
/Users/chrischiang/Projects/CoralProjects/byteboy/dozer/docs/superpowers/plans/2026-10-23-digger-handoff-1c-s6d2-task5-keyboard-ime.md

同一分支同一 worktree。需要人手的真机项写进报告"待用户",不要卡住。
```

## 第 2 步:S6d-3(1b 复核通过后即可开始,不等 1c)

```
请读并执行:
/Users/chrischiang/Projects/CoralProjects/byteboy/dozer/docs/superpowers/plans/2026-10-23-digger-handoff-2-s6d3-chrome-and-release.md

基线是 feat/agent-sessions 修复后的顶端(先问我确认哪个提交),新建 ../digger-s6d3、分支 feat/agent-chrome。
发布、升版本、打 tag、推送都先停下来问我。
```

## 第 3 步:S6e-2(随时可并行)

```
请读并执行:
/Users/chrischiang/Projects/CoralProjects/byteboy/dozer/docs/superpowers/plans/2026-10-23-digger-handoff-3-s6e2-digger-mcp.md

基线核对:从 feat/diggerd(46bb4c9)切 ../digger-s6e2、分支 feat/digger-mcp。不符就停下来问。
```

## 注意

- 三条线各用独立 worktree,互不碰对方目录。
- 1c 与 S6d-3 都会改 `product.rs`、`adapters/agent.rs`,后合并的一方处理冲突。
- 新开的 agent 读不到之前的对话,只看得到文件;背景都在交接说明里。
- 合并 `main`、打 tag、推送、发布一律先问用户。
