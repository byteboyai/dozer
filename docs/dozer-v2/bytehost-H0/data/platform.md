<!-- bytehost H0 机械报表快照;基线提交 e2ed23d3;生成命令: python3 scripts/audit/report.py platform -->

| 文件 | 引用的 extension | 引用的根/层模块(前5) | 归属建议(机械判断) |
|---|---|---|---|
| `platform/confirm_overlay.rs` | database×2, files×2, ssh×2, todo×2 | workspace×2, app×1 | 混合,需人工裁决 |
| `platform/database_drivers_overlay.rs` | database×1 | app×2 | 面板专属:database |
| `platform/database_source_overlay.rs` | database×1 | app×2, chrome×2, workspace×1 | 面板专属:database |
| `platform/edit_history_overlay.rs` | edit_history×1 | preview×10, app×3, assets×2 | 面板专属:edit_history |
| `platform/file_drag.rs` |  |  | 通用(候选 host) |
| `platform/file_history_overlay.rs` | diff_content×2, file_history×1 | preview×8, app×3, assets×2 | 混合,需人工裁决 |
| `platform/files_move_overlay.rs` | files×1 | app×2, chrome×2 | 面板专属:files |
| `platform/overlay_focus.rs` |  |  | 通用(候选 host) |
| `platform/overlay_gpu.rs` |  |  | 通用(候选 host) |
| `platform/overlay_window.rs` |  | theme×1 | 通用(候选 host) |
| `platform/picker.rs` |  |  | 通用(候选 host) |
| `platform/project_create_overlay.rs` | project_create×1 | app×2, chrome×2 | 面板专属:project_create |
| `platform/project_delete_overlay.rs` | project×1 | app×2 | 面板专属:project |
| `platform/project_scaffold_overlay.rs` | project×1 | app×2 | 面板专属:project |
| `platform/search_overlay.rs` | search×1 | runtime×3, app×2, chrome×2, event×1 | 面板专属:search |
| `platform/settings_overlay.rs` | project_create×1, settings×1 | app×2, chrome×2 | 混合,需人工裁决 |
| `platform/ssh_host_overlay.rs` | ssh×1 | app×2, chrome×2, workspace×1 | 面板专属:ssh |
| `platform/toast_overlay.rs` | toast×2 |  | 面板专属:toast |
| `platform/todo_detail_overlay.rs` | todo×1 | app×2, chrome×2 | 面板专属:todo |
| `platform/window.rs` |  |  | 通用(候选 host) |
| `platform/window_events.rs` | files×12, browser×6, project×6, project_create×4, todo×4, search×3, group_chat×1 | runtime×29, app×27, preview×10, event×4, chrome×3 | 混合,需人工裁决 |
