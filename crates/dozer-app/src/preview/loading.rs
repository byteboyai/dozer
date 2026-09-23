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
        for stage in [
            PreviewLoadStage::Profiling,
            PreviewLoadStage::Reserving,
            PreviewLoadStage::CreatingHost,
            PreviewLoadStage::Reading,
            PreviewLoadStage::Indexing,
            PreviewLoadStage::LoadingWindow,
            PreviewLoadStage::Parsing,
            PreviewLoadStage::Searching,
            PreviewLoadStage::SwitchingMode,
        ] {
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

    #[test]
    fn cancel_behaves_like_finish() {
        let mut state = PreviewLoadState::starting(10, PreviewLoadStage::Searching);
        state.cancel();
        assert_eq!(state.stage, PreviewLoadStage::Idle);
        assert_eq!(state.generation, 11);
        assert!(!state.accepts(10));
    }
}
