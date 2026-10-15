//! App 侧语言快照控制器(spec 2026-10-12-dozer-i18n-design §4.2/§4.3)。
//!
//! 纯状态机,不碰 iced/网络/平台——App 负责发起 `LocaleService` 读取并把结果
//! 交回来,所有"该不该发、这条回包还算不算数、要不要推进显示代"的判断都在
//! 这里,便于在单窗口测试宿主里独立验证(不引用全局可变 `App`)。
//!
//! 关键概念:
//! - **epoch(连接代)**:daemon 每次重连 +1。同代内按 `revision` 收敛(较旧
//!   revision 不回退);换代码=建立新基线,避免重连后较小 revision 被永久拒绝
//!   (spec §4.3)。
//! - **seq(请求序号)**:每次发起读取 +1。回包必须带发起时的 seq,`seq` 不是
//!   当前在途值即丢弃——迟到的旧回包不能覆盖刚确认的新语言(spec §4.3)。
//! - **display_generation(显示代)**:仅在 `active_locale` **真正变化**时 +1,
//!   供视图/webview 侧判断"要不要重绘/重推",避免无变化轮询引发风暴(spec §4.3)。
//!
//! 离线策略(spec §4.2):缓存只保留**最后一次确认**的快照;daemon 不可用时
//! 用缓存,无缓存用系统探测值**临时**显示(标记来源,不伪装成已保存)。

use std::time::{Duration, Instant};

use dozer_model::{Locale, LocalePreference, LocaleSnapshot};

/// 跨进程读取的最小间隔:运行期间至多每 2 秒一次(spec §4.3)。
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// Bound the complete read/reconcile operation so an unresponsive daemon cannot
/// leave refreshes permanently in flight.
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) async fn fetch_locale(
    service: &impl dozer_services::LocaleService,
    candidates: Vec<String>,
) -> FetchOutcome {
    fetch_locale_with_timeout(service, candidates, FETCH_TIMEOUT).await
}

async fn fetch_locale_with_timeout(
    service: &impl dozer_services::LocaleService,
    candidates: Vec<String>,
    timeout: Duration,
) -> FetchOutcome {
    let fetch = async {
        let snapshot = service.get_locale().await?;
        let system_locale = dozer_i18n::resolve_from_candidates(&candidates);
        if snapshot.preference == LocalePreference::System
            && snapshot.active_locale.as_str() != system_locale
        {
            // CAS protects an explicit preference selected in another window
            // between this read and write. A conflict is authoritative too.
            match service
                .set_locale(LocalePreference::System, snapshot.revision, candidates)
                .await?
            {
                dozer_model::LocaleWriteOutcome::Applied { snapshot }
                | dozer_model::LocaleWriteOutcome::Conflict { snapshot } => Ok(snapshot),
            }
        } else {
            Ok::<_, dozer_services::ServiceError>(snapshot)
        }
    };
    match tokio::time::timeout(timeout, fetch).await {
        Ok(Ok(snapshot)) => FetchOutcome::Ok(snapshot),
        Ok(Err(_)) | Err(_) => FetchOutcome::Unavailable,
    }
}

/// 快照来源:仅用于判断"当前显示的是不是权威值"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotSource {
    /// daemon 持久化的权威快照。
    Daemon,
    /// daemon 不可用时的本地缓存(最后一次确认的权威值)。
    Cached,
    /// 无缓存且 daemon 不可用时的系统探测临时值(未保存)。
    SystemTemporary,
    /// 什么都没拿到时的默认(en)。
    Default,
}

/// 一次读取的结果。
#[derive(Debug, Clone)]
pub enum FetchOutcome {
    /// 成功拿到 daemon 权威快照。
    Ok(LocaleSnapshot),
    /// daemon 不可用(离线/断连)。
    Unavailable,
}

