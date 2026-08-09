# Footbar 系统信息条 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `App::view` 根 `column!` 末尾追加第三行 `footbar`,常驻显示 CPU/RAM/SSD/HDD/Proxy/下行/上行 7 段系统信息。CPU/RAM/网速 1s 刷新一次,代理/磁盘 300s 刷新一次。新增 `extensions::footbar` 模块,App 级状态(不挂 `Workspace`,跨项目页签共享)。格式照用户样张:`｜CPU  18%｜RAM  43%｜SSD  90%｜HDD  50%｜Proxy  127.0.0.1:7890｜↓ 12.4 MB/s  ↑ 1.8 MB/s｜`。

**Architecture:** 新文件 `crates/dozer-app/src/extensions/footbar.rs`,镜像 `extensions::usage` 的 `WorkspaceState`/`Message`/`update`/`view`/`spawn_refresh` 五件套形状,但状态挂在 `App`(不是 `Workspace`)——同 `home_browser`/`git_log`/`todo`/`database` 的 App 级状态 precedent,因为系统信息跨所有项目页签共享。一个长生命周期 tokio 任务(`footbar::spawn_sampler`)在 `App::new_shell` 阶段 fire-and-forget 启动,内部 `tokio::time::interval(1s)` 自管节奏,每 tick 用 `spawn_blocking` 调 `sysinfo` 采样 + 整包发回 `Message::Footbar(Message::Sampled(sample))`;代理/磁盘走"距上次慢节奏采样时间差 ≥ 300s"的内部计时,合并进同一条 `Sampled` 消息。**不走 winit `ControlFlow::WaitUntil`** 周期唤醒——sysinfo 阻塞 ~200ms,放 winit 主循环会拖卡 UI;`proxy.send_event` 落地 iced 消息会自动 `request_redraw`,不需要 winit 唤醒协同。视觉口径复用 `theme::region::status_bar()` + `theme::geometry::status_bar_height()`(26px),不新增主题令牌。

**Tech Stack:** Rust workspace;iced 0.14;新增 `sysinfo = "0.32"`(与 `crates/legacy-boy/Cargo.toml:21` 同版本,workspace lockfile 已热,但 `legacy-boy` 禁止扩展,只在 `dozer-app` 自己声明)。

## Global Constraints

- 这是 footbar v1,**不含 GPU 数据**(brainstorming 会话 2026-08-09 决议:
  sysinfo 不提供 GPU;`powermetrics`/`ioreg` 路径需要进程 spawn + plist 解析,
  复杂度跳一档;开发场景下 GPU 价值低于 CPU/RAM。后续若需要单独一轮设计)。
- 状态挂在 `App.footbar: footbar::AppState`,**不挂 `Workspace`**——系统信息跨
  所有项目页签共享,挂 `Workspace` 会导致每个页签独立采样、串项目时数据漂移。
- `footbar::spawn_sampler` 是 fire-and-forget——`proxy.send_event` 内含
  `EventLoopProxy`(`Arc` 内含),runtime drop 时任务自然取消,不需要 `JoinHandle`
  管控生命周期,也不需要在 `App::drop` 里 abort。
- 网速口径是**全接口聚合 + 虚拟接口 denylist**(`lo`/`lo0`/`docker*`/`bridge*`/
  `veth*`/`br-*`),不是裸全聚合、也不是只看默认网卡(brainstorming 决议 2)。
- CPU 口径是**整机平均**(`sysinfo::System::global_cpu_usage()`),不是单核
  (brainstorming 决议 3)。
- footbar **无交互、无总开关**(brainstorming 决议 1),永远常驻在窗口底部。
- 视觉口径**复用 `status_bar` 主题令牌**,不新增 `theme::region::footbar()` /
  `theme::geometry::footbar_height()`——同一类"细条"不分两个令牌。
- 不动任何 `Workspace` 字段、不动任何 `Message::Files`/`Message::Project`/
  `Message::ProjectFsChanged` 等已落地路由,与 `extensions::project` 试点改同一个
  `workspace.rs` 但**字段/消息不重叠**,合并冲突风险低(主要可能冲突在 `App::view`
  的 `column!` 那一行与 `App` struct 字段块)。
- 每个任务结束都要 `cargo build -p dozer-app && cargo test -p dozer-app &&
  cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过(Task 5 是内核
  接线的中间态,允许编译报错,见该任务说明)。
- 设计文档:`docs/superpowers/specs/2026-08-09-footbar-system-info-design.md`
  (有疑问以它为准;设计文档里的 sysinfo API 签名已核对过 0.32.1 实际签名)。

---

### Task 1: 加 `sysinfo` 依赖

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`

**Interfaces:**
- Produces:`sysinfo = "0.32"` 进入 `dozer-app` 的依赖图。

- [ ] **Step 1: 加依赖行**

`crates/dozer-app/Cargo.toml` 的 `[dependencies]` 节末尾(`url = "2"` 之后)加:

```toml
# sysinfo = footbar 系统信息条(CPU/RAM/网络/磁盘)的统一数据源。
# 版本与 crates/legacy-boy 一致,workspace Cargo.lock 已热,不引入新版本解析。
sysinfo = "0.32"
```

