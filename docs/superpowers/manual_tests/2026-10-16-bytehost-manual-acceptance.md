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
| 1.3 | 退出整个 Dozer(dozerd 保持运行)再重开,进该应用 | 画还在 | ☐ Excalidraw 本体未做。**进程型应用的同一动作已用 `node-notes` 验证(2026-10-18):** `hits` 递增到 2 后 ⌘Q 退出整个 GUI,GUI 进程消失,而 dozerd、node 进程与网关端口 20027 仍在且网关继续应答(无令牌 403、伪造 Host 421);重新启动 GUI 后连上的仍是原 dozerd,node 进程未重启,`gateway.json` 端口不变,应用直接 Running,计数续上(`hits` 3,`counter.txt` 之后到 4) |
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
| 2.1 | 安装 | 审批卡显示运行方式、命令行、版本要求,所有权限标"仅声明,不强制" | py:⚠️ 运行方式/命令行/版本要求 ✅;权限标注未验(样例没声明权限,卡片写"不申请任何权限") |
| 2.2 | 批准、启动、打开面板 | 页面出现,计数可点,SSE 在滚动,WebSocket 能回显 | py:✅(WebSocket 原为 ❌,已修复,见"2.2 WebSocket"行) 页面、`hits`、运行时版本、SSE(tick 1–5)✅;**WebSocket ❌**(`ws error` / close 1006),见下表 ;node:✅(2026-10-18,隔离 `HOME=/tmp/dzh`,最新 `main`)经「本机目录」装 `node-notes` 并启动:标题按 i18n 显示"Node 笔记",`hits` 随刷新递增,`runtime: Node.js 24.14.0`(系统 node,非受管),SSE tick 1–5,`ws: hello` 回显(WebSocket 经网关正常),图标栏出现蓝色 "N" 首字母徽标;审批卡内容未逐项记录 |
| 2.3 | 点页面上的 crash | 面板短暂"启动中…",自动恢复 | py:✅ 命令触发 `/crash`;截图见"Py Notes / 启动中…",约 10 秒内回到 Running(自动恢复) |
| 2.4 | 点 hang | 约 40–45 秒后自动恢复 | py:✅ 命令触发 `/hang`;约 35 秒后状态 Starting、2 秒后 Running,页面重新加载(`hits` 3→4) |
| 2.5 | 装一份 manifest 里写 `python = ">=99"` 的副本 | 问题页,有「去设置安装」 | py:✅ 问题页显示"需要 Python / 要求 >=99,当前 3.13.1",按钮「去设置安装」「重试」;dozerd 状态 `Failed{需要 python3 >=99,当前 3.13.1, retryable:true}` |
| 2.6 | 临时把 `node` 从 PATH 里去掉后启动 node 样例 | 提示"尚未安装";装好后点「重试」恢复 | node:✅(2026-10-18)提示页原文:标题"Node Notes"、"需要 Node.js"、"尚未安装,可在 设置 → 应用 里一键安装",按钮「去设置安装」「重试」。把独立的 node(v24.14.0)放进 dozerd 的 PATH 目录后点「重试」,面板恢复运行,页面 `runtime: Node.js 24.14.0`、`hits: 2`,进程路径是新放入的那份(此前系统 node 是 26.11.1),说明重试确实重新解析了运行时。**注意构造方法:** 只改 `PATH` 藏不住 node——`resolve.rs` 总会把 `/opt/homebrew/bin`、`/usr/local/bin` 追加进搜索目录(GUI 拉起的 dozerd `PATH` 很短,这是有意的)。本次用 `sandbox-exec -p '(version 1)(allow default)(deny file* process-exec (literal <node 真实路径>)…)'` 在沙箱里启动隔离环境,不改动系统里的 node。 |
| 2.7 | 把 `server.py` 改成启动即退出再重装 | Crashed 页,点「查看日志」能看到输出末尾 | py:✅ 约 40 秒后出现 Crashed 页("应用已退出 / 健康检查没通过: 进程已经退出(重启次数过多,已放弃)",按钮「重新启动」「查看日志」);日志展开后 6 行输出(1 次首启 + 5 次重启);**但日志文字对比度极低**,见下表 |
| 2.8 | 卸载「保留数据」再装 | 计数还在 | py:✅ 卸载前计数 4,重装后 `counter.txt` 仍是 4,修改时间在卸载之前(未被重写) |
| 2.9 | 卸载「含数据」再装 | 计数归零 | py:✅ 卸载后 `data/` 目录消失;日志有"已清除应用的 WebView 数据存储";重装并启动后页面与 `counter.txt` 都是 `hits: 1` |
| 2.10 | 设置里装受管 Node(要联网),再启动 `node-notes` | 页面显示的 node 版本是受管版本 | ☐ |

