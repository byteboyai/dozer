# bytehost 人工验收清单(需要在 Dozer 窗口里操作或判断的项)

> 日期:2026-10-16。来源:`docs/superpowers/specs/` 下 A5、A6e、A6f、A6g、A6h 五份验收报告里"待人工在真实 GUI 里完成"的项,以及 `2026-10-14-bytehost-app-i18n-acceptance.md` 里标 ⏳ 的项。
> 命令行能验证的项(伪造 `Host` 被拒、令牌门、同源校验、crash/hang 自动恢复、停重启 dozerd 后应用恢复)已在隔离环境里自动跑过,**不在本清单里**。
> 分级与自动化可行性见 `docs/dozer-v2/bytehost-manual-acceptance-checklist.md`。
> **全部项有结论之前,不得把 A5、A6e–A6h 标成"已完成"。**

## 0. 准备(一次)

用隔离目录起一套独立的 dozerd 和 GUI,不碰真实的 Dozer 数据。

终端 1:

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
export HOME=/tmp/dzh
target/aarch64-apple-darwin/debug/dozerd
```

终端 2:

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
export HOME=/tmp/dzh
target/aarch64-apple-darwin/debug/dozer
```

- 必须用包含提交 `6bbf8e3d`(dozerd 处理 SIGTERM)的构建,否则停 dozerd 时可能留下孤儿应用进程。
- 样例目录:`/Users/chrischiang/Projects/CoralProjects/byteboy/bytehost/scripts/samples/`(`py-notes`、`node-notes`)。
- Excalidraw 先打包(要联网):`sh /Users/chrischiang/Projects/CoralProjects/byteboy/bytehost/scripts/excalidraw/package.sh ~/excalidraw-app`。
- 安装入口:设置(右上角齿轮)→ 应用 → 安装应用…。
- 想让 Claude 同步截图确认某一步,说明是哪一步即可(用 `orca computer --app pid:<GUI 的 pid>`;iced 界面不在辅助功能树里,只能靠截图和坐标,点击目前不可靠)。
- 结束后清理:`rm -rf /tmp/dzh`。

## 1. Excalidraw(A5)

| # | 操作 | 预期 | 结果 |
|---|---|---|---|
| 1.1 | 安装 `~/excalidraw-app` | 审批卡显示来源"本地目录"、运行方式 `static_web`、权限全最严 | ☐ |
| 1.2 | 批准安装,图标栏出现图标,打开,画几笔 | 画布正常 | ☐ |
| 1.3 | 退出整个 Dozer(dozerd 保持运行)再重开,进该应用 | 画还在 | ☐ |
| 1.4 | 审批卡停留时,另开终端改 `~/excalidraw-app/web/index.html` 再点批准 | 提示"应用源码在审批之后发生了变化" | ☐ |
| 1.5 | 卸载选「保留数据」,重装 | 画还在 | ☐ |
| 1.6 | 再画一笔,卸载选「连数据一起删」,重装。**卸载确认后立刻关掉设置窗口** | 画布空白(验证关窗也会清除,这是 Task 3 的 App 层接线唯一的验证途径) | ☐ |
| 1.7 | 另装一个会写 `localStorage.x=1` 的静态页 | 两个应用互相读不到对方的数据 | ☐ |
| 1.8 | 在 `manifest.toml` 加 `[permissions]` 下 `telepathy = "allow"` 再安装 | 安装流程里红字失败 | ☐ |
| 1.9 | 把 `[runtime]` 改成 `kind = "container"` 再安装 | 提示"暂不支持 container 类型的应用" | ☐ |
| 1.10 | 在 Excalidraw 里 `⌘C` 再 `⌘V` | 图形能复制粘贴 | ☐ |
| 1.11 | 点导出 PNG / 保存 | 不可用(下载被拒),这是预期 | ☐ |
| 1.12 | 切换主题,画英文字、中文字 | 手写体字形正常,中文按需加载正常 | ☐ |

## 2. 进程型应用(A6e)

对 `py-notes` 和 `node-notes` 各做一遍(结果列写 py / node)。

| # | 操作 | 预期 | 结果 |
|---|---|---|---|
| 2.1 | 安装 | 审批卡显示运行方式、命令行、版本要求,所有权限标"仅声明,不强制" | ☐ |
| 2.2 | 批准、启动、打开面板 | 页面出现,计数可点,SSE 在滚动,WebSocket 能回显 | ☐ |
| 2.3 | 点页面上的 crash | 面板短暂"启动中…",自动恢复 | ☐ |
| 2.4 | 点 hang | 约 40–45 秒后自动恢复 | ☐ |
| 2.5 | 装一份 manifest 里写 `python = ">=99"` 的副本 | 问题页,有「去设置安装」 | ☐ |
| 2.6 | 临时把 `node` 从 PATH 里去掉后启动 node 样例 | 提示"尚未安装";装好后点「重试」恢复 | ☐ |
| 2.7 | 把 `server.py` 改成启动即退出再重装 | Crashed 页,点「查看日志」能看到输出末尾 | ☐ |
| 2.8 | 卸载「保留数据」再装 | 计数还在 | ☐ |
| 2.9 | 卸载「含数据」再装 | 计数归零 | ☐ |
| 2.10 | 设置里装受管 Node(要联网),再启动 `node-notes` | 页面显示的 node 版本是受管版本 | ☐ |

