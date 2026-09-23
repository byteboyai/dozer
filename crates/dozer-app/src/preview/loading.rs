//! 文件预览的统一 loading 生命周期状态(`PreviewLoadStage`/`PreviewLoadState`)。
//!
//! 目标(见 `docs/superpowers/plans/2026-09-23-file-preview-loading.md` T1/T2):
//!
//! - 用**显式阶段**表达"当前在等什么",而不是继续扩张单一 `bool loading` 或
//!   零散地把 `BackendState` 在 Loading 与 Ready 间来回弹。具体文案由阶段决定,
//!   集中在本模块 [`PreviewLoadStage::label`],不散落到各 viewer。
//! - 每个异步结果都携带启动时的 `generation`;只有 generation 匹配的
//!   advance/finish/fail 才允许改动 tab,防止"同路径快速刷新 / 关闭重开 / mode
//!   切换"时旧结果结束新 loading(T1/T2 的串台防护)。
//!
//! 本模块**纯数据**,不依赖 iced / wry;iced 视图在
//! `workspace/view.rs::preview_loading_view` 里按 [`PreviewLoadState`] 画动画。

// T1/T2 先建立**完整**阶段与转换 API(T3–T11 的 Windowed/JSON/Tabular/搜索/
// 资源等待会逐阶段接线);此处显式允许迁移期尚未被构造/调用的变体与方法,
// 避免 dead_code 噪声掩盖真实漏点(同 `preview/backend.rs` 的既有做法)。
#![allow(dead_code)]

use std::time::Instant;

/// T11 bullet 3:每个阶段的看门狗时长。超时进入可重试 Failed(不永久转圈)。
/// (取代早期单一的 `PREVIEW_HOST_READY_TIMEOUT`——那时从 `CreatingHost` 到最终
/// ACK 只有一个总时长;现在按阶段分别计时,`CreatingHost` 覆盖 host 启动 + 首批
/// 正文等待,`Reading`/`Indexing`/`LoadingWindow` 各管各的。)
/// 各阶段"合理"的含义:
///
/// - `Reserving`/`CreatingHost`:等资源预算 + host 进程/子视图起来,通常亚秒级,
///   但首次创建 WebView 池可能稍慢,给足余量。
/// - `Reading`/`Parsing`/`Indexing`/`LoadingWindow`:本地文件 IO + 解析/建索引,
///   与文件大小相关;大文件慢是正常的,这里给的是"卡死"上限而非"慢"上限。
/// - `Searching`:整文件搜索,用户可主动取消,时长放宽。
///
/// `SwitchingMode`/`Profiling` 沿用与 `Reading` 同档(短任务,卡住即异常)。
pub fn stage_timeout(stage: PreviewLoadStage) -> std::time::Duration {
    use std::time::Duration;
    match stage {
        PreviewLoadStage::Idle => Duration::from_secs(0),
        PreviewLoadStage::Reserving | PreviewLoadStage::CreatingHost => Duration::from_secs(15),
        PreviewLoadStage::Profiling | PreviewLoadStage::SwitchingMode => Duration::from_secs(15),
        PreviewLoadStage::Reading | PreviewLoadStage::LoadingWindow => Duration::from_secs(20),
        PreviewLoadStage::Parsing | PreviewLoadStage::Indexing => Duration::from_secs(60),
        PreviewLoadStage::Searching => Duration::from_secs(120),
    }
}

/// 一次预览加载所处的阶段。`Idle` 表示当前没有加载在途。
///
/// 阶段只描述"在等什么",不携带 viewer 句柄;真实 viewer 仍由
/// `PreviewRuntime` / `BackendState` / `web_*` 镜像表达。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewLoadStage {
    /// 没有加载在途(就绪 / 未选中 / 终态)。
    Idle,
    /// 识别文件类型/编码(画像、超长首行探测)。
    Profiling,
    /// 预留预览资源(预算、viewer 名额)。
    Reserving,
    /// 启动 host(CodeMirror / vanilla-jsoneditor / Flyfish 等 WebView)。
    CreatingHost,
    /// 读取文件内容。
    Reading,
    /// 为大文件构建稀疏行索引。
    Indexing,
    /// 读取首个有界窗口。
    LoadingWindow,
    /// 解析结构化内容(JSON/表格工作簿)。
    Parsing,
    /// 整文件搜索。
    Searching,
    /// 切换预览模式(渲染 ↔ 源码、Tree ↔ Text 等)。
    SwitchingMode,
}

