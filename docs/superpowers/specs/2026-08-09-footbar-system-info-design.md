# Footbar 系统信息条设计

**状态:已批准(brainstorming 会话,2026-08-09)**

## 背景

用户提出在 workspace 窗口底部新增一条 footbar,常驻显示当前系统状态,以便开发时一眼掌握
资源占用与代理/网络状况。期望样张:

```
｜CPU  18%｜RAM  43%｜SSD  90%｜HDD  50%｜Proxy  127.0.0.1:7890｜↓ 12.4 MB/s  ↑ 1.8 MB/s｜
```

挖掘现状:

1. `App::view`(`workspace.rs:4614-4623`)根视图只有两行:`column![top_bar, body]`,
   再叠一层 `apply_overlays` 的浮层。**没有任何窗口级底部条**。
2. 现有"状态条"全部是**面板内部**的(`terminal_pane` 末尾的 `terminal_status_bar`,
   见 `workspace.rs:6766`),靠 `status_bar_container`(`workspace.rs:6808`)统一加
   固定 26px 高 + `theme::region::status_bar()`(`theme/region.rs:345`)的背景/边框。
   这个区域样式令牌是**可共享**的——`status_bar` 不是 terminal 专属。
3. 周期性刷新机制已有两套 precedent:
   - Pattern A(单次 spawn):`spawn_conversations_refresh`/`spawn_acceptance_count_refresh`
     /`spawn_project_git_refresh`,签名都是 `io.handle.spawn(async move {
     spawn_blocking; proxy.send_event })`(`workspace.rs:1914` 起)。
   - Pattern B(winit `ControlFlow::WaitUntil` 周期唤醒):`BLINK_INTERVAL=450ms`、
     `TODO_POLL_INTERVAL=1000ms`(`main.rs:144-149`),`Runner::about_to_wait`
     (`main.rs:994-1018`)挑最近下一个唤醒点。**整个 `dozer-app` 里没有
     `tokio::time::interval`**——周期性工作要么走 winit 唤醒,要么 spawn 一个
     `loop { sleep; ... }` 自管节奏。
4. 系统信息领域在 `dozer-app` 内是**完全空白**——`Cargo.toml` 无 `sysinfo`/`heim`/
   `nvml` 任何系统信息依赖;无 CPU/RAM/网络/代理检测代码。`legacy-boy` 已废弃且
   禁止扩展,但其 `Cargo.toml:21` 已 pin `sysinfo = "0.32"`,workspace lockfile 是
   热的——`dozer-app` 只需在自己 `Cargo.toml` 加同名依赖即可,不会引入新版本。
5. 扩展化重构已落地 9 个试点(`extensions/{acceptance,browser,database,files,
   git_log,project,ssh,todo,usage}`),`WorkspaceState`/`Message`/`update`/`view`
   四件套是既定形状。**App 级状态**(跨项目共享的,而非每个项目一份)有 4 个
   precedent:`home_browser`/`git_log`/`todo`/`database`(`App` struct 字段,
   `workspace.rs:1362-1372`)——系统信息显然跨项目,属于这一类。
6. `ShellIo`(`workspace.rs:1264-1271`)是 `App::update` 借 `&mut self` 同时想用
   `&self.client` 的标准解法,克隆廉价(`Client`/`Handle`/`EventLoopProxy` 都是
   Arc 内含),所有 spawn 函数都收一个 `&ShellIo` 拿 `handle` + `proxy`。

## 目标 / 非目标

**目标**:

1. 在根视图 `App::view` 末尾追加第三行 `footbar`,与 `top_bar`(上)/`body`(中)
   并列;高度复用 `theme::geometry::status_bar_height()`(26px,scale 后)的视觉口径,
   背景复用 `theme::region::status_bar()` 令牌,不新增主题区域令牌(同一类"细条"
   不应分两个令牌)。
