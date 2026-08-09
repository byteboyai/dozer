//! 底部 footbar 系统信息条:CPU/RAM/SSD/HDD/Proxy/下行/上行 7 段常驻显示。
//! App 级状态(挂 `App.footbar`,不挂 `Workspace`——跨所有项目页签共享)。
//! 设计见 `docs/superpowers/specs/2026-08-09-footbar-system-info-design.md`。

use crate::theme;
use iced_widget::core::{Alignment, Border, Element, Length};
use iced_widget::{container, row, text};
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
/// 转成顶层消息。无项目依赖、无交互——纯展示条,样式直接用
/// `theme::region::status_bar()` + `theme::geometry::status_bar_height()`
/// (视觉口径与 in-pane status_bar 一致,不新增主题令牌)。
pub fn view(state: &AppState) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
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
    segs.push(format!("Proxy  {}", s.proxy.as_deref().unwrap_or("—")));
    segs.push(format!(
        "↓ {}  ↑ {}",
        format_speed(s.net_down_bps),
        format_speed(s.net_up_bps)
    ));
    let body = segs.join("｜");

    // 用户样张首尾也带 `｜`,这里照做。
    let content = row![
        text("｜")
            .size(theme::font::caption())
            .color(theme::color::DIM),
        text(body)
            .size(theme::font::caption())
            .color(theme::color::BODY),
        text("｜")
            .size(theme::font::caption())
            .color(theme::color::DIM),
    ]
    .spacing(0)
    .align_y(Alignment::Center);

    container(content)
        .width(Length::Fill)
        .height(Length::Fixed(theme::geometry::status_bar_height()))
        .padding(region.padding)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: Border {
                color: base.color,
                width: base.width,
                radius: base.radius,
            },
            ..container::Style::default()
        })
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
        } else if !ro {
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
}