- [ ] **Step 2: 编译确认 lockfile 不变**

```bash
cargo build -p dozer-app
```

Expected: 编译通过。`Cargo.lock` 不应该新增条目(`sysinfo 0.32` 已被
`legacy-boy` 引入,workspace lockfile 已含);若 `Cargo.lock` 有变化,确认是
features 解析的微调(不应升级到 0.32 之外的版本)。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/Cargo.toml Cargo.lock
git commit -m "chore(dozer-app): add sysinfo 0.32 for footbar system info"
```

---

### Task 2: `extensions::footbar` 骨架——类型 + `update` + 纯函数辅助

**Files:**
- Create: `crates/dozer-app/src/extensions/footbar.rs`
- Modify: `crates/dozer-app/src/extensions.rs`(加 `pub mod footbar;`)

**Interfaces:**
- Produces:`pub struct AppState`、`pub struct Sample`、`pub enum Message`、
  `pub fn update(&mut AppState, Message)`、`fn format_speed(f64) -> String`、
  `fn is_virtual_interface(&str) -> bool`、`fn aggregate_disks(&[(PathBuf, u64, u64, bool)]) -> (f32, Option<f32>)`。

- [ ] **Step 1: 写失败的测试**

创建 `crates/dozer-app/src/extensions/footbar.rs`,先只放测试骨架:

```rust
//! 底部 footbar 系统信息条:CPU/RAM/SSD/HDD/Proxy/下行/上行 7 段常驻显示。
//! App 级状态(挂 `App.footbar`,不挂 `Workspace`——跨所有项目页签共享)。
//! 设计见 `docs/superpowers/specs/2026-08-09-footbar-system-info-design.md`。
#![cfg(test)]

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_speed_below_1kb_is_zero() {
        assert_eq!(format_speed(0.0), "0.0 KB/s");
        assert_eq!(format_speed(1023.9), "0.0 KB/s");
    }

    #[test]
    fn format_speed_kb_range() {
        assert_eq!(format_speed(1024.0), "1.0 KB/s");
        assert_eq!(format_speed(1_500.0), "1.5 KB/s");
    }

    #[test]
    fn format_speed_mb_range() {
        assert_eq!(format_speed(1_048_576.0), "1.0 MB/s");
        assert_eq!(format_speed(1_500_000.0), "1.4 MB/s");
    }

    #[test]
    fn format_speed_gb_range() {
        assert_eq!(format_speed(1_073_741_824.0), "1.0 GB/s");
        assert_eq!(format_speed(1.5e9), "1.4 GB/s");
    }

    #[test]
    fn is_virtual_interface_denylist() {
        assert!(is_virtual_interface("lo"));
        assert!(is_virtual_interface("lo0"));
        assert!(is_virtual_interface("docker0"));
        assert!(is_virtual_interface("dockerbridge1"));
        assert!(is_virtual_interface("bridge0"));
        assert!(is_virtual_interface("vethabc123"));
        assert!(is_virtual_interface("br-abc123"));
        // VPN 隧道不算虚拟接口(真实流量,见 brainstorming 决议 2)。
        assert!(!is_virtual_interface("utun0"));
        assert!(!is_virtual_interface("ipsec0"));
        // 物理网卡。
        assert!(!is_virtual_interface("en0"));
        assert!(!is_virtual_interface("eth0"));
        assert!(!is_virtual_interface("wlan0"));
    }

    #[test]
    fn aggregate_disks_boot_only_no_hdd() {
        let disks = vec![(
            std::path::PathBuf::from("/"),
            500_000_000_000,
            50_000_000_000,
            false,
        )];
        let (ssd, hdd) = aggregate_disks(&disks);
        assert_eq!(ssd, 90.0);
        assert_eq!(hdd, None);
    }

    #[test]
    fn aggregate_disks_boot_plus_extra() {
        let disks = vec![
            ("/", 500_000_000_000, 50_000_000_000, false),       // 90% SSD
            ("/Volumes/Ext", 1_000_000_000_000, 500_000_000_000, false), // 50% HDD
        ];
        let (ssd, hdd) = aggregate_disks(
            &disks
                .iter()
                .map(|(p, t, a, r)| (p.into(), *t, *a, *r))
                .collect::<Vec<_>>(),
        );
        assert_eq!(ssd, 90.0);
        assert_eq!(hdd, Some(50.0));
    }

    #[test]
    fn aggregate_disks_readonly_extra_excluded() {
        let disks = vec![
            ("/", 500_000_000_000, 50_000_000_000, false),
            ("/Volumes/Readonly", 1_000_000_000_000, 500_000_000_000, true),
        ];
        let (ssd, hdd) = aggregate_disks(
            &disks
                .iter()
                .map(|(p, t, a, r)| (p.into(), *t, *a, *r))
                .collect::<Vec<_>>(),
        );
        assert_eq!(ssd, 90.0);
        assert_eq!(hdd, None);
    }

    #[test]
    fn aggregate_disks_no_disks() {
        let (ssd, hdd) = aggregate_disks(&[]);
        assert_eq!(ssd, 0.0);
        assert_eq!(hdd, None);
    }

    #[test]
    fn update_sampled_stores_sample() {
        let mut state = AppState::default();
        let sample = Sample {
            cpu_percent: 18.0,
            ram_percent: 43.0,
            ssd_percent: 90.0,
            hdd_percent: Some(50.0),
            proxy: Some("127.0.0.1:7890".into()),
            net_down_bps: 12_000_000.0,
            net_up_bps: 1_800_000.0,
        };
        update(&mut state, Message::Sampled(sample.clone()));
        assert_eq!(state.sample.cpu_percent, 18.0);
        assert_eq!(state.sample.net_down_bps, 12_000_000.0);
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p dozer-app extensions::footbar::
```

Expected: 编译失败(所有引用的类型/函数未定义)。

- [ ] **Step 3: 实现类型与纯函数**

在文件顶部 `#![cfg(test)]` 之前加(把测试 `mod tests` 移到文件末尾):

```rust
//! 底部 footbar 系统信息条:CPU/RAM/SSD/HDD/Proxy/下行/上行 7 段常驻显示。
//! App 级状态(挂 `App.footbar`,不挂 `Workspace`——跨所有项目页签共享)。
//! 设计见 `docs/superpowers/specs/2026-08-09-footbar-system-info-design.md`。

use std::path::PathBuf;

/// App 级系统信息条状态——挂在 `App.footbar`,跨所有项目页签共享。
#[derive(Default)]
pub struct AppState {
    sample: Sample,
}

impl AppState {
    pub fn sample(&self) -> &Sample {
        &self.sample
    }
}

/// 一次完整采样的结果。CPU/RAM/网络是 1s 节奏,代理/硬盘是 300s 节奏
/// (代理在两次 300s 之间复用上次结果)。网络速度已是除以 elapsed 之后
/// 的最终 bytes/sec,UI 不再算差分。
#[derive(Clone, Default)]
pub struct Sample {
    /// 整机平均 CPU 百分比,0.0..=100.0。sysinfo
    /// `System::global_cpu_usage()` 的直接输出(brainstorming 决议 3)。
    pub cpu_percent: f32,
    /// RAM 占用百分比,0.0..=100.0。
    pub ram_percent: f32,
    /// 启动盘(`mount_point()=="/"`)的占用百分比。
    pub ssd_percent: f32,
    /// 其余可写盘聚合占用百分比;None = 无额外盘,该段折叠不渲染。
    pub hdd_percent: Option<f32>,
    /// `"host:port"`;None = 未启用代理,显示 "—"。
    pub proxy: Option<String>,
    /// 下行 bytes/sec。
    pub net_down_bps: f64,
    /// 上行 bytes/sec。
    pub net_up_bps: f64,
}

/// 顶层 `Message::Footbar(footbar::Message::...)` 的载荷。本模块是 App 级 +
/// 纯展示,没有用户交互消息,只有一个 `Sampled` 变体由 `spawn_sampler`
/// 周期性发回。
#[derive(Debug, Clone)]
pub enum Message {
    Sampled(Sample),
}

/// 处理消息——本模块不接触终端会话域、不写盘、不 `handle`/`emit`,
/// 全部同步完成。
pub fn update(state: &mut AppState, msg: Message) {
    match msg {
        Message::Sampled(s) => state.sample = s,
    }
}

/// 网速按量级自动选单位,1 位小数。`<1 KB/s` → `0.0 KB/s`(向下取整,
/// 避免 idle 时显示 `0.04 KB/s` 这种噪音)。
fn format_speed(bps: f64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    if bps < KB {
        "0.0 KB/s".to_string()
    } else if bps < MB {
        format!("{:.1} KB/s", bps / KB)
    } else if bps < GB {
        format!("{:.1} MB/s", bps / MB)
    } else {
        format!("{:.1} GB/s", bps / GB)
    }
}

/// 网络接口 denylist 模式匹配(brainstorming 决议 2)。`lo`/`lo0`/
/// `docker*`/`bridge*`/`veth*`/`br-*` 排除;`utun*`/`ipsec*` 不排除
/// (VPN 是真实流量)。
fn is_virtual_interface(name: &str) -> bool {
    const DENYLIST: &[&str] = &["lo", "lo0", "docker", "bridge", "veth", "br-"];
    DENYLIST.iter().any(|p| name.starts_with(p))
}

/// 磁盘占用聚合。输入切片是 `(mount_point, total_space, available_space,
/// is_read_only)`——从 `sysinfo::Disks` 适配成这个形状(测试时直接构造
/// 切片,不必 mock sysinfo 类型)。
///
/// 返回 `(ssd_%, hdd%)`:
/// - ssd = `mount_point=="/"` 的盘占用百分比;无启动盘 → 0.0。
/// - hdd = 其余可写盘的"已用之和 / 容量之和"百分比;无可写非启动盘 → None。
fn aggregate_disks(disks: &[(PathBuf, u64, u64, bool)]) -> (f32, Option<f32>) {
    let mut ssd_used = 0u64;
    let mut ssd_total = 0u64;
    let mut hdd_used = 0u64;
    let mut hdd_total = 0u64;
    for (mp, total, avail, ro) in disks {
        let used = total.saturating_sub(*avail);
        if mp == std::path::Path::new("/") {
            ssd_used += used;
            ssd_total += total;
        } else if !ro {
            hdd_used += used;
            hdd_total += total;
        }
    }
    let ssd = if ssd_total > 0 {
        (ssd_used as f64 / ssd_total as f64 * 100.0) as f32
    } else {
        0.0
    };
    let hdd = if hdd_total > 0 {
        Some((hdd_used as f64 / hdd_total as f64 * 100.0) as f32)
    } else {
        None
    };
    (ssd, hdd)
}
```

- [ ] **Step 4: 声明模块**

`crates/dozer-app/src/extensions.rs`,按字母序插入(在 `files` 之后、`git_log`
之前):

```rust
pub mod acceptance;
pub mod browser;
pub mod files;
pub mod footbar;
pub mod git_log;
pub mod project;
pub mod ssh;
pub mod todo;
pub mod usage;
```

- [ ] **Step 5: 跑测试确认通过**

```bash
cargo test -p dozer-app extensions::footbar::
```

Expected: 全部新增测试 PASS。

- [ ] **Step 6: `cargo clippy`/`fmt`**

```bash
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/extensions/footbar.rs crates/dozer-app/src/extensions.rs
git commit -m "feat(dozer-app): add footbar AppState/Sample/Message/update + pure helpers"
```

---

### Task 3: `extensions::footbar::view`——7 段渲染

**Files:**
- Modify: `crates/dozer-app/src/extensions/footbar.rs`

**Interfaces:**
- Consumes:Task 2 的 `AppState`/`Sample`。
- Produces:`pub fn view<'a>(state: &'a AppState) -> Element<'a, Message,
  iced_widget::Theme, iced_widget::Renderer>`。

- [ ] **Step 1: 文件顶部补齐 import**

把 Task 2 的 `use std::path::PathBuf;` 一行替换成:

```rust
use crate::theme;
use iced_widget::container;
use iced_widget::core::{container::Style, Border, Element, Length};
use iced_widget::{row, text};
use std::path::PathBuf;
```

- [ ] **Step 2: 加 `view` 函数**

在 `pub fn update` 之后加:

```rust
/// footbar 主入口。被 `App::view` 在根 `column!` 末尾调用,`.map(Message::Footbar)`
/// 转成顶层消息。无项目依赖、无交互——纯展示条,样式直接用
/// `theme::region::status_bar()` + `theme::geometry::status_bar_height()`
/// (视觉口径与 in-pane status_bar 一致,不新增主题令牌)。
pub fn view<'a>(
    state: &'a AppState,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = theme::region::status_bar();
    let base = region.border.unwrap_or_default();
    let s = &state.sample;

    let mut segs: Vec<String> = Vec::new();
    segs.push(format!("CPU  {:.0}%", s.cpu_percent));
    segs.push(format!("RAM  {:.0}%", s.ram_percent));
    segs.push(format!("SSD  {:.0}%", s.ssd_percent));
    if let Some(h) = s.hdd_percent {
        segs.push(format!("HDD  {:.0}%", h));
    }
    segs.push(format!(
        "Proxy  {}",
        s.proxy.as_deref().unwrap_or("—")
    ));
    segs.push(format!(
        "↓ {}  ↑ {}",
        format_speed(s.net_down_bps),
        format_speed(s.net_up_bps)
    ));
    let body = segs.join("｜");

    // 用户样张首尾也带 `｜`,这里照做。
    let content = row![
        text("｜").size(theme::font::caption()).color(theme::color::DIM),
        text(body).size(theme::font::caption()).color(theme::color::BODY),
        text("｜").size(theme::font::caption()).color(theme::color::DIM),
    ]
    .spacing(0)
    .align_y(iced_widget::core::Alignment::Center);

    container(content)
        .width(Length::Fill)
        .height(Length::Fixed(theme::geometry::status_bar_height()))
        .padding(region.padding)
        .style(move |_t: &iced_widget::Theme| Style {
            background: region.background.map(Into::into),
            border: Border {
                color: base.color,
                width: base.width,
                radius: base.radius,
            },
            ..Style::default()
        })
        .into()
}
```

- [ ] **Step 3: 编译,逐条修正**

```bash
cargo build -p dozer-app
```

Expected: 编译通过。`view` 目前未被内核调用,`dead_code` 警告可接受,不允许报错。
逐条修正类型/字段名不一致的地方(如 `theme::color`/`theme::font`/`theme::region`
/`theme::geometry` 的具体常量名要跟 `crates/dozer-app/src/theme` 模块实际导出的对上;
`theme::color::BODY`/`theme::color::DIM`/`theme::font::caption()`/
`theme::region::status_bar()`/`theme::geometry::status_bar_height()` 都已存在,
见 `theme/color.rs:16-17`/`theme/font.rs`/`theme/region.rs:345`/
`theme/geometry.rs:166`)。

- [ ] **Step 4: `cargo clippy`/`fmt`**

```bash
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/footbar.rs
git commit -m "feat(dozer-app): add footbar::view rendering 7 segments"
```

---

### Task 4: `detect_proxy` + `spawn_sampler`——异步采样任务

**Files:**
- Modify: `crates/dozer-app/src/extensions/footbar.rs`

**Interfaces:**
- Consumes:Task 1 的 `sysinfo 0.32`,Task 2 的 `Sample`/`Message`/`is_virtual_interface`/
  `aggregate_disks`,以及 `crate::workspace::ShellIo`(`workspace.rs:1264-1271`)。
- Produces:`pub fn spawn_sampler(io: &ShellIo)`、`fn detect_proxy() -> Option<String>`、
  `fn compute_disk_usage(disks: &sysinfo::Disks) -> (f32, Option<f32>)`。

- [ ] **Step 1: 文件顶部补齐 import**

在 Task 3 已加的 import 之上加:

```rust
use crate::workspace::ShellIo;
use std::collections::HashMap;
use std::time::{Duration, Instant};
```

- [ ] **Step 2: 写 `detect_proxy` 的测试**

`mod tests` 里加(单测只覆盖 env 变量路径,`scutil` 留给人工验收):

```rust
#[test]
fn detect_proxy_strips_http_scheme() {
    std::env::set_var("ALL_PROXY", "http://127.0.0.1:7890");
    std::env::remove_var("HTTPS_PROXY");
    std::env::remove_var("HTTP_PROXY");
    std::env::remove_var("https_proxy");
    std::env::remove_var("http_proxy");
    let p = detect_proxy_from_env();
    assert_eq!(p.as_deref(), Some("127.0.0.1:7890"));
    std::env::remove_var("ALL_PROXY");
}