2. 新建 `extensions::footbar`,拥有自己的 `AppState`(注意是 **App 级**,不是
   `WorkspaceState`)/`Message`/`update`/`view`/`spawn_sampler`,镜像
   `extensions::usage` 的五件套形状,但状态挂在 `App` 而非 `Workspace`(同
   `home_browser`/`git_log`/`todo`/`database` 的 App 级状态 precedent)。
3. 显示 7 段:`CPU %`/`RAM %`/`SSD %`/`HDD %`/`Proxy`/`↓ 下行速度`/`↑ 上行速度`,
   段间用全宽竖线 `｜` 分隔(用户样张格式),标签与值之间两空格。无 HDD(仅一块盘)
   时该段折叠;无代理时显示 `—`。
4. 刷新节奏:
   - **1s 一次**:CPU、RAM、网络速度(用户能感知变化的量)。
   - **300s 一次**:代理、硬盘占用(变化慢且 `scutil` 调用是进程 spawn,贵)。
   两套节奏在一个长生命周期 tokio 任务里以 `tokio::time::interval` 自管,不走
   winit `ControlFlow::WaitUntil`——因为采样必须 `spawn_blocking`(sysinfo 阻塞
   ~200ms 测 CPU),不应该卡 winit 主循环;并且 `proxy.send_event` 落地 iced 消息
   时会自动触发重绘,不需要 winit 唤醒协同。
5. 引入 `sysinfo = "0.32"` 依赖(与 `legacy-boy` 同版本,锁文件已热)。CPU/RAM/
   网络/磁盘全部走 sysinfo;代理检测不在 sysinfo 范围,自实现一个小函数
   `detect_proxy() -> Option<String>`,顺序:env `ALL_PROXY`/`HTTPS_PROXY`/
   `HTTP_PROXY`(及小写变体)→ macOS `scutil --proxy` 解析 `HTTPEnable=1`/`HTTPProxy`/
   `HTTPPort` → 都没有返回 `None`(显示 `—`)。
6. 网络速度 = sysinfo 的 `NetworkData::received()`/`transmitted()`(每次
   `Networks::refresh()` 后返回"自上次 refresh 以来的字节增量",sysinfo 内部
   自管 baseline,不需要 task 闭包里再存 `last_net: HashMap`)除以实际经过秒数,
   按量级自动选单位(`<1 KB/s` → `0.0 KB/s`、`<1 MB/s` → `XX.X KB/s`、
   `<1 GB/s` → `XX.X MB/s`、否则 `XX.X GB/s`),一律保留 1 位小数。第一次采样
   显示 `0.0 KB/s`(`received()` 在刚 `new_with_refreshed_list()` 后第一次
   `refresh()` 通常返回 0)。**统计口径是全接口聚合 + 虚拟接口 denylist**
   (见 brainstorming 决议 2),不是裸全聚合、也不是只看默认网卡。
7. SSD 段 = 启动盘(`mountpoint == "/"`)的占用百分比;HDD 段 = 其余可写磁盘的
   **聚合**占用百分比(总已用 / 总容量),无额外盘则整段 `HDD` 折叠不显示。
8. 启动:`App::bootstrap`(或 `App::new_shell`,见实现计划)里调一次
   `footbar::spawn_sampler(&io)`,fire-and-forget——`proxy.send_event` 内含
   `EventLoopProxy`(`Arc` 内含),runtime drop 时任务自然取消,不需要 `JoinHandle`
   管控生命周期。

**非目标**:

- 不显示进程级信息(哪个进程吃 CPU/RAM)——v1 只看整机。
- 不画历史曲线/迷你图——v1 只显示当前数值,纯文字。后续要加 mini sparkline
  时再单独一轮设计。
- 不支持用户配置刷新间隔(1s/300s 写死在 `spawn_sampler` 里)。
- 不支持切换/启停代理——footbar 只读展示。
- 不显示磁盘 I/O 速度、GPU、温度、电池——v1 范围明确排除。
- 不支持折叠/隐藏 footbar——v1 永远常驻在窗口底部。
- 不在 footbar 上加任何交互(点击/右键菜单)——纯展示条。
- 不为 footbar 单独建 `theme::region::footbar()` 令牌或 `footbar_height()`
  几何常量——视觉口径与 in-pane status_bar 完全一致,直接复用令牌。
