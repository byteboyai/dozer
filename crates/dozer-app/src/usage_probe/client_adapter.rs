//! `Client` 作为 `UsageService` 的适配行为:用 `FakeDozerd` 讲真实协议,覆盖成功 / 断开 / 被拒。
use super::fake_dozerd::{Behavior, FakeDozerd};
use crate::panel_host::testing::{TIMEOUT, runtime};
use dozer_services::{ServiceErrorKind, UsageService};

fn call(behavior: Behavior) -> Result<usize, (ServiceErrorKind, String)> {
    let rt = runtime();
    let dozerd = FakeDozerd::start(&rt, behavior);
    let client = dozer_client::Client::new(dozerd.socket().to_path_buf());
    rt.block_on(async {
        tokio::time::timeout(TIMEOUT, client.usage_summary("/p".into(), None)).await
    })
    .unwrap()
    .map(|rows| rows.len())
    .map_err(|e| (e.kind, e.message))
}

#[test]
fn rows_come_back_as_ok() {
    assert_eq!(call(Behavior::Rows(vec![])), Ok(0));
}

#[test]
fn hang_up_is_unavailable_with_the_legacy_text() {
    let (kind, msg) = call(Behavior::HangUp).unwrap_err();
    assert_eq!(kind, ServiceErrorKind::Unavailable);
    assert_eq!(msg, "daemon 断开");
}

#[test]
fn error_reply_is_rejected_with_the_legacy_prefix() {
    let (kind, msg) = call(Behavior::ErrorReply("boom".into())).unwrap_err();
    assert_eq!(kind, ServiceErrorKind::Rejected);
    assert_eq!(msg, "daemon 错误: boom");
}

#[test]
fn nonexistent_socket_is_unavailable() {
    let rt = runtime();
    let client = dozer_client::Client::new("/tmp/dozer-x0-no-such.sock".into());
    let e = rt
        .block_on(client.usage_summary("/p".into(), None))
        .unwrap_err();
    assert_eq!(e.kind, ServiceErrorKind::Unavailable);
}