## 3. 状态推送与日志(A6f)

| # | 操作 | 预期 | 结果 |
|---|---|---|---|
| 3.1 | 点 `/crash` | 面板在 1 秒内进入"启动中…",不是等 2 秒轮询 | ✅ 命令触发后 0.34 秒内的第一张截图就是"Py Notes / 启动中…"(截图间隔约 0.3 秒) |
| 3.2 | 另开终端只杀隔离的 dozerd:`HOME=/tmp/dzh pkill -9 -f "debug/dozerd"` | 2 秒内显示 dozerd 不可用;重启后自动恢复订阅 | ✅(原为 ❌,已修复,见末尾"已修复并复测")原先的现象:`kill -9` 后连续截图到 +25 秒,**界面任何地方都没有"dozerd 不可用"**(应用面板保持旧页面,终端显示"[会话已结束]",顶栏无徽标);GUI 日志只有每 2–4 秒一条 WARN"应用变更订阅失败,将退避重试"。重启 dozerd 后 WARN 立即停止(可推断已重新订阅),应用恢复 Running,但**日志里没有明确的"重订阅成功"一行**,面板仍是旧页面(`hits: 3`)未刷新。**[2026-10-17 已修复并复测]** bytehost `v0.4.6` 的状态机新增 `ConnectionLost`/`ConnectionRestored`,dozer 适配层映射成带 key 的 Toast(提交 dozer `d6800f3c`)。真 GUI 复测(隔离环境):`kill -9` dozerd 后约 **3 秒**出现 Warning Toast "Lost connection to dozerd, reconnecting…"(日志 08:17:12.6 kill → 08:17:15.6),此后重连又失败多次**不重复弹**;重启 dozerd 后约 3 秒出现 Success Toast "Reconnected to dozerd"。设置里**主动停止** dozerd(约 14 分钟、298 次订阅失败)期间**没有**新增"连接中断"。比报告原文的"2 秒"略慢,因为要等第二次重订阅失败才判定为真断线(单次抖动不提示)。Toast 是点击穿透的独立原生窗口,没有截到图,证据是日志行。 |
| 3.3 | 设置 → 应用 → 点「日志」,另开终端往应用发请求让它打印 | 约 1 秒内出现新行 | ✅ `py-notes` 对请求不打印,改用 `/crash` 触发重启产生新行 `listening on 65454`:日志文件 12:47:55.9 写入;约 1 秒后的截图里设置窗口仍是 3 行,约 2.6 秒后的截图是 4 行(延迟 1–2.6 秒,日志每秒整段重读,含截图约 0.3 秒滞后) |
| 3.4 | 在日志里上滚,再触发新行 | 不会被拉回底部 | ✅ 用自制 `py-chatty`(启动打 30 行、之后每 2 秒一行)。用户上滚后,相隔 5 秒的两张截图完全一致(都停在 `startup line 01–10`),期间日志文件从 36 行增到 46 行,位置未被拉回。第三张(+10 秒)位置变到底部附近,疑为用户又滚动,未确认 |
| 3.5 | 关掉设置页 | 之后不再有读日志的请求(可看 dozerd 日志确认) | ⚠️ GUI 层面未能观测:dozerd 对读日志请求不留任何日志(任何级别),文件 atime 也不随读取更新。代码层面:`bytehost-panel/src/logs.rs` 的 `tick` 只在展开着时才发读取、`hide` 后不再发,有单元测试覆盖 |
| 3.6 | 装一个 `package-lock.json` 引用不存在的包的 node 应用 | "依赖安装失败"问题页,动作是「查看日志」和「重试」,没有"去设置安装" | ✅ 自制 `node-badlock`(manifest 声明 `lockfile`,锁文件引用不存在的 `bytehost-no-such-package-zzz`)。dozerd 状态 `Failed{安装依赖失败: exit status: 1, retryable:true}`;问题页"依赖安装失败 / 安装依赖失败: exit status: 1 / 查看日志了解安装输出",按钮只有「查看日志」「重试」;日志展开后可见 `npm error code E404` 等完整输出 |