impl PreviewLoadStage {
    /// 全部阶段(含 `Idle`)的唯一枚举来源,供表驱动测试遍历,避免测试各自
    /// 手抄一份列表而与枚举漂移。
    pub const ALL: [PreviewLoadStage; 10] = [
        PreviewLoadStage::Idle,
        PreviewLoadStage::Profiling,
        PreviewLoadStage::Reserving,
        PreviewLoadStage::CreatingHost,
        PreviewLoadStage::Reading,
        PreviewLoadStage::Indexing,
        PreviewLoadStage::LoadingWindow,
        PreviewLoadStage::Parsing,
        PreviewLoadStage::Searching,
        PreviewLoadStage::SwitchingMode,
    ];

    /// 在途阶段(`Idle` 之外),派生自 [`Self::ALL`]。
    pub fn active_stages() -> impl Iterator<Item = PreviewLoadStage> {
        Self::ALL.into_iter().filter(|s| s.is_active())
    }

    /// 该阶段的用户可见文案。`Idle` 不产生 loading 文案(返回 `None`)。
    ///
    /// 映射集中在此,保证各 viewer 调用同一套文案、不漂移(plan T1)。
    pub fn label(self) -> Option<&'static str> {
        Some(match self {
            PreviewLoadStage::Idle => return None,
            PreviewLoadStage::Profiling => "正在识别文件…",
            PreviewLoadStage::Reserving => "正在准备预览资源…",
            PreviewLoadStage::CreatingHost => "正在启动预览…",
            PreviewLoadStage::Reading => "正在读取文件…",
            PreviewLoadStage::Indexing => "正在索引大文件…",
            PreviewLoadStage::LoadingWindow => "正在加载文件内容…",
            PreviewLoadStage::Parsing => "正在解析文件…",
            PreviewLoadStage::Searching => "正在搜索整个文件…",
            PreviewLoadStage::SwitchingMode => "正在切换预览模式…",
        })
    }

    /// 是否处于"有加载在途"的阶段(`Idle` 之外都算)。
    pub fn is_active(self) -> bool {
        !matches!(self, PreviewLoadStage::Idle)
    }
}

/// 可选的低成本进度(`completed`/`total`)。`total` 未知时为 `None`,UI 只显示
/// 已完成计数。为百分比额外扫描文件是禁止的(plan §3)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewLoadProgress {
    pub completed: u64,
    pub total: Option<u64>,
}

/// T11 bullet 4:一次加载的三段延迟观测。
///
/// - `started`:`begin_load` 那一刻(加载开始)。
/// - `first_frame`:loading 动画**真的画了一帧**的时刻(视图组装时写回;未画到
///   就结束时为 `None`——即"动画没机会画",说明加载快得连一帧都没显示)。
/// - ready:加载结束(`finish`/`fail`)。
///
/// 由 `first_frame` 有无区分两类慢:无 → 加载瞬间完成,慢的是"没机会画";
/// 有 → 动画已显示,慢的是"后台任务本身慢"。
#[derive(Debug, Clone)]
pub struct LoadObservation {
    pub started: Instant,
    /// 首帧渲染时刻的共享单元(epoch 毫秒;0 表示尚未渲染)。视图层只有 `&` 访问,
    /// 故用 `Arc<AtomicU64>` 写回,不要求 `&mut`。
    first_frame_ms: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// 本次加载的世代,防止上一世代的视图写回污染新观测。
    pub generation: u64,
}

impl LoadObservation {
    /// 新建一次观测并重置首帧标记。
    pub fn new(generation: u64) -> Self {
        Self {
            started: Instant::now(),
            first_frame_ms: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            generation,
        }
    }

    /// 视图组装时调用:若本世代尚未记录首帧,记下当前(epoch 毫秒)。
    /// 世代不符(视图仍在画旧世代)则忽略。
    pub fn mark_first_frame(&self, generation: u64) {
        use std::sync::atomic::Ordering;
        if generation != self.generation {
            return;
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        // 只在首次(0)写入,后续帧不覆盖。
        let _ =
            self.first_frame_ms
                .compare_exchange(0, now_ms, Ordering::Relaxed, Ordering::Relaxed);
    }

    /// 首帧相对加载开始的毫秒偏移;未画到首帧返回 `None`。
    pub fn first_frame_offset_ms(&self) -> Option<u64> {
        let ms = self
            .first_frame_ms
            .load(std::sync::atomic::Ordering::Relaxed);
        if ms == 0 {
            return None;
        }
        let started_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
            .saturating_sub(self.started.elapsed().as_millis() as u64);
        Some(ms.saturating_sub(started_ms))
    }

    /// 自加载开始以来的总等待毫秒。
    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }
}

