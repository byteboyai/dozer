//! Agent 面板的本地化 owner 与错误 key。
//!
//! 四个错误 key 原在 `dozer-app` 的 `workspace/state.rs` 里以
//! `LocalizedText::keyed(HOST_OWNER, key, 英文默认值)` 使用(host 语言包里
//! 没有这些 key,实际总是回落到默认值)。S6c-1 把调用点搬进本库后 owner
//! 改为 `dozer.agent`;key 与默认文案原样保留,用户可见文本不变。
//!
//! 注意:`dozer.agent` 这个 owner 的 locale 资源仍由 `dozer-app` 的
//! `locales/panels/agent/*.json` 注册(agent_context 也在用同一个 owner)。
//! 本库不另建 `locales/`——同 owner 两份资源会互相覆盖。

use dozer_model::LocalizedText;

pub const AGENT_OWNER: &str = "dozer.agent";

pub const ERROR_NEW_SESSION_FAILED: &str = "error.new_session_failed";
pub const ERROR_ATTACH_SESSION_FAILED: &str = "error.attach_session_failed";
pub const ERROR_TERM_INITIAL_WRITE_FAILED: &str = "error.term_initial_write_failed";
pub const ERROR_TERM_DISPATCH_WRITE_FAILED: &str = "error.term_dispatch_write_failed";

/// 新建会话失败(英文默认值与抽库前逐字一致)。
pub fn new_session_failed(error: impl ToString) -> LocalizedText {
    LocalizedText::keyed(
        AGENT_OWNER,
        ERROR_NEW_SESSION_FAILED,
        "Could not create a new session: {error}",
    )
    .with_arg("error", error.to_string())
}

/// attach 新会话失败。
pub fn attach_session_failed(error: impl ToString) -> LocalizedText {
    LocalizedText::keyed(
        AGENT_OWNER,
        ERROR_ATTACH_SESSION_FAILED,
        "Could not attach the new session: {error}",
    )
    .with_arg("error", error.to_string())
}

/// 启动命令写入失败。
pub fn term_initial_write_failed(error: impl ToString) -> LocalizedText {
    LocalizedText::keyed(
        AGENT_OWNER,
        ERROR_TERM_INITIAL_WRITE_FAILED,
        "Could not write the launch command to the terminal: {error}",
    )
    .with_arg("error", error.to_string())
}

/// 跟随文本(待办派发)写入失败。
pub fn term_dispatch_write_failed(error: impl ToString) -> LocalizedText {
    LocalizedText::keyed(
        AGENT_OWNER,
        ERROR_TERM_DISPATCH_WRITE_FAILED,
        "Could not write the task text to the terminal: {error}",
    )
    .with_arg("error", error.to_string())
}