#[test]
fn detect_proxy_no_scheme_passthrough() {
    std::env::set_var("HTTPS_PROXY", "127.0.0.1:7890");
    std::env::remove_var("ALL_PROXY");
    std::env::remove_var("HTTP_PROXY");
    std::env::remove_var("https_proxy");
    std::env::remove_var("http_proxy");
    let p = detect_proxy_from_env();
    assert_eq!(p.as_deref(), Some("127.0.0.1:7890"));
    std::env::remove_var("HTTPS_PROXY");
}

#[test]
fn detect_proxy_lowercase_var_also_works() {
    std::env::set_var("http_proxy", "127.0.0.1:8080");
    std::env::remove_var("ALL_PROXY");
    std::env::remove_var("HTTPS_PROXY");
    std::env::remove_var("https_proxy");
    std::env::remove_var("HTTP_PROXY");
    let p = detect_proxy_from_env();
    assert_eq!(p.as_deref(), Some("127.0.0.1:8080"));
    std::env::remove_var("http_proxy");
}

#[test]
fn detect_proxy_none_when_no_env() {
    std::env::remove_var("ALL_PROXY");
    std::env::remove_var("HTTPS_PROXY");
    std::env::remove_var("HTTP_PROXY");
    std::env::remove_var("https_proxy");
    std::env::remove_var("http_proxy");
    assert_eq!(detect_proxy_from_env(), None);
}
```

(测试串改 env 不是线程安全的,但 `dozer-app` 的 `cargo test` 默认单线程跑
`#[test]`,与 `extensions/files.rs`/`extensions/usage.rs` 现有 env 测试口径一致。)

