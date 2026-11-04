//! Dozer 的 Git 账户适配层。
//!
//! 领域 DTO、provider endpoint / 鉴权 / 响应解析、校验 / 连接 / 断开 / 仓库列表
//! 以及 Keychain / 元数据读写全部来自 `bytehost-git-accounts`。本文件只保留
//! **Dozer 的注入点**:`dozer-git` Keychain service、`git_accounts.json` 路径,
//! 以及面向既有调用点的兼容函数 / 本地化错误。
//!
//! 不新增任何 Token 校验、Keychain 或 provider 解析的第二份实现。

use std::io;
use std::path::{Path, PathBuf};

use std::sync::Arc;

use bytehost_git_accounts::{
    AccountService, Admission, JsonFileMetadataStore, KeyringCredentialStore, MetadataStore,
    ReqwestTransport, SecretToken,
};

pub use bytehost_git_accounts::{
    ConnectedAccount, GitAccountError, GitAccountsState, GitProvider, RemoteRepo,
};

/// Dozer 的 Keychain service(保持历史命名,不迁移)。
pub const KEYCHAIN_SERVICE: &str = "dozer-git";

/// Dozer 元数据文件名(保持历史路径 / schema)。
pub const METADATA_FILE: &str = "git_accounts.json";

fn file_path() -> PathBuf {
    dozer_core::paths::config_dir().join(METADATA_FILE)
}

fn metadata() -> JsonFileMetadataStore {
    JsonFileMetadataStore::new(file_path())
}

fn credentials() -> KeyringCredentialStore {
    KeyringCredentialStore::new(KEYCHAIN_SERVICE)
}

pub type Service = AccountService<KeyringCredentialStore, JsonFileMetadataStore, ReqwestTransport>;

pub(crate) fn service() -> Service {
    AccountService::new(credentials(), metadata(), ReqwestTransport)
}

/// 同一产品(同一设置对话框 / 全部请求)共享的提交准入。每个异步任务
/// 新建的 `AccountService` 必须带上它,取消 / 乱序请求才不会继续落盘。
pub fn service_with_admission(admission: Arc<Admission>) -> Service {
    AccountService::new_with_admission(credentials(), metadata(), ReqwestTransport, admission)
}

pub fn load() -> GitAccountsState {
    load_from(&file_path())
}

pub fn save(state: &GitAccountsState) -> io::Result<()> {
    save_to(&file_path(), state)
}

/// 与旧实现一致:缺失 / 损坏文件按空状态处理(重开时 `query` 还会以凭据校正)。
fn load_from(path: &Path) -> GitAccountsState {
    JsonFileMetadataStore::new(path).load().unwrap_or_default()
}

fn save_to(path: &Path, state: &GitAccountsState) -> io::Result<()> {
    bytehost_git_accounts::MetadataStore::save(&JsonFileMetadataStore::new(path), state)
        .map_err(|error| io::Error::other(error.stable()))
}

/// 读不到按未连接处理,不 panic(同 SSH 面板的既有口径)。
pub fn get_token(provider: GitProvider) -> Option<String> {
    bytehost_git_accounts::CredentialStore::get(&credentials(), provider)
        .ok()
        .flatten()
        .map(|token| token.expose().to_owned())
}

pub fn set_token(provider: GitProvider, token: &str) -> Result<(), GitAccountError> {
    bytehost_git_accounts::CredentialStore::set(&credentials(), provider, &SecretToken::new(token))
}

/// 条目本来就不存在视为成功——断开一个从未真正连上的账户不报错。
pub fn delete_token(provider: GitProvider) -> Result<(), GitAccountError> {
    bytehost_git_accounts::CredentialStore::delete(&credentials(), provider)
}

pub(crate) fn extract_username(
    provider: GitProvider,
    json: &str,
) -> Result<String, GitAccountError> {
    provider
        .extract_username(json)
        .map(|account| account.username)
}

pub(crate) fn parse_repo_list(
    provider: GitProvider,
    json: &str,
) -> Result<Vec<RemoteRepo>, GitAccountError> {
    provider.parse_repo_list(json)
}

/// 校验 PAT 并返回用户名。
pub async fn validate_token(provider: GitProvider, token: &str) -> Result<String, GitAccountError> {
    service()
        .validate_token(provider, &SecretToken::new(token))
        .await
        .map(|account| account.username)
}

/// 拉取 provider 账户名下的个人仓库。
pub async fn list_repos(
    provider: GitProvider,
    token: &str,
) -> Result<Vec<RemoteRepo>, GitAccountError> {
    service()
        .list_repos(provider, &SecretToken::new(token))
        .await
}

/// 让既有调用点继续使用 `.localized()`;实现只有一份,映射到 Dozer 文案 key。
pub trait LocalizedGitError {
    fn localized(&self) -> dozer_model::LocalizedText;
}

