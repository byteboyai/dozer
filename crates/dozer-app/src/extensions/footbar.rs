//! 底部 footbar 系统信息条:CPU/RAM/SSD/HDD/下行/上行/Proxy 常驻显示,
//! 末段呈现为 `图标 网速 | Proxy`:有代理时为 `Proxy {addr}`,无代理时
//! 为 `Proxy OFF`(不再隐藏整段)。
//! App 级状态(挂 `App.footbar`,不挂 `Workspace`——跨所有项目页签共享)。
//! 设计见 `docs/superpowers/specs/2026-08-09-footbar-system-info-design.md`。

use byteui::interaction::icons;
use crate::theme;
use crate::theme::icon_size;
use crate::workspace::ShellIo;
use iced_widget::core::{Alignment, Element, Length};
use iced_widget::{Space, container, row, text};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// App 级系统信息条状态——挂在 `App.footbar`,跨所有项目页签共享。
#[derive(Default)]
pub struct AppState {
    sample: Sample,
}

/// 一次完整采样的结果。CPU/RAM/网络是 1s 节奏,代理/硬盘是 300s 节奏
/// (代理在两次 300s 之间复用上次结果)。网络速度已是除以 elapsed 之后
/// 的最终 bytes/sec,UI 不再算差分。
#[derive(Debug, Clone, Default)]
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