- [ ] **Step 3: 跑测试确认失败**

```bash
cargo test -p dozer-app extensions::footbar::tests::detect_proxy
```

Expected: 编译失败(`detect_proxy_from_env` 未定义)。

- [ ] **Step 4: 实现 `detect_proxy` + `detect_proxy_from_env`**

在 `aggregate_disks` 之后加:

```rust
/// 代理检测:env(`ALL_PROXY`/`HTTPS_PROXY`/`HTTP_PROXY` 及小写变体)→
/// macOS `scutil --proxy` 解析 → 都没有返回 None。env 值剥掉
/// `http://`/`https://`/`socks5://` 等 scheme 前缀,只留 `host:port`。
fn detect_proxy() -> Option<String> {
    if let Some(p) = detect_proxy_from_env() {
        return Some(p);
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(p) = detect_proxy_via_scutil() {
            return Some(p);
        }
    }
    None
}

/// env 变量路径——单测覆盖(不开进程,不依赖系统)。优先级:
/// `ALL_PROXY` > `HTTPS_PROXY` > `HTTP_PROXY`,大小写变体都查。
fn detect_proxy_from_env() -> Option<String> {
    for name in [
        "ALL_PROXY",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "all_proxy",
        "https_proxy",
        "http_proxy",
    ] {
        if let Ok(v) = std::env::var(name) {
            let v = v.trim();
            if v.is_empty() {
                continue;
            }
            return Some(strip_proxy_scheme(v));
        }
    }
    None
}