- 不把 `terminal_status_bar` 也并入 footbar——两件事不冲突,in-pane 状态条
  讲的是 agent 会话状态,footbar 讲的是整机状态,职责正交。

## 关键语义确认(brainstorming 已批准,2026-08-09)

- **状态归属:App 级,不是 Workspace 级**。系统信息跨所有项目页签共享,挂
  `App` 字段(`self.footbar: footbar::AppState`),不挂 `Workspace`。判断依据
  与 `extensions::project` 设计的"谁展示就归谁"原则一致——但系统信息没有"每个
  项目一份"的展示需求,只有一个全局展示位置,所以归 App。
- **节奏编排:一个长生命周期 tokio 任务 + 两个 `tokio::time::interval`**,
  不走 winit `ControlFlow::WaitUntil`、不走每次 winit 唤醒时同步 `spawn_blocking`。
  理由:① sysinfo 的 `refresh_cpu()` 阻塞 ~200ms,放 winit 主循环会拖卡 UI;
  ② iced 的 `proxy.send_event` 触发的消息处理会自动 `window.request_redraw()`,
  不需要 winit 唤醒协同;③ 一个 task 内自己管两个 interval 比把"快慢两种节奏"
  塞进 winit 唤醒逻辑简单得多——winit 唤醒点是单值(`WaitUntil(next)`),
  要管两种节奏还得每次都算 `min(快tick, 慢tick)`,复杂度白白上升。
- **采样在 `spawn_blocking` 里做,结果整包发回**。CPU/RAM/网络/磁盘/代理五件
  一次 `spawn_blocking` 采完,发一个 `Message::Footbar(Sampled { .. })`——
  不拆 5 条消息。理由:5 件是同源同周期采样的(代理除外,见下),拆 5 条只会
  让 `update` 多 4 次 `self.footbar.x = ..` 的赋值,没有信息增益。代理的慢节奏
  通过"上次代理检测距今 ≥ 300s 才重测"的内部计时实现,结果合并进同一条
  `Sampled` 消息的 `proxy` 字段(缓存上次结果)。
- **`sysinfo::System`/`Networks`/`Disks` 长生命周期实例住在 task 的 `async move`
  块里,不进 `AppState`**。`AppState` 只存"最近一次 `Sampled` 的值"用于渲染;
  计算用的 baseline(上次采样时刻、`sysinfo::System`/`Networks`/`Disks` 句柄——
  注意网络增量 sysinfo 内部自管,task 闭包不再持有 `last_net: HashMap`)留在
  task 闭包里,与 UI 状态解耦——UI 状态永远可以 `Default::default()` 出来从零
  开始,task 启动后第一次采样落地才填上真实值。
- **代理检测是 process spawn(`scutil`),必须放慢节奏**。`scutil --proxy`
  在 macOS 上 ~50-100ms 一次,1s 调一次会拖累采样任务;且代理配置几乎不会秒级
  变化(用户切代理也是手动操作,300s 内足够看见)。env 变量本身读取廉价,但
  统一走 300s 缓存路径简单——不为此区分快慢。
- **HDD 段聚合,不逐盘列出**。用户样张只有一个 `HDD 50%` 段,不打算列多盘。
  聚合方式:把所有"非启动盘且 `is_read_only()==false`"的盘的 `available_space`
  求和、`total_space` 求和,算百分比。无此类盘时 `hdd_percent: None`,view
  整段不渲染。
- **格式:全宽竖线 `｜` 分隔 + 标签值之间两空格**。直接照用户样张格式,不
  换成更紧凑的 `·` 或 iced 自带 spacing——footbar 是信息密度优先的展示条,
  用户明确要求这个分隔符。
