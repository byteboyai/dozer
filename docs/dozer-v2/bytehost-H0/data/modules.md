<!-- bytehost H0 机械报表快照;基线提交 e2ed23d3;生成命令: python3 scripts/audit/report.py modules -->

| 模块 | 扇入单元数 | 扇入 | 扇出(模块) | 依赖 iced/byteui | 公开项 | 行数 |
|---|---|---|---|---|---|---|
| `theme` | 20 | ext:agent_context, ext:browser, ext:conversations, ext:database, ext:files, ext:footbar, ext:git_log, ext:group_chat, ext:settings, ext:ssh, ext:todo, app, chrome, platform, preview, term, theme, workspace, main, webview_geometry |  | 否 | 3 | 98 |
| `assets` | 5 | platform, preview, term, osc, runtime | preview | 否 | 4 | 1392 |
| `project` | 5 | ext:files, ext:ssh, app, platform, workspace |  | 否 | 20 | 856 |
| `capabilities` | 4 | app, preview, workspace, runtime |  | 否 | 9 | 301 |
| `conversation` | 4 | ext:conversations, ext:usage, app, chrome |  | 否 | 4 | 132 |
| `project_meta` | 4 | ext:project, ext:project_create, app, workspace |  | 否 | 2 | 73 |
| `runtime` | 4 | ext:settings, app, platform, main | app, assets, capabilities, extensions, preview | 是 | 6 | 772 |
| `menu_spec` | 3 | ext:files, chrome, workspace | chrome | 是 | 7 | 200 |
| `external_apps` | 2 | ext:files, app |  | 否 | 6 | 216 |
| `git_accounts` | 2 | ext:project_create, ext:settings |  | 否 | 18 | 388 |
| `layout` | 2 | app, panel_layouts | app, chrome | 否 | 3 | 147 |
| `secrets` | 2 | ext:database, ext:ssh | extensions | 否 | 8 | 196 |
| `transcript` | 2 | app, workspace |  | 否 | 4 | 828 |
| `webview_geometry` | 2 | app, platform | app, chrome, theme, workspace | 否 | 8 | 2055 |
| `event` | 1 | platform |  | 是 | 3 | 73 |
| `frosted` | 1 | chrome |  | 是 | 1 | 35 |
| `keymap` | 1 | platform |  | 否 | 3 | 294 |
| `open_projects` | 1 | app |  | 否 | 3 | 72 |
| `osc` | 1 | workspace | assets | 否 | 4 | 170 |
| `panel_layouts` | 1 | app | app, layout | 否 | 3 | 203 |
| `preview_state` | 1 | workspace | preview | 否 | 6 | 295 |