/// 剥掉 `http://`/`https://`/`socks5://`/`socks5h://` 等 scheme 前缀。
fn strip_proxy_scheme(s: &str) -> String {
    let s = s.trim();
    for scheme in ["http://", "https://", "socks5://", "socks5h://", "socks4://"] {
        if let Some(rest) = s.strip_prefix(scheme) {
            return rest.trim_end_matches('/').to_string();
        }
    }
    // 兼容 `socks5://` 大小写。
    for scheme in ["HTTP://", "HTTPS://", "SOCKS5://", "SOCKS5H://", "SOCKS4://"] {
        if let Some(rest) = s.strip_prefix(scheme) {
            return rest.trim_end_matches('/').to_string();
        }
    }
    s.to_string()
}

/// macOS `scutil --proxy` 解析:找 `HTTPEnable: 1` 后取 `HTTPProxy:` 与
/// `HTTPPort:`,组装 `host:port`。无 enable 或无 host/port → None。
#[cfg(target_os = "macos")]
fn detect_proxy_via_scutil() -> Option<String> {
    let out = std::process::Command::new("scutil")
        .arg("--proxy")
        .output()
        .ok()?;
    let txt = String::from_utf8_lossy(&out.stdout);
    let mut enable = false;
    let mut host: Option<String> = None;
    let mut port: Option<String> = None;
    for line in txt.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("HTTPEnable : ") {
            enable = v == "1";
        } else if let Some(v) = line.strip_prefix("HTTPProxy : ") {
            host = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("HTTPPort : ") {
            port = Some(v.to_string());
        }
    }
    if enable {
        if let (Some(h), Some(p)) = (host, port) {
            return Some(format!("{h}:{p}"));
        }
    }
    None
}
```

(把 `detect_proxy_from_env` 拆成独立函数让单测能直接调它,绕过 `scutil` 的
进程依赖。)

- [ ] **Step 5: 跑测试确认通过**

```bash
cargo test -p dozer-app extensions::footbar::tests::detect_proxy
```

Expected: 4 个新测试 PASS。

- [ ] **Step 6: 实现 `compute_disk_usage` + `spawn_sampler`**

在 `detect_proxy_via_scutil` 之后加:

```rust
/// 把 `sysinfo::Disks` 适配成 `aggregate_disks` 需要的切片再调它。
/// 分离这一层让 `aggregate_disks` 可以纯单测覆盖(不必 mock `sysinfo` 类型)。
fn compute_disk_usage(disks: &sysinfo::Disks) -> (f32, Option<f32>) {
    let slice: Vec<(PathBuf, u64, u64, bool)> = disks
        .iter()
        .map(|d| {
            (
                d.mount_point().to_path_buf(),
                d.total_space(),
                d.available_space(),
                d.is_read_only(),
            )
        })
        .collect();
    aggregate_disks(&slice)
}