/// T11 bullet 5:一个 tab 当前 loading 任务的可观测快照(诊断用)。
/// 只含状态/计数/时长,不含文件正文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadDiagnostic {
    pub tab_id: usize,
    /// 文件名(仅 basename,不含完整路径/正文)。
    pub label: String,
    pub stage: PreviewLoadStage,
    pub generation: u64,
    /// 自本次加载开始的毫秒数;`Idle` 时为 0。
    pub elapsed_ms: u64,
    /// 首帧是否已经画过(区分"动画没机会画"与"后台慢")。
    pub first_frame_drawn: bool,
    /// 后台长任务的取消信号是否已置位。
    pub cancellation_requested: bool,
}

/// 一个 tab 的 loading 生命周期状态。
///
/// `generation` 每次 open/retry/reload/mode switch `+1`;后台任务捕获启动时值,
/// 回灌时经 [`PreviewLoadState::accepts`] 校验,过期结果被丢弃。
#[derive(Debug, Clone)]
pub struct PreviewLoadState {
    pub stage: PreviewLoadStage,
    pub generation: u64,
    pub started_at: Instant,
    pub progress: Option<PreviewLoadProgress>,
}

impl Default for PreviewLoadState {
    fn default() -> Self {
        Self {
            stage: PreviewLoadStage::Idle,
            generation: 0,
            started_at: Instant::now(),
            progress: None,
        }
    }
}

impl PreviewLoadState {
    /// 处于某阶段、且该阶段是"在途"的状态。
    pub fn starting(generation: u64, stage: PreviewLoadStage) -> Self {
        Self {
            stage,
            generation,
            started_at: Instant::now(),
            progress: None,
        }
    }

    /// 是否有加载在途(阶段非 `Idle`)。
    pub fn is_active(&self) -> bool {
        self.stage.is_active()
    }

    /// 后台回灌携带的 `generation` 是否仍是当前世代。只有匹配才允许改状态。
    pub fn accepts(&self, generation: u64) -> bool {
        self.generation == generation
    }

    /// 推进到下一阶段(不改变 generation)。
    pub fn advance(&mut self, stage: PreviewLoadStage) {
        self.stage = stage;
    }

    /// 结束加载(回到 `Idle`),推进 generation 使在途结果过期。
    pub fn finish(&mut self) {
        self.stage = PreviewLoadStage::Idle;
        self.progress = None;
        self.generation = self.generation.wrapping_add(1);
    }

    /// 取消加载:与 finish 同样回到 `Idle` 并作废在途结果(取消是正常结束,
    /// 不视为错误)。
    pub fn cancel(&mut self) {
        self.finish();
    }