- **不在 v1 加 `theme::geometry::footbar_height()` 或 `theme::region::footbar()`**。
  复用 `status_bar_height()`/`status_bar()` 令牌,避免主题分叉。后续若需要
  调 footbar 单独高度/边框,再单独加令牌。

### brainstorming 决议(2026-08-09,3 个开放问题)

1. **footbar 总开关——v1 不做,且不留扩展点备注**。footbar 永远显示。
   理由:① 26px 占用极小,在意空间用 ⌘+/- 缩放更直接;② Dozer 现在的 settings
   入口缺一个真正承载 per-user 偏好的配置面板(主要是 `ShellLayout` 持久化),
   为 footbar 一个开关就要先做配置面板基础设施,不值;③ 真要做也是后续把
   "用户偏好"基础设施和多个开关一起做,不是 footbar 单独背负。若将来真有
   用户反馈挤占,可后续单独补设计,这次不留 non-goal 备注——留了反而暗示
   "马上要做"。
2. **网速口径——全接口聚合 + 虚拟接口 denylist**。统计所有 `sysinfo::Networks`
   接口的 `bytes_sent`/`bytes_received`,但排除名字匹配下列模式的接口:
   - `lo` / `lo0`(loopback)
   - `docker*` / `bridge*` / `veth*` / `br-*`(Docker / 容器网桥)
   - `utun*` / `ipsec*`(VPN 隧道在容器内可能双算,但实际 utun 是真实 VPN
     流量应该统计——**这条不进 denylist**,只排除前两类纯虚拟接口)
   
   denylist 是一个 `&[&str]` 静态常量,匹配方式 `name.starts_with(pattern)`,
   写在 `spawn_sampler` 顶部几行。理由:① 选默认网卡在 sysinfo 里没有直接 API,
   需要查路由表或 `scutil --nwi`,复杂度高且平台耦合;② 全聚合裸值在 Docker
   用户机器上会因 `docker0`/`bridge0` 内部容器流量虚高,而 Dozer 的目标用户
   (开发 macOS 本机 agent)Docker 使用率相当高,值得加 denylist 兜底;③ denylist
   几行常量,可读、可后续按需扩展,不过度设计。
3. **CPU 口径——整机平均**。用 `sysinfo::System::cpu_usage()` 默认输出。理由:
   ① 用户样张就是整机百分比(`18%` 在 8 核机器上不会让人误解);② 单核需要
   额外 UI 决策(显示哪个核?峰值核?平均最高核?),复杂度白白上升;③ 整机
   平均是 universal baseline,后续若加进程级信息(哪个进程吃 CPU)再细粒度更顺;
   ④ sysinfo `cpu_usage()` 默认就是整机平均,零额外代码。

## 架构与数据流

### 1. 状态类型(`extensions/footbar.rs`)

```rust
/// App 级系统信息条状态——挂在 `App.footbar`,跨所有项目页签共享。
#[derive(Default)]
pub struct AppState {
    sample: Sample,
}

/// 一次完整采样的结果。CPU/RAM/网络是 1s 节奏,代理/硬盘是 300s 节奏
/// (代理在两次 300s 之间复用上次结果)。网络速度已是除以 elapsed 之后
/// 的最终 bytes/sec,UI 不再算差分。
#[derive(Clone, Default)]
pub struct Sample {
    cpu_percent: f32,           // 0.0..=100.0 整机平均
    ram_percent: f32,           // 0.0..=100.0
    ssd_percent: f32,           // 启动盘占用百分比
    hdd_percent: Option<f32>,   // 其余盘聚合占用百分比;None = 无额外盘,该段折叠
    proxy: Option<String>,       // "host:port";None = 未启用代理,显示 "—"
    net_down_bps: f64,           // bytes/sec 下行
    net_up_bps: f64,             // bytes/sec 上行
}

/// 顶层 `Message::Footbar(footbar::Message::...)` 的载荷。
#[derive(Debug, Clone)]
pub enum Message {
    /// 采样任务周期性发回的快照。每 1s 一条(代理/硬盘字段在缓存的 300s 周期
    /// 内复用上次值,不会每秒重算)。
    Sampled(Sample),
}
```