/// footbar 主入口。被 `App::view` 在根 `column!` 末尾调用,`.map(Message::Footbar)`
/// 转成顶层消息。无项目依赖、无交互——纯展示条,外层容器背景/对齐沿用
/// `theme::region::status_bar()`,但字号用更小的 `caption_sm()`、高度用
/// 独立的 `theme::geometry::footbar_height()`(比 in-pane status_bar 更矮更紧凑,
/// 不与 `status_bar_height()` 共用,避免改 footbar 时连坐 in-pane status bar)。
pub fn view(state: &AppState) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::status_bar();
    let s = &state.sample;

    // 各段:CPU/RAM/SSD/HDD 是数值段(用量 >75% 时数值变红,见 `metric_row`),
    // 代理段与网速段纯展示。最终呈现为 `… HDD ｜ [图标] Proxy ｜ 网速`:
    // 代理段以图标作前导(替代原先的 `Proxy` 文字标签),排在网速段之前;
    // 网速段前用 `｜` 分隔,排在代理段之后。
    /// 每段前导分隔:`None`=无,`Pipe`=`｜`,`Icon`=指定图标(默认
    /// square-radical,可覆盖)。
    #[derive(Clone, Copy)]
    enum Lead {
        None,
        Pipe,
        Icon,
    }

    let mut segs: Vec<(
        Lead,
        Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>,
    )> = Vec::new();
    segs.push((Lead::None, metric_row("CPU", s.cpu_percent)));
    segs.push((Lead::Pipe, metric_row("RAM", s.ram_percent)));
    // SSD/HDD 不存在时折叠不显示(SSD=0.0 表示无启动盘,HDD=None 表示无额外盘)。
    if s.ssd_percent > 0.0 {
        segs.push((Lead::Pipe, metric_row("SSD", s.ssd_percent)));
    }
    if let Some(h) = s.hdd_percent {
        segs.push((Lead::Pipe, metric_row("HDD", h)));
    }
    // 代理段:以网卡图标作前导(替代原先的 `Proxy` 文字标签),排在网络段之前。
    // 永远渲染——有代理显示 `{addr}`,无代理显示 `OFF`(系统未配置代理的明确
    // 状态,不再像之前那样整段隐藏)。
    segs.push((
        Lead::Icon,
        text(format!("Proxy  {}", s.proxy.as_deref().unwrap_or("OFF")))
            .size(theme::font::caption_sm())
            .color(theme::color::BG)
            .into(),
    ));
    // 网速段:排在代理段之后,前导 `｜`,纯展示。
    segs.push((
        Lead::Pipe,
        text(format!(
            "↓ {}  ↑ {}",
            format_speed(s.net_down_bps),
            format_speed(s.net_up_bps)
        ))
        .size(theme::font::caption_sm())
        .color(theme::color::BG)
        .into(),
    ));

    // 逐段拼装:每段前导由 `Lead` 决定——默认 `｜`,代理段前用 icon
    // (Lucide,深色描边浮在奶油背景上,与文字同色、垂直居中)作区分,
    // 网速段前用普通 `｜`。
    let mut parts: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
        Vec::with_capacity(segs.len() * 2);
    for (i, (lead, elem)) in segs.into_iter().enumerate() {
        if i > 0 {
            match lead {
                Lead::None => {}
                Lead::Pipe => parts.push(
                    text("｜")
                        .size(theme::font::caption_sm())
                        .color(theme::color::BG)
                        .into(),
                ),
                Lead::Icon => parts.push(icons::view(
                    icons::IconKind::SquareRadical,
                    icon_size::row(),
                    theme::color::BG,
                )),
            }
        }
        parts.push(elem);
    }

    // 布局:系统信息(CPU/RAM/SSD/HDD/网速/Proxy)**靠左**,Dozer 应用名称与
    // 版本**靠右**——中间用 `Space::with_width(Length::Fill)` 撑开。footbar
    // 背景用窗口根背景色(`#dcc9a3`,theme::region::background),文字/图标
    // 用深色 `#0a0e16`(theme::color::BG)浮在奶油背景上。CPU 段前缀图标
    // (Lucide square-activity)同色同对齐。
    let cpu_icon = icons::view(
        icons::IconKind::SquareActivity,
        icon_size::row(),
        theme::color::BG,
    );

    let left = row![cpu_icon]
        .push(row(parts).spacing(4).align_y(Alignment::Center))
        .align_y(Alignment::Center)
        .spacing(4);

    // 右侧:应用名称 + 版本,右对齐。版本号取 crate 版本
    // (`env!("CARGO_PKG_VERSION")`),与 Cargo.toml 同步。footbar 背景是奶油色
    // `#dcc9a3`:金 `#F2D94E` 在其上对比度极低(几乎看不见),次级灰 DIM
    // 又太接近深色 BG 不易区分,所以名称用深色 BG、版本号用主题蓝
    // `#4D8CFF` 作明确区分——在奶油底上清晰可读且与左侧系统信息拉开层级。
    let app_icon = icons::view(
        icons::IconKind::SquareTerminal,
        icon_size::row(),
        theme::color::BG,
    );
    let app_name = text("Dozer AI Coder")
        .size(theme::font::caption_sm())
        .color(theme::color::BG);
    let app_version = text(format!("v{}", env!("CARGO_PKG_VERSION")))
        .size(theme::font::caption_sm())
        .color(iced_widget::core::Color::from_rgb8(0xFF, 0x6E, 0x6E));
    let right = row![app_icon, app_name, app_version]
        .spacing(6)
        .align_y(Alignment::Center);

    let content = row![left, Space::new().width(Length::Fill), right]
        .align_y(Alignment::Center)
        .width(Length::Fill);

    container(content)
        .width(Length::Fill)
        .height(Length::Fixed(theme::geometry::footbar_height()))
        .padding(region.padding)
        .align_y(Alignment::Center)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::region::background().into()),
            ..container::Style::default()
        })
        .into()
}