/// 在 `App::new_shell` 阶段调一次,fire-and-forget。runtime drop 时任务随
/// `tokio::Runtime` 一起取消。内部管两个节奏:1s 的 CPU/RAM/网络,
/// 300s 的代理/磁盘——靠"距上次慢节奏采样的时间差"判断,不另起 interval
/// (避免快慢两个 interval tick 漂移导致同秒双发)。
pub fn spawn_sampler(io: &ShellIo) {
    let proxy = io.proxy.clone();
    io.handle.spawn(async move {
        let mut sys = sysinfo::System::new();
        let mut nets = sysinfo::Networks::new_with_refreshed_list();
        let mut disks = sysinfo::Disks::new_with_refreshed_list();
        // 代理缓存:启动时立刻测一次,之后 300s 复用。
        let mut cached_proxy: Option<String> = detect_proxy();
        let mut last_proxy_check = Instant::now();
        // CPU 首次 refresh——sysinfo 需要一次 baseline 才能
        // `global_cpu_usage()` 出值。
        sys.refresh_cpu_usage();
        tokio::time::sleep(Duration::from_millis(500)).await;

        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.tick().await; // 跳过首次立即触发,与上面的 500ms 一起避免冷启 0%。
        let mut last_ts = Instant::now();
        loop {
            tick.tick().await;
            // 1s 节奏
            sys.refresh_cpu_usage();
            sys.refresh_memory();
            nets.refresh();
            let now = Instant::now();
            let elapsed = (now - last_ts).as_secs_f64().max(0.001);
            last_ts = now;
            // 收到 `&Networks` 引用时直接读每接口的 `received()`/`transmitted()`
            // ——sysinfo 自管 baseline,这两个方法返回"自上次 refresh 以来的增量"。
            let mut down: u64 = 0;
            let mut up: u64 = 0;
            for (name, net) in &nets {
                if is_virtual_interface(name) {
                    continue;
                }
                down += net.received();
                up += net.transmitted();
            }
            let cpu = sys.global_cpu_usage();
            let ram = (sys.used_memory() as f64 / sys.total_memory().max(1) as f64) * 100.0;
            // 300s 节奏(代理/磁盘)
            if now.duration_since(last_proxy_check).as_secs() >= 300 {
                cached_proxy = detect_proxy();
                last_proxy_check = now;
                disks.refresh();
            }
            let (ssd, hdd) = compute_disk_usage(&disks);
            let sample = Sample {
                cpu_percent: cpu,
                ram_percent: ram as f32,
                ssd_percent: ssd,
                hdd_percent: hdd,
                proxy: cached_proxy.clone(),
                net_down_bps: down as f64 / elapsed,
                net_up_bps: up as f64 / elapsed,
            };
            let _ = proxy.send_event(Message::Footbar(Message::Sampled(sample)));
        }
    });
}
```

- [ ] **Step 7: 编译,逐条修正**

```bash
cargo build -p dozer-app
```

Expected: 编译通过。`spawn_sampler`/`detect_proxy`/`compute_disk_usage` 目前
未被内核调用,`dead_code` 警告可接受,不允许报错。逐条修正 `sysinfo` API
签名不一致(若 0.32 在某些方法上有微调)——核对方法名:

- `sysinfo::System::new()` ✅
- `sysinfo::System::refresh_cpu_usage(&mut self)` ✅
- `sysinfo::System::refresh_memory(&mut self)` ✅
- `sysinfo::System::global_cpu_usage(&self) -> f32` ✅
- `sysinfo::System::used_memory(&self) -> u64` ✅
- `sysinfo::System::total_memory(&self) -> u64` ✅
- `sysinfo::Networks::new_with_refreshed_list() -> Self` ✅
- `sysinfo::Networks::refresh(&mut self)` ✅
- `&Networks` 迭代 `(&String, &NetworkData)` ✅
- `NetworkData::received(&self) -> u64` ✅
- `NetworkData::transmitted(&self) -> u64` ✅
- `sysinfo::Disks::new_with_refreshed_list() -> Self` ✅
- `sysinfo::Disks::refresh(&mut self)` ✅
- `Disks` 迭代 `&Disk`(via `Deref<Target=[Disk]>`)✅
- `Disk::mount_point(&self) -> &Path` ✅
- `Disk::total_space(&self) -> u64` ✅
- `Disk::available_space(&self) -> u64` ✅
- `Disk::is_read_only(&self) -> bool` ✅

- [ ] **Step 8: 跑测试 + clippy + fmt**

```bash
cargo test -p dozer-app extensions::footbar::
cargo clippy -p dozer-app --all-targets
cargo fmt
```

Expected: 全绿。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/extensions/footbar.rs
git commit -m "feat(dozer-app): add footbar detect_proxy + spawn_sampler long-lived task"
```