### 2. `update`

```rust
pub fn update(state: &mut AppState, msg: Message) {
    match msg {
        Message::Sampled(s) => state.sample = s,
    }
}
```

不收 `&ShellIo`、不 `emit`、不带 `project_id`——这是 App 级 + 纯展示,内核路由
比其他 extension 更简单:

```rust
Message::Footbar(msg) => {
    footbar::update(&mut self.footbar, msg);
}
```

无需 `with_project`/`with_focused_project`,因为不依赖任何 `Workspace`。

### 3. 长生命周期采样任务

```rust
/// 在 `App` bootstrap 阶段调一次,fire-and-forget。runtime drop 时任务随
/// `tokio::Runtime` 一起取消。内部管两个节奏:1s 的 CPU/RAM/网络,
/// 300s 的代理/磁盘——靠"距上次慢节奏采样的时间差"判断,不另起 interval
/// (避免快慢两个 interval tick 漂移导致同秒双发)。
pub fn spawn_sampler(io: &ShellIo) {
    let proxy = io.proxy.clone();
    io.handle.spawn(async move {
        let mut sys = sysinfo::System::new();
        let mut nets = sysinfo::Networks::new_with_refreshed_list();
        let mut disks = sysinfo::Disks::new_with_refreshed_list();
        // 代理缓存:启动时立刻测一次,之后 300s 复用
        let mut cached_proxy: Option<String> = detect_proxy();
        let mut last_proxy_check = Instant::now();
        // CPU 首次 refresh——sysinfo 需要一次 baseline 才能
        // `global_cpu_usage()` 出值
        sys.refresh_cpu_usage();
        tokio::time::sleep(Duration::from_millis(500)).await;

        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.tick().await; // 跳过首次立即触发,与上面的 500ms 一起避免冷启 0%
        loop {
            tick.tick().await;
            // 1s 节奏
            sys.refresh_cpu_usage();
            sys.refresh_memory();
            nets.refresh();
            let now = Instant::now();
            // 收到 `&Networks` 引用时直接读每接口的 `received()`/`transmitted()`
            // ——sysinfo 自管 baseline,这两个方法返回"自上次 refresh 以来的增量"。
            // 注意:循环外只持有 `&mut Networks`,这里要重建迭代器。
            let mut down: u64 = 0;
            let mut up: u64 = 0;
            for (name, net) in &nets {
                if is_virtual_interface(name) { continue; }
                down += net.received();
                up += net.transmitted();
            }
            let elapsed = 1.0; // 1s interval 周期;实际可 (now - last_ts) 取更精确值
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

/// 网络接口 denylist 模式匹配(见 brainstorming 决议 2)。`lo`/`docker*`/
/// `bridge*`/`veth*`/`br-*` 排除;`utun*`/`ipsec*` 不排除(VPN 是真实流量)。
fn is_virtual_interface(name: &str) -> bool {
    const DENYLIST: &[&str] = &["lo", "lo0", "docker", "bridge", "veth", "br-"];
    DENYLIST.iter().any(|p| name.starts_with(p))
}
```

设计要点(不写实现细节,只点决策):

- **两个节奏合一个 `interval` + 距上次慢节奏时间差判断**,不开两个 interval。
  理由:两个 interval tick 会漂移,某次恰好同时到点会触发两次 `send_event`,
  UI 闪一下;合并到一个 loop 让"快 1s / 慢 300s"在一个采样帧内一次发完。
- **CPU 首次采样需要预 `refresh_cpu_usage()` + 500ms 等待**,否则 `sysinfo`
  第一次 `global_cpu_usage()` 返回 0(无 baseline)。冷启 footbar 头 1s 显示
  0% 可接受,500ms 等待后第一条真实数据落地。