## 4. 升级与回滚(A6g)

**顺序必须是 4.1 → 4.2 → 4.3,不能先做自动回滚。** 回滚会消耗上一版;装 1.2.0 时 1.0.0 已被清理,先做自动回滚就没有上一版可以手动回滚了。

| # | 操作 | 预期 | 结果 |
|---|---|---|---|
| 4.1 | 装 `py-notes` 1.0.0,启动,点几下让计数增加。把样例版本改成 1.1.0、改一下页面文字,再装一次 | 审批卡显示"正在运行,会短暂中断";批准后面板自动显示新页面,计数还在 | ✅ 用户反馈符合预期(审批卡文案、面板切换、计数保留);后端 `Upgraded 1.0.0→1.1.0` 已核对,本样例为 `py-ver` |
| 4.2 | 设置里点「回滚到 1.0.0」 | 确认文案是"应用数据不会一起回滚,旧版本可能无法读取新版本写过的数据。确定回滚?";确认后面板回到 1.0.0 | ✅ 用户反馈符合预期(确认文案、面板回到 1.0.0);后端 `RolledBack 1.1.0→1.0.0`,原因"用户手动回滚"已核对 |
| 4.3 | 装一个起不来的 1.2.0 | 数秒内设置页出现"已从 1.2.0 自动回滚到 …",面板回到可用版本 | ✅ 用户反馈符合预期;后端 `RolledBack 1.2.0→1.1.0`、最终 Running、当前版本 1.1.0 已核对。**耗时约 32 秒**(1.2.0 先退避重启 5 次才放弃),清单写"数秒内" |

## 5. 非本机来源(A6h)

| # | 操作 | 预期 | 结果 |
|---|---|---|---|
| 5.1 | 选本机目录安装一个应用(回归) | 仍正常 | ✅ 本次验收所有应用(py-notes、py-ver、py-badver、py-crashy、py-chatty、node-badlock 等)均经「本机目录」安装,流程正常 |
| 5.2 | 把 `py-notes` 打成 zip 后安装并启动。用顶层目录形态:`cd <samples 的上一级> && zip -r py-notes.zip py-notes`;tar.gz 要加 `--no-mac-metadata` | 能装能跑 | ✅ 自制 `py-zip.zip`(顶层目录 `py-zip/`,`zip -X`)经「本机压缩包」安装。审批卡显示来源"本机压缩包"、位置、压缩包 SHA-256、大小 3.3 KiB、"顶层目录已剥掉 py-zip"、"来自压缩包,内容已解压校验";批准后在面板点「启动」,页面正常出现,`package/1.0.0/` 下直接是 `manifest.toml`、`server.py` |
| 5.3 | 填一个真实 https 的静态应用 zip 地址 | 审批卡显示来源、哈希、"来自网络,不可信";批准后能打开 | ☐ |
| 5.4 | 同一个地址故意填错 sha256 | 输入框内联拒绝,不弹 Toast,已填的 URL 和 sha256 保留 | ☐ |
| 5.5 | 填一个 Node 应用的 URL | 内联提示"网络来源只能安装静态应用" | ☐ |
| 5.6 | 在这些输入框里用中文输入法输入、粘贴 | 都正常 | ☐ |

5.3–5.5 需要一个真实的 https 静态服务器;没有的话先跳过并在结果列注明。

## 6. 应用 i18n(等 T7 发布真实 tag 后再做)