---

### Task 5: 内核接线——App 字段 / Message 变体 / view column(允许中间态编译失败)

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes:Task 2 的 `footbar::AppState`,Task 3 的 `footbar::view`,Task 4 的
  `footbar::spawn_sampler`/`footbar::Message`/`footbar::update`。

#### Part A:`App` struct 字段 + `App::new_shell` 初始化

- [ ] **Step 1: `use` 引入**

`workspace.rs` 顶部按字母序加(在 `extensions::files`/`extensions::git_log`
之间,看现有实际位置):

```rust
use crate::extensions::footbar;
```

- [ ] **Step 2: `App` struct 加字段**

`pub struct App { .. }`(现约 1276-1373 行)末尾加(在 `database: database::AppState,`
之后):

```rust
/// footbar 系统信息条 App 级状态——跨所有项目页签共享(见
/// `extensions::footbar::AppState`)。
footbar: footbar::AppState,
```

- [ ] **Step 3: `App::new_shell` 初始化 + 启动采样任务**

`App::new_shell`(现约 2671-2715 行)末尾的 `Self { .. }` 字段块里,在
`database: database::AppState::default(),` 之后加:

```rust
footbar: footbar::AppState::default(),
```

紧接 `App::new_shell` 返回之前(或在 `App::bootstrap` 第一次拿到 `ShellIo`
之后——具体取决于 `new_shell` 是否已经持有 `Handle`/`EventLoopProxy`;
若 `new_shell` 不方便拿到 `&ShellIo`,改成在 `App::update` 的**第一次调用**
里启动,见 Step 3b):

```rust
let io = self.shell_io();
footbar::spawn_sampler(&io);
```

- [ ] **Step 3b: 备选——`App::update` 首次调用时启动采样**

如果 `new_shell` 拿不到 `ShellIo` 或不方便 spawn,改成在 `App::update` 顶部
加"首次启动"守卫(需要一个标志字段):

```rust
pub struct App {
    // ...
    footbar: footbar::AppState,
    footbar_sampler_started: bool,
}
```

`App::update` 顶部(`fn update(&mut self, msg: Message, io: &ShellIo)` 一进
函数体)加:

```rust
if !std::mem::replace(&mut self.footbar_sampler_started, true) {
    footbar::spawn_sampler(io);
}
```

(选 Step 3 还是 3b 取决于 `new_shell` 的实际签名;若 `new_shell` 已经
收 `&ShellIo` 参数则 Step 3 更干净,若不收则 3b。看代码实际状态再选。)