- **磁盘 refresh 与代理 refresh 共用 300s 节奏**,避免磁盘每秒被 refresh(磁盘
  refresh 在某些机器涉及 statfs 调用,虽然便宜但没必要 1s 一次)。
- **网络增量用 `received()`/`transmitted()` 而非自己存 baseline 算 `total_*()` 差**。
  sysinfo 内部维护 baseline,每次 `Networks::refresh()` 后这两个方法直接返回自
  上次 refresh 的增量。task 闭包里**不需要** `last_net: HashMap`——比老版 sysinfo
  的 `bytes_sent()`/`bytes_received()` API 更简洁。
- **`sysinfo::System::global_cpu_usage()` 直接返回整机平均百分比**,不需要
  遍历 `cpus()` 自己算。这是 brainstorming 决议 3"整机平均"口径的直接 API 对应。

### 4. `view` 渲染

```rust
/// footbar 主入口。被 `App::view` 在根 column 末尾调用。无项目依赖、
/// 无交互——纯展示条,样式直接用 `theme::region::status_bar()` + 
/// `theme::geometry::status_bar_height()`(视觉口径与 in-pane status_bar 一致)。
pub fn view<'a>(state: &'a AppState) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
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
    segs.push(format!("↓ {}  ↑ {}", format_speed(s.net_down_bps), format_speed(s.net_up_bps)));
    let text = segs.join("｜");
    container(
        row![
            text("｜").size(theme::font::caption()).color(theme::color::DIM),
            text(text).size(theme::font::caption()).color(theme::color::BODY),
            text("｜").size(theme::font::caption()).color(theme::color::DIM),
        ]
        .spacing(0)
        .align_y(iced_widget::core::Alignment::Center),
    )
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
    if bps < 1024.0 {
        "0.0 KB/s".into()
    } else if bps < 1024.0 * 1024.0 {
        format!("{:.1} KB/s", bps / 1024.0)
    } else if bps < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB/s", bps / 1024.0 / 1024.0)
    } else {
        format!("{:.1} GB/s", bps / 1024.0 / 1024.0 / 1024.0)
    }
}
```

### 5. 磁盘/代理辅助纯函数