| # | 操作 | 预期 | 结果 |
|---|---|---|---|
| 6.1 | zh-CN 与 en 下查看应用标题、文案;缺译文;未知宿主语言 | 精确匹配、default、fallback 与页面拿到的 `app_locale` 一致 | ⚠️ zh-CN、en 两侧 ✅(宿主跟随系统=zh-CN;`bilingual-fix` 页面 `你好,世界!`、底部 `ui_locale=zh-CN app_locale=zh-CN revision=1 version=1`);en 一侧由 6.2 切换后看到(`你好,世界!`→`Hello, world!`);缺译文、未知宿主语言未验。注意:仓库样例 `bilingual-live` 本身跑不起来,见下表,本项用修复副本 `bilingual-fix` |
| 6.2 | 用 `bilingual-live` 样例热切换语言 | 页面不刷新;输入框草稿、日志滚动、登录状态都保留 | ✅ 用 `bilingual-fix` 把宿主语言从系统(zh-CN)切到 English:页面不刷新,标题/提示变英文,底部 `ui_locale=en app_locale=en revision=2`;用户输入的 `hello world` 保留;日志框停在第 53–60 行(未回到第 1 行,说明未重载;用户称滚到中间,位置是否被改动待确认);宿主界面本身也同步变英文(`Idle`、`Agent context · 0 items`)。页面 `revision=2` 与宿主 `locale_settings.json` 的 `revision: 3` 不一致,推测切换时多切了一次(未验证) |
| 6.3 | 开两个窗口、语言不同 | 互不影响 | ⏸ 未做:当前版本只有单个主窗口(分离窗口已推迟),没法开两个窗口看显示代次隔离 |

## 已知会干扰验收的事项

- tar.gz 里的 `./` 条目和 macOS 的 AppleDouble(`._*`)伴随文件会让归档被拒(A6h 报告的已知局限),打包时按 5.2 的方式处理。
- A6e–A6h 报告里列的偶发测试失败与 GUI 验收无关,不要混淆。
- Excalidraw 的导出/保存在一期不可用(下载被拒),是预期行为,不是缺陷。
- `orca computer` 目前点击 Dozer 的 iced 界面不可靠,自动化只能做到截图确认。

## 填写结果

每项做完把"结果"列从 ☐ 改成 ✅ 或 ❌;失败的项在下面补一行现象。全部有结论后,回到五份验收报告更新勾选状态。