#### Part B:顶层 `Message` 枚举 + `App::update` 路由

- [ ] **Step 4: `Message` 枚举新增变体**

在 `Message::Files(files::Message),`(现约 1054 行)之后加(与
`Message::Project`/`Message::Todo` 等并列):

```rust
/// Footbar 系统信息条的消息,内核只转发不解读——见 `extensions::footbar::Message`。
Footbar(footbar::Message),
```

- [ ] **Step 5: `App::update` 新增路由分支**

在 `Message::Files(msg) => { .. }` 兜底分支(或 `Message::Project(msg)` 分支,
看现有 routing 顺序)之后加:

```rust
Message::Footbar(msg) => {
    footbar::update(&mut self.footbar, msg);
}
```

(footbar 是 App 级 + 纯展示,不需要 `with_project`/`with_focused_project`/`emit`,
路由比其它 extension 更简单。)

#### Part C:`App::view` 根 column 加第三行

- [ ] **Step 6: `App::view` 末尾追加 footbar**

`App::view`(现约 4614-4623 行)改:

```rust
pub fn view(&self) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let top = top_bar(self);
    let body = self.page_content(self.current_page, self.active_project_id);
    let foot = footbar::view(&self.footbar).map(Message::Footbar);
    let base = column![top, body, foot];
    self.apply_overlays(base.into())
}
```

`column!` 默认子项 `Length::Fill` 高度,`top_bar`/`footbar::view` 各自容器内
显式 `.height(Length::Fixed(..))` 钉死,`body` 自然占中间剩余空间。

- [ ] **Step 7: 编译,逐条修正**

```bash
cargo build -p dozer-app
```

Expected: 应该编译通过(若 Tasks 2-4 都已完成)。若报错,基本是字段名/
import 残留——逐条按上面 Step 1-6 的模式修正。**不允许有"缺 match 分支"
类的错误**(footbar 没有 `LeftView`/`RailButton` 穷举 match 需要补分支,
不像 `extensions::project` 试点)。

- [ ] **Step 8: 全量测试 + clippy + fmt**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt
```

Expected: 全绿。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): wire footbar into App struct, Message enum, and view column"
```

---

### Task 6: 全量校验与人工验收

**Files:** 无代码改动。

- [ ] **Step 1: 全 workspace 构建 + 测试 + clippy + fmt**

```bash
cargo build
cargo test -p dozer-app
cargo clippy --all-targets
cargo fmt --check
```

Expected: 全绿。

- [ ] **Step 2: 人工验收(`cargo run -p dozer-app`)**

对照设计文档逐项走一遍:

- 窗口最底部出现一条 footbar,高度与 in-pane status bar 一致(26px,scale 后),
  背景同 `status_bar` 令牌(`#0a0e16`),顶部有 1px `BORDER` 边框。
- footbar 渲染 7 段:`CPU %`/`RAM %`/`SSD %`/`HDD %`/`Proxy`/`↓ 速度`/`↑ 速度`,
  段间全宽竖线 `｜`,标签与值之间两空格。
- CPU/RAM/网速每秒刷新一次(数值在变,能看出实时性)。
- 代理段:设置了 `HTTPS_PROXY` env 变量时显示 `host:port`;没设时显示 `—`。
- 网速段:看视频/下载文件时数值明显上升,idle 时显示 `0.0 KB/s`。
- HDD 段:无外置盘时折叠不显示(整机只有 SSD 一块盘时只看到 6 段);
  插上外置盘等 5min 内(300s 节奏)出现 HDD 段。
- 启动 `dozer` app 后头 1s footbar 显示 `CPU 0%`(sysinfo 冷启 baseline
  等待),~1.5s 后第一条真实数据落地,数值变成实际值。
- 切换项目页签:footbar 不重建、不闪烁,数值继续按原节奏刷新(系统信息
  跨项目共享,不串项目)。
- 窗口缩放:footbar 高度恒定 26px(scale 后),宽度填满窗口宽度。
- `⌘+`/`⌘-` 字号缩放:footbar 文字(11px caption)同比例放大/缩小,
  与顶栏/状态条字号缩放同步。
- 关闭 app 后 `ps aux | grep dozer` 确认没有残留采样进程(runtime drop 时
  `spawn_sampler` 任务应该被取消)。

- [ ] **Step 3: macOS 代理段专项验收**

- 在终端 `export HTTPS_PROXY=http://127.0.0.1:7890` 然后启动 `dozer`:
  footbar 代理段应立即显示 `127.0.0.1:7890`(env 路径)。
- 在系统设置里配置 HTTP 代理(系统设置 → 网络 → 高级 → 代理),不设 env
  启动 `dozer`:footbar 代理段应在首次启动或 5min 内显示 `host:port`
  (`scutil --proxy` 路径)。
- 关掉系统代理:5min 内(300s 节奏)footbar 代理段变成 `—`。

- [ ] **Step 4: 确认没有遗留未提交的改动**

```bash
git status
```

Expected: 干净,无未跟踪/未暂存改动(`.dozer/` 目录等项目运行时产物除外)。