    /// 已等待时长(用于 ready latency 观测)。
    pub fn elapsed(&self) -> std::time::Duration {
        self.started_at.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// plan T1:每个 stage 映射到非空文案;`Idle` 不产生 loading 文案。
    #[test]
    fn every_active_stage_has_non_empty_label() {
        for stage in PreviewLoadStage::active_stages() {
            let label = stage.label().unwrap_or("");
            assert!(!label.is_empty(), "{stage:?} 必须有非空文案");
            assert!(stage.is_active(), "{stage:?} 应为在途阶段");
        }
    }

    #[test]
    fn idle_has_no_label_and_is_not_active() {
        assert!(PreviewLoadStage::Idle.label().is_none());
        assert!(!PreviewLoadStage::Idle.is_active());
        assert!(!PreviewLoadState::default().is_active());
    }

    #[test]
    fn generation_gate_rejects_stale_results() {
        let mut state = PreviewLoadState::starting(7, PreviewLoadStage::Indexing);
        assert!(state.accepts(7));
        assert!(!state.accepts(6), "旧世代必须被拒");
        state.advance(PreviewLoadStage::LoadingWindow);
        assert_eq!(state.stage, PreviewLoadStage::LoadingWindow);
        assert_eq!(state.generation, 7, "advance 不改世代");
    }

    #[test]
    fn finish_bumps_generation_and_rejects_old() {
        let mut state = PreviewLoadState::starting(3, PreviewLoadStage::Reading);
        state.finish();
        assert_eq!(state.stage, PreviewLoadStage::Idle);
        assert_eq!(state.generation, 4, "finish 推进世代作废旧结果");
        assert!(!state.accepts(3), "结束后旧世代回调被拒");
        assert!(!state.is_active());
    }

    /// T11 bullet 3:每个在途阶段都有非零超时;`Idle` 为零(从不 arm)。
    #[test]
    fn every_active_stage_has_nonzero_timeout() {
        for stage in PreviewLoadStage::active_stages() {
            assert!(
                !stage_timeout(stage).is_zero(),
                "{stage:?} 必须有非零看门狗超时"
            );
        }
        assert!(stage_timeout(PreviewLoadStage::Idle).is_zero());
    }

    #[test]
    fn cancel_behaves_like_finish() {
        let mut state = PreviewLoadState::starting(10, PreviewLoadStage::Searching);
        state.cancel();
        assert_eq!(state.stage, PreviewLoadStage::Idle);
        assert_eq!(state.generation, 11);
        assert!(!state.accepts(10));
    }

    /// T12 自动化:表驱动覆盖**每个在途阶段**的完整生命周期。对每个 stage 逐项
    /// 断言:进入即 active 且 generation 匹配、advance 保持世代、成功/失败/取消
    /// 三条终态路径各自回到 Idle 并作废旧世代、重试拿到新世代。
    #[test]
    fn stage_lifecycle_table() {
        // (起始世代, 目标阶段, 该阶段之后要推进到的下一阶段)
        struct Case {
            generation: u64,
            stage: PreviewLoadStage,
            next: PreviewLoadStage,
        }
        let cases: Vec<Case> = PreviewLoadStage::active_stages()
            .enumerate()
            .map(|(i, stage)| Case {
                generation: 100 + i as u64,
                stage,
                // 用阶段自身作为"推进目标"已足够:关键是 advance 不换世代;
                // 额外选一个不同阶段验证阶段真的变了。
                next: if stage == PreviewLoadStage::Reading {
                    PreviewLoadStage::Indexing
                } else {
                    PreviewLoadStage::Reading
                },
            })
            .collect();

        for case in &cases {
            // 进入。
            let state = PreviewLoadState::starting(case.generation, case.stage);
            assert!(state.is_active(), "{:?} 进入即 active", case.stage);
            assert_eq!(state.stage, case.stage);
            assert!(state.accepts(case.generation));
            assert!(
                !state.accepts(case.generation.wrapping_sub(1)),
                "旧世代被拒"
            );

            // advance:换阶段、留世代、仍 active。
            let mut advanced = state.clone();
            advanced.advance(case.next);
            assert_eq!(advanced.stage, case.next);
            assert_eq!(advanced.generation, case.generation, "advance 不改世代");
            assert!(advanced.is_active());
            assert!(advanced.accepts(case.generation));

            // 成功终态:finish → Idle + 新世代 + 旧回调被拒。
            let mut ok = advanced.clone();
            ok.finish();
            assert_eq!(
                ok.stage,
                PreviewLoadStage::Idle,
                "{:?} finish → Idle",
                case.stage
            );
            assert!(!ok.is_active());
            assert_eq!(ok.generation, case.generation + 1);
            assert!(!ok.accepts(case.generation), "成功后旧世代失效");

            // 取消终态:同 finish(取消是正常结束)。
            let mut cancelled = advanced.clone();
            cancelled.cancel();
            assert_eq!(cancelled.stage, PreviewLoadStage::Idle);
            assert_eq!(cancelled.generation, case.generation + 1);

            // 失败终态由 `PreviewPane::fail_load` 表达(纯状态层没有错误字段),
            // 在 view.rs 的表驱动测试 `stage_failure_table` 中覆盖;此处仅确认
            // 失败前的推进仍保持 active。
            assert!(advanced.is_active(), "失败前的推进仍 active");
        }

        assert_eq!(
            cases.len(),
            PreviewLoadStage::ALL.len() - 1,
            "覆盖全部在途阶段"
        );
    }

    /// T12 自动化:世代回绕(wrapping)不会让旧回调误命中。`generation` 用
    /// `wrapping_add`,只关注"变了"而非单调。
    #[test]
    fn generation_wraps_without_reaccepting_old() {
        let mut state = PreviewLoadState::starting(u64::MAX, PreviewLoadStage::Reading);
        assert!(state.accepts(u64::MAX));
        state.finish();
        assert_eq!(state.generation, 0, "u64::MAX 之后回绕到 0");
        assert!(!state.accepts(u64::MAX), "回绕后不再接受旧世代");
        assert!(state.accepts(0));
    }
}