| 项 | 现象 |
|---|---|
| 2.2 WebSocket(py-notes) | 经网关的 `/ws` 在 webview 里握手失败(`error`、close 1006)。网关放行、应用也回了 101、`Sec-WebSocket-Accept` 正确,命令行直连握手与回显都成功。网关转发的 101 响应头是 `connection: Upgrade, close`(应用直连是 `Upgrade`)。同一 webview 里 A/B:服务端回 `Connection: Upgrade` 能 open 并收到消息,回 `Connection: Upgrade, close` 失败(1006)。**很可能的根因(未打补丁实测)**:`bytehost-apps/src/gateway/mod.rs` 的 `http1::Builder::keep_alive(false)` 让 hyper 追加 `close`。A6e 报告的"WebSocket 同源升级成功"是用裸客户端测的,没经 WKWebView,所以没发现 **[2026-10-17 已修复:bytehost `14763a7`(`v0.4.5`)去掉网关的全局 `keep_alive(false)`,改为对非 101 响应显式 `Connection: close`;真 GUI 复测页面出现 `ws: hello` ✅]** |
| 观察:安装后入口不易发现 | 已安装应用在图标栏里是通用列表图标,放在左栏最下面,没有名称/应用图标区分;设置页的应用行没有「启动」按钮,启动靠点开面板 **[2026-10-17 图标区分度已修复:已安装应用在图标栏里画首字母徽标(byteui `v0.4.5` 的 `glyph_button_entry`,dozer `a1d59dc4`),底色由应用 id 哈希决定、字符取当前语言下的标题首字;「设置页没有启动按钮」未改]** |
| 观察:审批卡"来源"重复 | 出现三处"来源"(`本地目录` / 空的 `来源:` 小标题 / `本机目录`),且"本地目录"与"本机目录"措辞不一致 **[2026-10-17 见下面"缺陷:审批卡来源标签混乱"一行,已修复]** |
| 观察:同版本重复安装 | 提示"安装失败:版本 1.0.0 已经安装",文案清楚 |
| 验收环境 | `agent-desktop` 在 PTY 里被拒(权限授给了 App 而非责任进程 `dozerd`),改用 `orca computer`:截图可用,辅助功能树只有窗口外壳,坐标点击只触发悬停 |
| 观察:hang 期间界面无提示 | `/hang` 后约 35 秒内状态一直是 Running,面板继续显示崩溃前的页面(不响应),没有任何"无响应"提示 **[2026-10-17 已裁决:有意不改。要显示就得新增"无响应"状态并追加 wire、改 supervisor 与面板,收益小]** |
| 撤回:crash 后页面未重载 | **这条观察是我看错/记错了,不是缺陷。** 2026-10-17 复测(临时给适配层加了 `ClearUrl`/`SetUrl`/`ListLoaded` 日志,已还原):对运行中的应用 `/crash` → `Running → Starting → ClearUrl → Running → SetUrl`,计数 `1→2`、`2→3`,页面 `hits` 与计数一致,`tick`/`ws` 重新出现;`/hang` 同样的 `ClearUrl → SetUrl`,计数 `3→4`。两条路径都会重载页面。当初记"`hits: 3` 未变"时,我在第一轮手动 `curl -X POST /hit` 改过计数,屏幕上的数字和文件里的数不是我以为的那样 |
| 观察:卸载重装后图标位置变化 | `py-notes` 卸载重装后被追加到图标栏末尾,从第 8 个变成第 9 个,不同应用图标又相同(通用列表图标),验收时就点错过一次 **[2026-10-17 图标现在是首字母徽标,不会再点错;重装后追加到栏末的位置行为未改]** |
| 观察:Crashed 页要等约 40 秒 | 启动即崩的应用按 1/2/4/8/16 秒退避重启 5 次才放弃,期间面板一直只显示"启动中…",看不出在反复崩溃重启 **[2026-10-17 已裁决:有意不改。退避 1/2/4/8/16 秒重启 5 次是 supervisor 的设计,只作为已知行为记录]** |
| 缺陷:应用日志文字没跟主题走,深色主题下几乎看不见 | 深色主题下日志文字(深色)压在深色背景上难以辨认;用户的浅色主题截图里同一处是黑字、清晰可读。代码层面:面板日志 `dozer-app/src/app/view.rs:1450` 与设置页日志 `extensions/settings_apps_view.rs:367` 的 `iced_widget::text(...)` 都没有 `.color(...)`,用的是 iced 默认文字色;同一处的标签文字都显式设了 `.color(dim)`。应改为取主题色 **[2026-10-17 已修复:dozer `79737edd`;深色主题下日志文字是奶油黄,真 GUI 复测清晰]** |
| 观察:措辞 | 后端状态原因写 `python3`,问题页标题写 `Python` |
| 说明:§4 的界面文案来源 | 4.1–4.3 的审批卡/确认/回滚提示文案是用户肉眼确认后口头反馈,我没有逐步截图;状态变化、版本和计数来自 dozerd 事件日志与 `state.json`、`counter.txt`。用的是自制样例 `py-ver`(1.0.0/1.1.0/1.2.0),不是 `py-notes` |
| 3.2 的原因与处置(已裁决:补 Toast) | 订阅断开时状态机原先只回退到快速轮询并退避重订阅,不写 `App.daemon_unavailable`(按项目约定只有"启动连不上、设置里停止、打开项目成功即清除"三处写入),所以运行中被 `kill -9` 界面静默;状态机模块文档"顶栏的 dozerd 徽标负责告知"的假设不成立。处置:状态机发中立效果,dozer 映射成 Toast;"dozerd 断着"既是事件也是状态,也可映射成顶栏徽标,换映射只改 `host_adapters/app_host.rs::connection_toast`。计划:`docs/superpowers/plans/2026-10-17-dozerd-disconnect-hint.md` |
| 发现:重订阅退避实际从不拉长 | 复测时 dozerd 停了 14 分钟,GUI 每 2–4 秒重连一次共 298 次。原因:`subscribe_if_due` 一发起重订阅就把状态从 `Backoff` 改成 `Pending`,下一次失败时 `attempt` 又从 0 算起,`RESUBSCRIBE_BACKOFF` 的 2/5/15/30 秒档位在真实流程里只会用到第 0 档(现有测试因连续喂 `SubscriptionLost` 而没经过 `subscribe_if_due`,没抓到)。未修 **[2026-10-17 已修复:bytehost `b3baed4`(`v0.4.7`);`Pending` 现在带着上一次的档位]** |
| 发现:两个依赖仓库状态/并行的偶发测试 | ① bytehost `service::tests::the_first_run_is_unavailable_when_the_port_cannot_be_saved`:断言端口关闭后可重新绑定,并行跑时临时端口会被别的测试抢到,整仓 3 次里失败 1 次,单独跑 15/15 通过;② dozer `extensions::git_log::tests::build_marks_head_branch_and_labels`:读 dozer 自己的 git 仓库并期望历史第 0 行带当前分支,当 HEAD 落后于仓库里最新提交时(例如在落后于 `main` 的 worktree 里)稳定失败。均与功能无关,未修 **[2026-10-17 ① 端口释放那个已修:bytehost `fccc5f7`(整套并行 20 遍失败 1 次 → 修后约 100 遍 0 次);② `git_log` 那个已修:dozer `21e21cca`(在落后的 worktree 里改前稳定失败、改后通过)。**复测中又发现两个同类偶发(未修):** `service::tests::start_uses_the_persisted_port_across_restarts`(同样是先挑空闲端口再绑定的竞态,40 遍失败 2 次)与 `manager::tests::a_python_app_installs_starts_runs_behind_the_gateway_and_stops`(事件顺序断言,约 1/40)]** |
| 缺陷:设置页应用行的按钮被裁切 | 设置窗口里 `Py Crashy`、`Py Notes` 两行右侧按钮被窗口右边缘裁掉(只露出「卸载」的一小截红色),`Py BadVer` 一行完整。行内文字或按钮较多时没有换行/收缩 **[2026-10-17 已修复:dozer `75fc7d46`,按钮放进可换行的第二行,真 GUI 复测所有按钮完整可见]** |
| 观察:清单"dozerd 日志确认"不可行 | dozerd 对 `Logs` 请求没有任何日志,A6f 报告和本清单 3.5 里"看 dozerd 日志确认"的验证手段不存在;若要保留这条验收,需给该请求加 debug 级日志 **[2026-10-17 已处理:bytehost `1c6ce71`(`v0.4.5`)给 `Logs` 请求加了 debug 级日志(`RUST_LOG=bytehost::service=debug`),GUI 层面的"关页后不再读"本次未再复验]** |
| 观察:4.3 提示文案已见 | 设置页 `Py Ver` 行下方显示"已从 1.2.0 自动回滚到 1.1.0:新版本 1.2.0 启动失败,已自动回滚"(4.3 的预期文案,截图可见) |
| 缺陷:设置页按钮布局挤压 | `Py WS Debug` 的「回滚到 1.0.0」按钮被挤成三行(`回滚 / 到 / 1.0.0`);与前述按钮被窗口边缘裁切是同一类问题:应用行没有按可用宽度换行/收缩 **[2026-10-17 已修复:同上 `75fc7d46`;运行时区域的「安装…」被挤成两行是同类问题,dozer `10797923` 修复,真 GUI 复测单行显示]** |
| 说明:本机 npm 使用镜像 | 本机 npm 配的是 `registry.npmmirror.com`,验证 `npm ci` 失败时实际请求走镜像;应用内安装器的 404 输出里是 `registry.npmjs.org` |
| 缺陷:审批卡"来源"标签混乱 | 压缩包的审批卡上先有一行`来源 本地目录`,下面又有`来源: / 来源 本机压缩包 / 位置 …`。第一行是来源渠道(Provenance=本地),第二行才是具体来源类型,两者都叫"来源",且对压缩包来说第一行的"本地目录"是错的/误导的;之前记的"来源重复"应改为这个解释 **[2026-10-17 已修复:bytehost `c097aa2`(`v0.4.5`),渠道一行改为"来源渠道 本机"]** |
| 观察:已安装未启动应用的入口 | 打开面板显示"应用未运行 / 启动"按钮,启动入口在面板里,设置页的应用行没有「启动」按钮(只有停止/回滚/日志/卸载) |
| 缺陷(样例):`bilingual-live` 在宿主里跑不起来 | 样例 `scripts/samples/i18n/bilingual-live/web/index.html` 第 42 行用内联 `<script>`,而网关给静态应用的 CSP 是 `script-src 'self' 'wasm-unsafe-eval'`(不允许内联脚本),脚本被拦,页面停在占位状态(标题 `…`、提示/底部为空、60 行日志也没生成)。把脚本挪到 `web/app.js` 并改成 `<script src=\"app.js\">`(副本 `bilingual-fix`)后立即正常。按项目规则不得放宽 CSP,应修样例。`partial-initial` 很可能有同样问题(未验) **[2026-10-17 已修复:bytehost `98599c4`(`v0.4.5`),两个 i18n 样例的脚本移到外部 `app.js`,并新增"静态样例不得有内联可执行脚本"的门禁测试;真 GUI 复测原版样例直接渲染]** |