/// 应用一次确认后的动作:调用方据此决定要不要重绘/清缓存。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyEffect {
    /// `active_locale` 是否发生变化(据此 +1 display generation)。
    pub active_changed: bool,
    /// display generation 的新值。
    pub generation: u64,
}

pub struct LocaleController {
    snapshot: LocaleSnapshot,
    source: SnapshotSource,
    display_generation: u64,
    epoch: u64,
    next_seq: u64,
    /// 当前在途读取的 seq;`None` = 没有请求在飞(可发起新读取)。
    in_flight: Option<u64>,
    /// 上一个被采纳快照所属的 epoch。用于"换代码=建立新基线":当 epoch 与
    /// 上次采纳时不同,允许 revision 回退(新代码的权威数据可能来自恢复)。
    applied_epoch: u64,
    /// 上次真正发起读取的时刻(用于 2s 限速)。
    last_fetch_at: Option<Instant>,
    /// 被合并的请求计数(诊断/测试用):在途时又到来的刷新请求数。
    coalesced: u64,
}

impl Default for LocaleController {
    fn default() -> Self {
        Self::new()
    }
}

impl LocaleController {
    /// 初始控制器:默认快照(System/en/revision 0),未连接(epoch 0)。
    ///
    /// 调用方可在启动时用缓存/系统探测结果覆盖初始显示,见
    /// [`Self::adopt_initial`]。
    pub fn new() -> Self {
        LocaleController {
            snapshot: LocaleSnapshot::default_snapshot(),
            source: SnapshotSource::Default,
            display_generation: 0,
            epoch: 0,
            next_seq: 1,
            in_flight: None,
            applied_epoch: 0,
            last_fetch_at: None,
            coalesced: 0,
        }
    }

    /// 启动时注入一个初始显示快照(缓存或系统探测临时值),不推进 display
    /// generation、不视为已确认权威值。仅在真正没有任何输入时保留默认。
    pub fn adopt_initial(&mut self, snapshot: LocaleSnapshot, source: SnapshotSource) {
        let changed = self.snapshot.active_locale != snapshot.active_locale;
        self.snapshot = snapshot;
        self.source = source;
        if changed {
            // 初始显示的切换也应让视图刷新一次(生成代从 0 起)。
            self.display_generation += 1;
        }
    }

    pub fn snapshot(&self) -> &LocaleSnapshot {
        &self.snapshot
    }

    /// 当前生效语言(渲染用)。
    pub fn active_locale(&self) -> &str {
        self.snapshot.active_locale.as_str()
    }

    pub fn source(&self) -> SnapshotSource {
        self.source
    }

    /// 显示代:仅在生效语言真正变化时 +1。Task 05+ 的设置页/webview 会消费它
    /// 判断是否需要重绘/重推;当前仅测试与诊断读取。
    #[allow(dead_code)]
    pub fn display_generation(&self) -> u64 {
        self.display_generation
    }

    /// 当前连接代。仅供诊断与测试。
    #[allow(dead_code)]
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// 是否有读取在途(诊断/测试)。
    #[allow(dead_code)]
    pub fn has_in_flight(&self) -> bool {
        self.in_flight.is_some()
    }

    #[cfg(test)]
    pub(crate) fn coalesced_count(&self) -> u64 {
        self.coalesced
    }

    /// daemon 重连:推进 epoch,作废在途请求,强制下一次读取立即发生。
    ///
    /// 新代码 = 新 revision 基线(spec §4.3):不沿用旧代"只接受更大 revision"
    /// 的约束,daemon 数据恢复后的较小 revision 不会被永久拒绝。
    pub fn on_reconnect(&mut self) {
        self.epoch += 1;
        self.in_flight = None;
        self.last_fetch_at = None;
    }

    /// 窗口获焦:请求立即刷新(绕过 2s 限速)。
    pub fn request_immediate(&mut self) {
        self.last_fetch_at = None;
    }