/// 单个数值段:标签(深色)+ 数值(用量 >75% 时变红 `#FF6E6E`)。
/// 标签与数值分两段拼接,只让数值部分随阈值变色,标签保持原色。
fn metric_row(
    prefix: &'static str,
    percent: f32,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let value_color = if percent > 75.0 {
        iced_widget::core::Color::from_rgb8(0xFF, 0x6E, 0x6E)
    } else {
        theme::color::BG
    };
    row![
        text(prefix)
            .size(theme::font::caption_sm())
            .color(theme::color::BG),
        text(format!("  {:.0}%", percent))
            .size(theme::font::caption_sm())
            .color(value_color),
    ]
    .align_y(Alignment::Center)
    .into()
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
            ssd_total += *total;
        } else if !ro && !is_macos_apfs_system_volume(mp) {
            // 非启动盘、可写、不是 macOS APFS 系统 volume → HDD。
            // macOS APFS 把启动盘分成多个 volume(`/`、`/System/Volumes/Data`、
            // `/Volumes/Preboot`、`/Volumes/Recovery` 等),其中可写的非启动盘
            // (如 `/System/Volumes/Data`)是启动盘的子卷不是独立盘,必须过滤,
            // 否则 MacBook 会错误显示 HDD 段。Preboot/Recovery 通常只读已被
            // `!ro` 过滤,这里主要挡 Data 卷。
            hdd_used += used;
            hdd_total += *total;
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

/// 判断 mountpoint 是否是 macOS APFS 系统 volume(启动盘的子卷,
/// 不是独立盘)。这些 volume 是可写的(`/System/Volumes/Data`),
/// 但它们跟启动盘在同一个 APFS container,占用空间共享,
/// 不应该当作 HDD 单独统计。
fn is_macos_apfs_system_volume(mp: &std::path::Path) -> bool {
    let p = mp.to_string_lossy();
    p.starts_with("/System/Volumes/")
        || p == "/Volumes/Preboot"
        || p == "/Volumes/Recovery"
        || p == "/Volumes/VM"
        || p == "/private/var/vm"
}

/// 把 `sysinfo::Disks` 适配成 `aggregate_disks` 需要的切片再调它。
/// 分离这一层让 `aggregate_disks` 可以纯单测覆盖(不必 mock sysinfo 类型)。
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

/// 剥掉 `http://`/`https://`/`socks5://`/`socks5h://`/`socks4://`/`socks4a://`
/// 等 scheme 前缀(大小写不敏感),并去掉尾随 `/`。
fn strip_proxy_scheme(s: &str) -> String {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    for scheme in [
        "http://",
        "https://",
        "socks5://",
        "socks5h://",
        "socks4://",
        "socks4a://",
    ] {
        if lower.starts_with(scheme) {
            return s[scheme.len()..].trim_end_matches('/').to_string();
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
    if enable && let (Some(h), Some(p)) = (host, port) {
        return Some(format!("{h}:{p}"));
    }
    None
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
            // `received()`/`transmitted()` 返回"自上次 refresh 以来的增量"
            // ——sysinfo 自管 baseline,task 闭包不需要持有 `last_net: HashMap`。
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
            let _ = proxy.send_event(crate::app::Message::Footbar(Message::Sampled(sample)));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    fn make_disks(spec: &[(&str, u64, u64, bool)]) -> Vec<(PathBuf, u64, u64, bool)> {
        spec.iter()
            .map(|(p, t, a, r)| (PathBuf::from(p), *t, *a, *r))
            .collect()
    }

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
        let disks = make_disks(&[("/", 500_000_000_000, 50_000_000_000, false)]);
        let (ssd, hdd) = aggregate_disks(&disks);
        assert!(approx_eq(ssd, 90.0), "ssd was {ssd}");
        assert_eq!(hdd, None);
    }

    #[test]
    fn aggregate_disks_boot_plus_extra() {
        let disks = make_disks(&[
            ("/", 500_000_000_000, 50_000_000_000, false),
            ("/Volumes/Ext", 1_000_000_000_000, 500_000_000_000, false),
        ]);
        let (ssd, hdd) = aggregate_disks(&disks);
        assert!(approx_eq(ssd, 90.0), "ssd was {ssd}");
        assert!(
            matches!(hdd, Some(v) if approx_eq(v, 50.0)),
            "hdd was {hdd:?}"
        );
    }

    #[test]
    fn aggregate_disks_readonly_extra_excluded() {
        let disks = make_disks(&[
            ("/", 500_000_000_000, 50_000_000_000, false),
            (
                "/Volumes/Readonly",
                1_000_000_000_000,
                500_000_000_000,
                true,
            ),
        ]);
        let (ssd, hdd) = aggregate_disks(&disks);
        assert!(approx_eq(ssd, 90.0), "ssd was {ssd}");
        assert_eq!(hdd, None);
    }

    #[test]
    fn aggregate_disks_macos_apfs_system_volumes_excluded() {
        // MacBook 上 APFS 把启动盘分成多个 volume,可写的非启动盘
        // (如 `/System/Volumes/Data`)是启动盘子卷不是独立盘,必须过滤,
        // 否则会错误显示 HDD 段。Preboot/Recovery 通常只读已被 `!ro` 过滤,
        // 这里主要验证 Data 卷(可写)被排除。
        let disks = make_disks(&[
            ("/", 500_000_000_000, 50_000_000_000, false),
            (
                "/System/Volumes/Data",
                500_000_000_000,
                100_000_000_000,
                false,
            ),
            ("/Volumes/Preboot", 100_000_000, 50_000_000, true),
        ]);
        let (ssd, hdd) = aggregate_disks(&disks);
        assert!(approx_eq(ssd, 90.0), "ssd was {ssd}");
        assert_eq!(hdd, None, "macOS APFS 系统 volume 不应该算 HDD");
    }

    #[test]
    fn aggregate_disks_no_disks() {
        let (ssd, hdd) = aggregate_disks(&[]);
        assert!(approx_eq(ssd, 0.0), "ssd was {ssd}");
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
        assert!(approx_eq(state.sample.cpu_percent, 18.0));
        assert!(approx_eq(state.sample.net_down_bps as f32, 12_000_000.0));
    }

    #[test]
    fn detect_proxy_from_env_priority_and_scheme_stripping() {
        // 单测试函数内串行改 env,避免 cargo test 默认多线程下的 env race。
        // Rust 2024 edition 把 set_var/remove_var 标记 unsafe(非线程安全),
        // 通过 helper 函数封装 unsafe 块。
        fn set_env(name: &str, value: &str) {
            // SAFETY: 测试函数串行执行,无并发访问同名 env 变量。
            unsafe { std::env::set_var(name, value) }
        }
        fn remove_env(name: &str) {
            // SAFETY: 同上。
            unsafe { std::env::remove_var(name) }
        }
        let names = [
            "ALL_PROXY",
            "HTTPS_PROXY",
            "HTTP_PROXY",
            "all_proxy",
            "https_proxy",
            "http_proxy",
        ];
        for v in names {
            remove_env(v);
        }

        // 1. 无 env → None
        assert_eq!(detect_proxy_from_env(), None);

        // 2. ALL_PROXY 带 http:// scheme → 剥 scheme
        set_env("ALL_PROXY", "http://127.0.0.1:7890");
        assert_eq!(detect_proxy_from_env().as_deref(), Some("127.0.0.1:7890"));
        remove_env("ALL_PROXY");

        // 3. HTTPS_PROXY 无 scheme → 原样
        set_env("HTTPS_PROXY", "127.0.0.1:7890");
        assert_eq!(detect_proxy_from_env().as_deref(), Some("127.0.0.1:7890"));
        remove_env("HTTPS_PROXY");

        // 4. http_proxy(小写)→ 优先级最低,但 None 时被命中
        set_env("http_proxy", "127.0.0.1:8080");
        assert_eq!(detect_proxy_from_env().as_deref(), Some("127.0.0.1:8080"));
        remove_env("http_proxy");

        // 5. 优先级:ALL_PROXY > HTTPS_PROXY
        set_env("ALL_PROXY", "http://1.1.1.1:1111");
        set_env("HTTPS_PROXY", "127.0.0.1:7890");
        assert_eq!(detect_proxy_from_env().as_deref(), Some("1.1.1.1:1111"));
        remove_env("ALL_PROXY");
        assert_eq!(detect_proxy_from_env().as_deref(), Some("127.0.0.1:7890"));
        remove_env("HTTPS_PROXY");

        // 6. scheme 剥离 socks5:// 与尾随斜杠
        set_env("ALL_PROXY", "socks5://127.0.0.1:1080/");
        assert_eq!(detect_proxy_from_env().as_deref(), Some("127.0.0.1:1080"));
        remove_env("ALL_PROXY");

        // 7. 大写 scheme 也要剥(HTTP://)
        set_env("ALL_PROXY", "HTTP://127.0.0.1:7890");
        assert_eq!(detect_proxy_from_env().as_deref(), Some("127.0.0.1:7890"));
        remove_env("ALL_PROXY");
    }
}
