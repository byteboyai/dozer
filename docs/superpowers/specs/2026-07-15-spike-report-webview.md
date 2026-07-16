# Spike 报告：iced × wry WebView 合成（P1a Task 4/5）

日期：2026-07-16 · 环境：macOS Darwin 25.5.0 / Apple Silicon / iced 0.14.0 / iced_winit 0.14.0 / iced_wgpu 0.14.0 / iced_widget 0.14.2 / wry 0.55.1 / winit 0.30.13

## Spike A（winit+wry 子视图）

| # | 验收项 | 结果 | 备注 |
|---|--------|------|------|
| 1 | 子视图区域渲染 | ✓ | |
| 2 | resize 跟随 | ✓ | |
| 3 | 隐藏/显示 | ✓ | |
| 4 | 滚动/点击 | ✓ | |
| 5 | 中文 IME | ✓ | |
| 6 | 焦点隔离 | ✓ | |

## Spike B（iced_winit 集成 + wry）

| # | 验收项 | 结果 | 备注 |
|---|--------|------|------|
| 1 | 同窗共存 | ✓ | |
| 2 | ToggleGate 隐藏恢复 | ✓ | |
| 3 | resize 跟随 | ✓ | |
| 4 | 隐藏露出 wgpu 背景 | ✓ | |
| 5 | 焦点切换无死区 | ✓ | |
| 6 | 空闲 CPU 正常 | ✓ | |

## 裁决

- [x] **GO**：dozer-app 采用 iced_winit 自持 event loop + iced_wgpu + wry 子视图；⌘K 隐藏预览方案成立。

## 移交 P1c/P1d 的约束

1. wry 统一 0.55.1；`WebViewBuilder::new().with_url().with_bounds().build_as_child(&window)` 用法已验证。
2. ToggleGate 类副作用须在拥有 window/webview 句柄的消息分发执行环执行（纯视图层拿不到句柄）。
3. Spike 的 `preview_bounds` 覆盖 35%–75% 宽度区间（brief 公式如此，模拟中偏右 pane），P1d 按真实左二布局重算。
4. 官方 integration 示例的 futures 依赖需 `features = ["thread-pool"]` 才能编译（`executor` 特性）。