    /// 是否现在应发起一次跨进程读取。
    ///
    /// 在途时**不**发起(合并,spec §4.3),返回 `None` 并记一次合并;
    /// 距上次发起不足 `REFRESH_INTERVAL` 且未请求立即刷新时返回 `None`。
    pub fn begin_fetch(&mut self, now: Instant) -> Option<u64> {
        if self.in_flight.is_some() {
            self.coalesced += 1;
            return None;
        }
        if let Some(last) = self.last_fetch_at
            && now.duration_since(last) < REFRESH_INTERVAL
        {
            return None;
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.in_flight = Some(seq);
        self.last_fetch_at = Some(now);
        Some(seq)
    }

    /// 下一次周期性读取应在多久之后(`about_to_wait` 排唤醒用)。
    ///
    /// 在途时有回包在等,不额外排;否则返回距 `last_fetch_at` 满
    /// `REFRESH_INTERVAL` 的剩余时间(从未发过则立即)。
    pub fn next_wake(&self, now: Instant) -> Option<Duration> {
        if self.in_flight.is_some() {
            return None;
        }
        Some(match self.last_fetch_at {
            None => Duration::ZERO,
            Some(last) => REFRESH_INTERVAL.saturating_sub(now.duration_since(last)),
        })
    }

    /// 应用一次读取回包。
    ///
    /// - `seq` 必须等于当前在途 seq,否则是被更新的读取取代的迟到回包,丢弃
    ///   (返回 `None`)。
    /// - 同代码内,`Ok` 的 revision 小于当前已确认值时丢弃(永不回退);相等
    ///   视为无变化。
    /// - `Unavailable` 只清在途、保留当前快照(离线沿用缓存/临时值)。
    /// - `active_locale` 真正变化时 +1 display generation。
    pub fn apply_fetch(&mut self, seq: u64, outcome: FetchOutcome) -> Option<ApplyEffect> {
        if self.in_flight != Some(seq) {
            return None; // 迟到/被取代的回包
        }
        self.in_flight = None;
        match outcome {
            FetchOutcome::Unavailable => None,
            FetchOutcome::Ok(snapshot) => {
                // 同代码收敛:较旧 revision 不回退(spec §4.3)。daemon 在同一
                // 连接内 revision 单调;若拿到更小值多半是乱序残留。换代码时
                // (`epoch != applied_epoch`)建立新基线,允许 revision 回退。
                let same_epoch = self.applied_epoch == self.epoch;
                if same_epoch && snapshot.revision < self.snapshot.revision {
                    return None;
                }
                let active_changed = self.snapshot.active_locale != snapshot.active_locale;
                let preference_changed = self.snapshot.preference != snapshot.preference;
                self.snapshot = snapshot;
                self.source = SnapshotSource::Daemon;
                self.applied_epoch = self.epoch;
                if active_changed {
                    self.display_generation += 1;
                } else if preference_changed {
                    // 偏好变了但生效语言没变(System→Explicit(同语言)):
                    // 不需要重绘,但快照已更新,交给调用方决定是否刷新设置页。
                }
                Some(ApplyEffect {
                    active_changed,
                    generation: self.display_generation,
                })
            }
        }
    }

    /// 应用一次**本地发起**的写入结果(设置页改语言)。
    ///
    /// 与 `apply_fetch` 的差别:本地写入的权威性不靠 revision 比较(用户刚提交,
    /// 结果里带的就是最新快照),直接采纳并更新基线;同时作废任何在途读取的旧
    /// 回包(写入已确立新值,之前发起的读取是旧的)。`Conflict` 也带回了服务端
    /// 当前快照,同样采纳(spec §4.2:冲突时以服务端为准,提示重选)。
    ///
    /// 返回 `ApplyEffect`(active 是否变化 + 新显示代),调用方据此广播/清缓存。
    pub fn apply_local_write(&mut self, snapshot: LocaleSnapshot) -> ApplyEffect {
        self.in_flight = None;
        let active_changed = self.snapshot.active_locale != snapshot.active_locale;
        self.snapshot = snapshot;
        self.source = SnapshotSource::Daemon;
        self.applied_epoch = self.epoch;
        if active_changed {
            self.display_generation += 1;
        }
        ApplyEffect {
            active_changed,
            generation: self.display_generation,
        }
    }

    /// 测试/诊断:直接读取当前偏好。
    #[allow(dead_code)]
    pub fn preference(&self) -> &LocalePreference {
        &self.snapshot.preference
    }

    /// 用默认/系统临时值覆盖显示(无缓存且 daemon 离线时用)。
    #[allow(dead_code)]
    pub fn show_system_temporary(&mut self, locale: Locale) {
        let changed = self.snapshot.active_locale != locale;
        self.snapshot.active_locale = locale;
        self.source = SnapshotSource::SystemTemporary;
        if changed {
            self.display_generation += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_model::LocalePreference;

    fn snap(rev: u64, active: &str, pref: LocalePreference) -> LocaleSnapshot {
        LocaleSnapshot {
            schema_version: 1,
            preference: pref,
            active_locale: Locale(active.into()),
            revision: rev,
        }
    }

    fn explicit(loc: &str) -> LocalePreference {
        LocalePreference::Explicit {
            locale: Locale(loc.into()),
        }
    }

    #[test]
    fn pending_fetch_suspends_wakes_and_failure_allows_retry() {
        let mut c = LocaleController::new();
        let now = Instant::now();
        let seq = c.begin_fetch(now).unwrap();
        let later = now + Duration::from_secs(10);
        assert_eq!(c.next_wake(later), None);
        c.request_immediate();
        assert_eq!(c.next_wake(later), None);
        c.apply_fetch(seq, FetchOutcome::Unavailable);
        assert_eq!(c.next_wake(later), Some(Duration::ZERO));
        assert!(c.begin_fetch(later).is_some());
    }

    struct LocaleServiceStub {
        snapshot: LocaleSnapshot,
        conflict: Option<LocaleSnapshot>,
        writes: std::sync::atomic::AtomicUsize,
        stall_read: bool,
        stall_write: bool,
    }

    impl Default for LocaleServiceStub {
        fn default() -> Self {
            Self {
                snapshot: LocaleSnapshot::default_snapshot(),
                conflict: None,
                writes: std::sync::atomic::AtomicUsize::new(0),
                stall_read: false,
                stall_write: false,
            }
        }
    }

    impl dozer_services::LocaleService for LocaleServiceStub {
        fn get_locale(
            &self,
        ) -> dozer_services::BoxFuture<'_, Result<LocaleSnapshot, dozer_services::ServiceError>>
        {
            Box::pin(async move {
                if self.stall_read {
                    std::future::pending::<()>().await;
                }
                Ok(self.snapshot.clone())
            })
        }

        fn set_locale(
            &self,
            preference: LocalePreference,
            expected_revision: u64,
            candidates: Vec<String>,
        ) -> dozer_services::BoxFuture<
            '_,
            Result<dozer_model::LocaleWriteOutcome, dozer_services::ServiceError>,
        > {
            Box::pin(async move {
                self.writes
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                assert_eq!(preference, LocalePreference::System);
                assert_eq!(expected_revision, self.snapshot.revision);
                if self.stall_write {
                    std::future::pending::<()>().await;
                }
                if let Some(snapshot) = &self.conflict {
                    return Ok(dozer_model::LocaleWriteOutcome::Conflict {
                        snapshot: snapshot.clone(),
                    });
                }
                Ok(dozer_model::LocaleWriteOutcome::Applied {
                    snapshot: snap(
                        expected_revision + 1,
                        dozer_i18n::resolve_from_candidates(candidates),
                        preference,
                    ),
                })
            })
        }
    }

    #[tokio::test]
    async fn first_launch_reconciles_system_language_before_display() {
        let service = LocaleServiceStub::default();
        let FetchOutcome::Ok(snapshot) = fetch_locale(&service, vec!["zh-CN".into()]).await else {
            panic!("expected confirmed snapshot");
        };
        assert_eq!(snapshot, snap(1, "zh-CN", LocalePreference::System));
    }

    #[tokio::test]
    async fn explicit_or_matching_system_preference_does_not_write() {
        for snapshot in [
            snap(3, "en", explicit("en")),
            snap(4, "zh-CN", LocalePreference::System),
        ] {
            let service = LocaleServiceStub {
                snapshot: snapshot.clone(),
                ..Default::default()
            };
            let FetchOutcome::Ok(actual) = fetch_locale(&service, vec!["zh-CN".into()]).await
            else {
                panic!("expected confirmed snapshot");
            };
            assert_eq!(actual, snapshot);
            assert_eq!(service.writes.load(std::sync::atomic::Ordering::Relaxed), 0);
        }
    }

    #[tokio::test]
    async fn system_reconciliation_preserves_concurrent_explicit_choice() {
        let chosen = snap(1, "en", explicit("en"));
        let service = LocaleServiceStub {
            conflict: Some(chosen.clone()),
            ..Default::default()
        };
        let FetchOutcome::Ok(actual) = fetch_locale(&service, vec!["zh-CN".into()]).await else {
            panic!("expected conflict snapshot");
        };
        assert_eq!(actual, chosen);
        assert_eq!(service.writes.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn stalled_read_and_reconciliation_both_time_out() {
        for stall_read in [true, false] {
            let service = LocaleServiceStub {
                stall_read,
                stall_write: !stall_read,
                ..Default::default()
            };
            assert!(matches!(
                fetch_locale_with_timeout(
                    &service,
                    vec!["zh-CN".into()],
                    Duration::from_millis(10)
                )
                .await,
                FetchOutcome::Unavailable
            ));
        }
    }

    #[test]
    fn default_is_system_en_rev0() {
        let c = LocaleController::new();
        assert_eq!(c.active_locale(), "en");
        assert_eq!(c.snapshot().revision, 0);
        assert_eq!(c.source(), SnapshotSource::Default);
        assert_eq!(c.display_generation(), 0);
    }

    #[test]
    fn cadence_limits_to_refresh_interval() {
        let mut c = LocaleController::new();
        let t0 = Instant::now();
        let seq1 = c.begin_fetch(t0).expect("首次应发起");
        assert!(c.has_in_flight());
        // 在途:再次请求被合并。
        assert_eq!(c.begin_fetch(t0), None);
        assert_eq!(c.coalesced_count(), 1);
        // 回包落地后,2s 内不再发起。
        c.apply_fetch(seq1, FetchOutcome::Unavailable);
        assert_eq!(c.begin_fetch(t0 + Duration::from_millis(500)), None);
        // 满 2s 后可再次发起。
        let seq2 = c
            .begin_fetch(t0 + REFRESH_INTERVAL)
            .expect("满 2s 应可再次发起");
        assert!(seq2 > seq1);
    }

    #[test]
    fn late_reply_with_wrong_seq_is_dropped() {
        let mut c = LocaleController::new();
        let t0 = Instant::now();
        let seq1 = c.begin_fetch(t0).unwrap();
        c.apply_fetch(seq1, FetchOutcome::Ok(snap(1, "zh-CN", explicit("zh-CN"))));
        // 再发起一次,拿到 seq2。
        let seq2 = c.begin_fetch(t0 + REFRESH_INTERVAL).unwrap();
        // 一个比 seq2 更旧的迟到回包(seq1)到达:应被丢弃。
        assert!(
            c.apply_fetch(seq1, FetchOutcome::Ok(snap(9, "en", explicit("en"))))
                .is_none()
        );
        assert_eq!(c.active_locale(), "zh-CN", "迟到旧回包不得覆盖");
        // seq2 的正常回包可落地。
        assert!(
            c.apply_fetch(seq2, FetchOutcome::Ok(snap(2, "zh-CN", explicit("zh-CN"))))
                .is_some()
        );
    }

    #[test]
    fn generation_only_advances_when_active_changes() {
        let mut c = LocaleController::new();
        let t0 = Instant::now();
        let seq = c.begin_fetch(t0).unwrap();
        // en -> en:无变化,不推进。
        let eff = c
            .apply_fetch(seq, FetchOutcome::Ok(snap(1, "en", explicit("en"))))
            .unwrap();
        assert!(!eff.active_changed);
        assert_eq!(eff.generation, 0);
        // en -> zh-CN:推进。
        let seq = c.begin_fetch(t0 + REFRESH_INTERVAL).unwrap();
        let eff = c
            .apply_fetch(seq, FetchOutcome::Ok(snap(2, "zh-CN", explicit("zh-CN"))))
            .unwrap();
        assert!(eff.active_changed);
        assert_eq!(eff.generation, 1);
    }

    #[test]
    fn same_epoch_never_regresses_revision() {
        let mut c = LocaleController::new();
        let t0 = Instant::now();
        let seq = c.begin_fetch(t0).unwrap();
        c.apply_fetch(seq, FetchOutcome::Ok(snap(5, "zh-CN", explicit("zh-CN"))));
        let seq = c.begin_fetch(t0 + REFRESH_INTERVAL).unwrap();
        // revision 更小的回包被丢弃。
        assert!(
            c.apply_fetch(seq, FetchOutcome::Ok(snap(3, "en", explicit("en"))))
                .is_none()
        );
        assert_eq!(c.active_locale(), "zh-CN");
        assert_eq!(c.snapshot().revision, 5);
    }

    #[test]
    fn reconnect_resets_baseline_and_forces_immediate_fetch() {
        let mut c = LocaleController::new();
        let t0 = Instant::now();
        let seq = c.begin_fetch(t0).unwrap();
        c.apply_fetch(seq, FetchOutcome::Ok(snap(5, "zh-CN", explicit("zh-CN"))));
        // 刚发过,2s 内不会再发。
        assert_eq!(c.begin_fetch(t0 + Duration::from_millis(100)), None);
        // 重连:epoch+1,允许立即发起,并接受较小 revision(新基线)。
        c.on_reconnect();
        assert_eq!(c.epoch(), 1);
        let seq = c.begin_fetch(t0 + Duration::from_millis(100)).unwrap();
        let eff = c
            .apply_fetch(seq, FetchOutcome::Ok(snap(1, "en", explicit("en"))))
            .unwrap();
        assert_eq!(c.active_locale(), "en");
        assert!(eff.active_changed);
    }

    #[test]
    fn unavailable_keeps_snapshot() {
        let mut c = LocaleController::new();
        let t0 = Instant::now();
        let seq = c.begin_fetch(t0).unwrap();
        c.apply_fetch(seq, FetchOutcome::Ok(snap(2, "zh-CN", explicit("zh-CN"))));
        let seq = c.begin_fetch(t0 + REFRESH_INTERVAL).unwrap();
        assert!(c.apply_fetch(seq, FetchOutcome::Unavailable).is_none());
        assert_eq!(c.active_locale(), "zh-CN", "离线沿用最后确认值");
        assert!(!c.has_in_flight());
    }

    #[test]
    fn adopt_initial_from_system_temporary() {
        let mut c = LocaleController::new();
        c.adopt_initial(
            LocaleSnapshot {
                schema_version: 1,
                preference: LocalePreference::System,
                active_locale: Locale("zh-CN".into()),
                revision: 0,
            },
            SnapshotSource::SystemTemporary,
        );
        assert_eq!(c.active_locale(), "zh-CN");
        assert_eq!(c.source(), SnapshotSource::SystemTemporary);
        assert_eq!(c.display_generation(), 1, "初始显示变化也应刷新一次");
    }
}