```rust
/// 返回 (ssd_%, hdd%)。ssd = mountpoint=="/" 的盘占用百分比;hdd = 其余
/// 可写盘的"已用之和 / 容量之和"百分比;无可写非启动盘 → None。
fn compute_disk_usage(disks: &sysinfo::Disks) -> (f32, Option<f32>) { .. }

/// 代理检测:env(`ALL_PROXY`/`HTTPS_PROXY`/`HTTP_PROXY` 及小写)→ macOS
/// `scutil --proxy` 解析 → 都没有返回 None。env 值剥掉 `http://`/`socks5://`
/// 等 scheme 前缀,只留 `host:port`。
fn detect_proxy() -> Option<String> { .. }
```

### 6. 内核接线

- `App` struct 加字段 `footbar: footbar::AppState`(`workspace.rs:1276-1373`
  App struct 定义处)。构造点 `App::new_shell`(`workspace.rs:2671-2715`)
  初始化 `footbar: footbar::AppState::default()`;紧接其后调
  `footbar::spawn_sampler(&io)` 启动长生命周期任务。
- 顶层 `Message` 枚举加变体 `Footbar(footbar::Message)`(`workspace.rs:1054`
  附近,`Files`/`Project` 等并列)。
- `App::update` 路由分支:`Message::Footbar(msg) => footbar::update(&mut self.footbar, msg),`
- `App::view`(`workspace.rs:4614-4623`)改:
  ```rust
  let top = top_bar(self);
  let body = self.page_content(self.current_page, self.active_project_id);
  let foot = footbar::view(&self.footbar);
  let base = column![top, body, foot];
  self.apply_overlays(base.into())
  ```
  `column!` 默认子项 `Length::Fill` 高度,`top_bar`/`footbar::view` 各自
  容器内显式 `.height(Length::Fixed(..))` 钉死,`body` 自然占中间剩余空间。

## 错误处理

- `sysinfo` 调用失败本身不抛错——sysinfo 大多返回 0 而非 Result,footbar 显示
  `0%`/`0.0 KB/s` 即可,不报错文案。
- `scutil` 找不到/异常退出 → 退到只用 env 变量结果(可能 `None` → 显示 `—`)。
  不写日志噪音——代理检测失败本来就是正常路径(用户没设代理)。
- 启动盘检测不到(`/` 不在 `disks` 列表)→ SSD 段显示 `—`,HDD 段逻辑不变。
  极少见(只有容器化环境才会发生),不专门设计容错。
- `App` 状态默认 `Sample::default()`(全 0 + `proxy: None`),首次采样落地
  之前 footbar 显示 `CPU 0%｜RAM 0%｜...｜Proxy —｜↓ 0.0 KB/s  ↑ 0.0 KB/s`,
  不显示 "loading" 文案——空状态就是合法初始值。

## 测试策略

- `format_speed` 单测:`0.0` → `0.0 KB/s`、`1500.0` → `1.5 KB/s`、
  `1_500_000.0` → `1.4 MB/s`(`1_500_000/1048576 ≈ 1.43`)、
  `1.5e9` → `1.4 GB/s`。覆盖量级边界(刚好 1024 的归属)。
- `compute_disk_usage` 单测:mock `sysinfo::Disks` 不现实(sysinfo 类型不易
  mock),改为对纯逻辑函数 `aggregate_disks(&[(total, available)]) -> (f32, Option<f32>)`
  做单测,`compute_disk_usage` 只是把 `sysinfo::Disks` 适配成这个切片再调它。
- `detect_proxy` 单测:env `ALL_PROXY=http://127.0.0.1:7890` → 剥 scheme 后
  `127.0.0.1:7890`;env `HTTPS_PROXY=127.0.0.1:7890`(无 scheme)→ 原样;
  env 全空且非 macOS → `None`;env 全空且 macOS 时跳过 `scutil` mock(单测
  不调真实 `scutil`,留作人工验收路径)。
- `footbar::update` 单测:`Message::Sampled(s)` 落地后 `state.sample == s`。
- 人工验收:启动 `cargo run -p dozer-app`,确认 footbar 在窗口最底部渲染,
  7 段齐全(或 HDD 段在无外置盘时折叠);CPU/RAM/网速每秒变化;代理段在
  设置了 `HTTPS_PROXY` 后重启 app 立刻看到;切代理配置(改 env)后等 5min
  内更新;切项目页签时 footbar 不重建、不闪烁;窗口缩放时 footbar 高度恒定
  26px(scale 后);`⌘+`/`⌘-` 字号缩放时 footbar 文字同比例放大。

## 依赖变更

- 新增 `sysinfo = "0.32"` 到 `crates/dozer-app/Cargo.toml` `[dependencies]`
  (与 `crates/legacy-boy/Cargo.toml:21` 同版本,workspace `Cargo.lock` 已含
  该版本解析结果,不需要新增 lock 条目)。
- 无新增 iced 生态依赖。无新增 macOS 平台依赖(sysinfo 自带)。

## 排期备注

- 这个面板只动 `App` struct/`App::view`/`App::update`/`App::new_shell` 和
  新建 `extensions/footbar.rs`,不碰任何 `Workspace` 字段、不碰
  `Message::ProjectFsChanged`/`Message::Files` 等已落地路由——与 `extensions::project`
  试点改动同一个文件(`workspace.rs`)但**字段/消息不重叠**,合并冲突风险低
  (主要是 `App::view` 的 `column!` 那一行可能冲突)。
- 若 `extensions::project` 试点还在进行中,这个面板可以并行启动;若觉得
  `workspace.rs` 同时改两处不安全,等 project 试点合并再开始,排期上属于
  "短任务、低冲突"一类。