impl LocalizedGitError for GitAccountError {
    fn localized(&self) -> dozer_model::LocalizedText {
        use dozer_model::LocalizedText;
        let owner = crate::i18n::HOST_OWNER;
        match self {
            GitAccountError::Network { detail } => LocalizedText::keyed(
                owner,
                "error.network_request_failed",
                "Network request failed: {detail}",
            )
            .with_arg("detail", detail.clone()),
            GitAccountError::ParseResponse { detail } => LocalizedText::keyed(
                owner,
                "error.parse_response_failed",
                "Failed to parse the response: {detail}",
            )
            .with_arg("detail", detail.clone()),
            GitAccountError::TokenInvalid => LocalizedText::keyed(
                owner,
                "error.token_invalid",
                "The token is invalid or has expired",
            ),
            GitAccountError::MissingLoginField => LocalizedText::keyed(
                owner,
                "error.response_missing_login",
                "The response is missing the login field",
            ),
            GitAccountError::NotArray => LocalizedText::keyed(
                owner,
                "error.response_not_array",
                "The response is not an array",
            ),
            GitAccountError::RepoListTokenExpired => LocalizedText::keyed(
                owner,
                "error.repo_list_token_expired",
                "Failed to load the repository list: the token may have expired; reconnect the account",
            ),
            GitAccountError::CredentialRead { detail } => LocalizedText::keyed(
                owner,
                "error.keychain_read_failed",
                "Failed to read from the system keychain: {detail}",
            )
            .with_arg("detail", detail.clone()),
            GitAccountError::CredentialWrite { detail } => LocalizedText::keyed(
                owner,
                "error.keychain_write_failed",
                "Failed to write to the system keychain: {detail}",
            )
            .with_arg("detail", detail.clone()),
            GitAccountError::CredentialDelete { detail } => LocalizedText::keyed(
                owner,
                "error.keychain_delete_failed",
                "Failed to delete from the system keychain: {detail}",
            )
            .with_arg("detail", detail.clone()),
            GitAccountError::MetadataRead { detail }
            | GitAccountError::MetadataWrite { detail } => LocalizedText::keyed(
                owner,
                "error.git_account_local_save_failed",
                "Could not save the local Git account record: {detail}",
            )
            .with_arg("detail", detail.clone()),
            GitAccountError::StaleRequest => LocalizedText::keyed(
                owner,
                "error.git_account_stale_request",
                "The account request was superseded; try again",
            ),
            GitAccountError::Unsupported { detail } => LocalizedText::keyed(
                owner,
                "error.git_account_unsupported",
                "This account operation is not supported: {detail}",
            )
            .with_arg("detail", detail.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn metadata_round_trips_the_legacy_shape() {
        let dir = dir();
        let path = dir.path().join(METADATA_FILE);
        let mut state = GitAccountsState::default();
        state.set(
            GitProvider::GitHub,
            Some(ConnectedAccount {
                username: "octocat".into(),
            }),
        );
        save_to(&path, &state).unwrap();
        assert_eq!(load_from(&path), state);
        let json = std::fs::read_to_string(&path).unwrap();
        assert!(json.contains("\"github\""));
        assert!(json.contains("\"octocat\""));
        assert!(!json.contains("token"));
    }

    #[test]
    fn missing_and_corrupt_files_load_as_empty() {
        let dir = dir();
        assert_eq!(
            load_from(&dir.path().join("missing.json")),
            GitAccountsState::default()
        );
        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, "not json").unwrap();
        assert_eq!(load_from(&bad), GitAccountsState::default());
    }

    #[test]
    fn provider_dtos_come_from_the_shared_crate() {
        assert_eq!(GitProvider::ALL.len(), 3);
        assert_eq!(GitProvider::GitHub.as_key(), "github");
        assert_eq!(
            GitProvider::GitHub.whoami_url(),
            "https://api.github.com/user"
        );
        assert!(
            GitProvider::Gitee
                .token_creation_url()
                .starts_with("https://")
        );
    }

    #[test]
    fn username_and_repo_parsing_are_shared() {
        assert_eq!(
            extract_username(GitProvider::GitHub, r#"{"login":"octocat"}"#).unwrap(),
            "octocat"
        );
        let repos = parse_repo_list(
            GitProvider::GitLab,
            r#"[{"path_with_namespace":"a/b","http_url_to_repo":"u"}]"#,
        )
        .unwrap();
        assert_eq!(repos[0].full_name, "a/b");
    }

    #[test]
    fn localized_errors_never_embed_a_token() {
        let error = GitAccountError::TokenInvalid;
        let text = error.localized();
        assert!(!format!("{text:?}").contains("ghp_"));
    }
}
