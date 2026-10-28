# Dozer 浏览器面板暴露面:复现结果(计划 Task 0)

日期:2026-10-28。计划:`docs/superpowers/plans/2026-10-28-dozer-browser-untrusted-webview.md`。复现页:`spike/dozer-browser-exposure/`。

**方法:** 真实的 Dozer 开发构建(临时把首页全局浏览器的初始网址指向复现页;改动没有提交),`HOME` 指向临时目录隔离数据与 daemon,Orca 截图读页面日志。页面自动运行,不需要点击(Orca 的合成点击进不了 iced 控件)。

## 结果

| # | 项 | 结果 | 依据 |
|---|----|------|------|
| A | 页面里有 `window.ipc` | **能**(`object`) | 截图 |
| C | 注入脚本留下的全局(`window.__dozer_webview`) | **能**(值为 0) | 截图 |
| D | `fetch('dozer://review-trace/data.json', {mode:'no-cors'})` | **到达协议处理器**(返回 opaque 响应) | 截图 |
| D2 | `<img src="dozer://…">` | 触发 `onerror`(协议有应答,但不是图片) | 截图 |
| E | 页面把自己**顶层导航**到 `dozer://review-trace/data.json` | **能**:地址栏变成 `dozer://review-trace/data.json`,页面是协议处理器的应答 `not found`(隔离环境里没有审阅快照,所以是 404;有快照时回显快照) | 截图(改动前) |
| B | 页面经 `window.ipc.postMessage('title:…')` 改页签标题 | **没观察到**:页签标题仍是 `file:`。页面发的 id 是 0,不一定对得上这个页签的 id,**没有换 id 再试,不要据此认为不可行** | 截图 |

## 没测、也不能下结论的

- 页面能否**读到**"已允许文件"(`dozer://…/__file__<编码路径>`)或审阅快照的内容:隔离环境里这两样都是空的。顶层导航到 `dozer://` 之后页面成了宿主来源,但那一页是协议处理器的应答,攻击者的脚本不在里面运行;跨来源 `fetch` 没有 CORS 头,读不到响应。所以**目前没有证据显示能读出数据**,只证明了"能到达处理器"。
- `window.ipc` 能发的消息范围:读代码(`classify_ipc`)是 `focus` / `mouseup` / `zoom_*` / `find_*` / `paste_request`(目前在 `update.rs` 里是空操作)/ `title:`;`Host` 事件需要页面绑定,浏览器页面造不出。影响面小,但没有逐条实测。
- 下载:这个分支里没有下载处理器(只有应用 webview 显式拒绝),按 wry 默认应放行,**没有实测**。

## 严重度判断(我的判断,不是实测)

比最初担心的低:没有证据显示页面能读文件或审阅数据。但"不可信页面和宿主共用同一个构建器"本身就是错的方向——页面能到达 `dozer://`、能看到 IPC 和注入脚本,任何一次这些东西的改动(新增命名空间、新增 IPC 消息、`dozer://` 应答带上 CORS 头)都可能把它变成真漏洞。止血 + 迁移都值得做,但不是紧急事故。

## 止血补丁(已做,分支 `fix/browser-nav-allowlist`)

浏览器分支加了导航白名单(`bytehost_webview::browse::allows_browse_navigation`)、下载拒绝、`target=_blank` 地址过白名单(`allows_new_tab`)。**真机复验**:改动后复现页的 E 项写出 `E2 3 秒后仍在本页 => 导航被拒`,地址栏保持 `file://…`;A / C / D / D2 仍然可达(预期——补丁只堵导航,其余靠整体迁移到 `web_child`)。钉死测试 `the_browser_webview_branch_gates_navigation_downloads_and_new_windows`,3 个变异(导航恒真、去掉下载拒绝、新窗口不过白名单)均变红。