## 3. 状态推送与日志(A6f)

| # | 操作 | 预期 | 结果 |
|---|---|---|---|
| 3.1 | 点 `/crash` | 面板在 1 秒内进入"启动中…",不是等 2 秒轮询 | ☐ |
| 3.2 | 另开终端只杀隔离的 dozerd:`HOME=/tmp/dzh pkill -9 -f "debug/dozerd"` | 2 秒内显示 dozerd 不可用;重启后自动恢复订阅 | ☐ |
| 3.3 | 设置 → 应用 → 点「日志」,另开终端往应用发请求让它打印 | 约 1 秒内出现新行 | ☐ |
| 3.4 | 在日志里上滚,再触发新行 | 不会被拉回底部 | ☐ |
| 3.5 | 关掉设置页 | 之后不再有读日志的请求(可看 dozerd 日志确认) | ☐ |
| 3.6 | 装一个 `package-lock.json` 引用不存在的包的 node 应用 | "依赖安装失败"问题页,动作是「查看日志」和「重试」,没有"去设置安装" | ☐ |

## 4. 升级与回滚(A6g)

**顺序必须是 4.1 → 4.2 → 4.3,不能先做自动回滚。** 回滚会消耗上一版;装 1.2.0 时 1.0.0 已被清理,先做自动回滚就没有上一版可以手动回滚了。

| # | 操作 | 预期 | 结果 |
|---|---|---|---|
| 4.1 | 装 `py-notes` 1.0.0,启动,点几下让计数增加。把样例版本改成 1.1.0、改一下页面文字,再装一次 | 审批卡显示"正在运行,会短暂中断";批准后面板自动显示新页面,计数还在 | ☐ |
| 4.2 | 设置里点「回滚到 1.0.0」 | 确认文案是"应用数据不会一起回滚,旧版本可能无法读取新版本写过的数据。确定回滚?";确认后面板回到 1.0.0 | ☐ |
| 4.3 | 装一个起不来的 1.2.0 | 数秒内设置页出现"已从 1.2.0 自动回滚到 …",面板回到可用版本 | ☐ |

## 5. 非本机来源(A6h)

| # | 操作 | 预期 | 结果 |
|---|---|---|---|
| 5.1 | 选本机目录安装一个应用(回归) | 仍正常 | ☐ |
| 5.2 | 把 `py-notes` 打成 zip 后安装并启动。用顶层目录形态:`cd <samples 的上一级> && zip -r py-notes.zip py-notes`;tar.gz 要加 `--no-mac-metadata` | 能装能跑 | ☐ |
| 5.3 | 填一个真实 https 的静态应用 zip 地址 | 审批卡显示来源、哈希、"来自网络,不可信";批准后能打开 | ☐ |
| 5.4 | 同一个地址故意填错 sha256 | 输入框内联拒绝,不弹 Toast,已填的 URL 和 sha256 保留 | ☐ |
| 5.5 | 填一个 Node 应用的 URL | 内联提示"网络来源只能安装静态应用" | ☐ |
| 5.6 | 在这些输入框里用中文输入法输入、粘贴 | 都正常 | ☐ |

5.3–5.5 需要一个真实的 https 静态服务器;没有的话先跳过并在结果列注明。

## 6. 应用 i18n(等 T7 发布真实 tag 后再做)

| # | 操作 | 预期 | 结果 |
|---|---|---|---|
| 6.1 | zh-CN 与 en 下查看应用标题、文案;缺译文;未知宿主语言 | 精确匹配、default、fallback 与页面拿到的 `app_locale` 一致 | ☐ |
| 6.2 | 用 `bilingual-live` 样例热切换语言 | 页面不刷新;输入框草稿、日志滚动、登录状态都保留 | ☐ |
| 6.3 | 开两个窗口、语言不同 | 互不影响 | ☐ |

## 已知会干扰验收的事项

- tar.gz 里的 `./` 条目和 macOS 的 AppleDouble(`._*`)伴随文件会让归档被拒(A6h 报告的已知局限),打包时按 5.2 的方式处理。
- A6e–A6h 报告里列的偶发测试失败与 GUI 验收无关,不要混淆。
- Excalidraw 的导出/保存在一期不可用(下载被拒),是预期行为,不是缺陷。
- `orca computer` 目前点击 Dozer 的 iced 界面不可靠,自动化只能做到截图确认。

## 填写结果

每项做完把"结果"列从 ☐ 改成 ✅ 或 ❌;失败的项在下面补一行现象。全部有结论后,回到五份验收报告更新勾选状态。

| 项 | 现象 |
|---|---|
|  |  |
