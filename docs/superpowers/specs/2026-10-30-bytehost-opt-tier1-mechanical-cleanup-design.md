# bytehost 优化 Tier1：低风险机械改动 规格

> 日期：2026-10-30。状态：草案，尚未实施。实施计划见 [plan](../plans/2026-10-30-bytehost-opt-tier1-mechanical-cleanup.md)。
> 本梯队是全面分析后识别出的最低风险机械改动，不涉及行为变更或架构调整。

## 1. 目标与边界

消除 dozer 侧 bytehost 依赖管理的分散性、清理未使用依赖与死代码；修正 bytehost 文档漂移；解决 `SurfaceEvent` 同名歧义。

**不在本梯队**：结构性重构（Tier3）、去重收敛（Tier2）、性能优化（Tier4）、跨仓治理（Tier5）。

## 2. 已核对的代码基线

| 位置 | 当前事实 | 改动责任 |
| --- | --- | --- |
| dozer 13 个 crate 的 `Cargo.toml`（约 25 处） | bytehost tag v0.4.17 分散声明；dozer-app 一家 8 处 | dozer：上收至根 `[workspace.dependencies]` |
| `spike/v2-excalidraw/Cargo.toml:12`、`spike/browser-probe/Cargo.toml:21` | 分别钉 v0.2.0、v0.4.16 旧 tag | dozer：清理或注明独立 workspace 不受影响 |
| `crates/dozer-panel-files/Cargo.toml:18-19` | 声明 bytehost-core/desktop，src 中零直接引用 | dozer：删除未用依赖 |
| `crates/dozer-panel-project/Cargo.toml:17` | 声明 bytehost-desktop，src 中零直接引用 | dozer：删除未用依赖 |
| `crates/dozer-app/src/app/layout_compat.rs` | 整模块 `#![allow(dead_code)]`，注释自承未接线 | dozer：删除模块及 mod 声明 |
| bytehost `README.md` | 写"七个 crate"，实为 9 个（缺 settings/settings-ui） | bytehost：修正数字与表格 |
| `crates/bytehost-panel/src/lib.rs:2`、`crates/bytehost-webview/src/lib.rs:2` | 仍称"Tauri 宿主（Digger）"，Tauri 方案已撤回 | bytehost：更新注释 |
| `crates/bytehost-desktop/src/shell.rs:18` vs `crates/bytehost-core/src/surface.rs:111` | 两个 `SurfaceEvent` 同名不同形，均在 crate 根导出 | bytehost：重命名 desktop 侧为 `HostSurfaceEvent` |

## 3. 方案

### 3.1 dozer bytehost tag 上收 workspace

在 dozer 根 `Cargo.toml` 的 `[workspace.dependencies]` 统一声明 9 个 bytehost crate（tag v0.4.17），各消费 crate 改为 `.workspace = true`。参考 digger 根 Cargo.toml 写法。spike 属独立 workspace，其 Cargo.toml 不受主 workspace 管控；旧 tag 单独清理或加注说明。

### 3.2 删除 dozer 未用依赖

`dozer-panel-files` 与 `dozer-panel-project` 声明的 bytehost-core/desktop 在 src 中无直接 use（均经 dozer-panel-host 间接消费）。删除后跑 `cargo check` 确认编译通过。

### 3.3 删除 layout_compat.rs

该模块为布局迁移桥，建了但未接入启动/落盘路径。删除文件及 `app/mod.rs` 中的 `mod layout_compat;` 声明。

### 3.4 bytehost 文档修正

README crate 数量与表格补全；panel/webview lib.rs 顶部注释去除 Tauri 字样，改为反映当前 iced+wry 事实。

### 3.5 SurfaceEvent 重命名（破坏性）

`desktop::shell::SurfaceEvent` → `HostSurfaceEvent`。需 bytehost 发版 + dozer/digger 同日升级消费点。用 Grep 确认所有引用处并同步更新。

## 4. 兼容与发布

- 3.1–3.4 为 dozer/bytehost 内部改动，不涉及 wire 或公开 API 破坏（3.4 仅文档）。
- 3.5 是破坏性重命名：bytehost 发新 tag → dozer/digger 同日升级 → 验证编译与测试。发版与推送先问用户。

## 5. 验收与完成定义

| ID | 场景 | 必须观察到的结果 |
| --- | --- | --- |
| T1-01 | dozer 全仓 grep bytehost tag | 仅根 Cargo.toml 出现 v0.4.17；spike 旧 tag 已清理或注明 |
| T1-02 | `cargo build` dozer 全仓 | 编译通过，无 unused dependency 警告 |
| T1-03 | grep layout_compat | 无残留引用；`cargo test -p dozer-app` 全绿 |
| T1-04 | bytehost README + lib.rs 注释 | crate 数=9；无 Tauri 字样 |
| T1-05 | dozer/digger 升级新 tag 后 `cargo test --all-features` | 全绿；glob import 不再歧义 |
