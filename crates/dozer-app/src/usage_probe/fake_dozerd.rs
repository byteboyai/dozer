//! 假 dozerd:在临时目录里监听一个 UDS,讲真实协议(`dozer_core::protocol` 的按行 JSON),
//! 只实现 `GetUsageSummary`。

use dozer_core::protocol::{
    ConversationSummary, Reply, Request, UsagePayload, decode_line, encode_line,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

pub(crate) type UsageRows = Vec<(ConversationSummary, UsagePayload)>;

#[derive(Clone)]
pub(crate) enum Behavior {
    Rows(UsageRows),
    ErrorReply(String),
    HangUp,
}

pub(crate) struct FakeDozerd {
    socket: PathBuf,
    cwds: Arc<Mutex<Vec<String>>>,
    _dir: tempfile::TempDir,
}

impl FakeDozerd {
    pub(crate) fn start(rt: &tokio::runtime::Runtime, behavior: Behavior) -> Self {
        // 短前缀 + 固定短文件名:macOS 的 UDS 路径上限约 104 字节。
        let dir = tempfile::Builder::new().prefix("dzp").tempdir().unwrap();
        let socket = dir.path().join("d.sock");
        let listener = {
            // `UnixListener::bind` 要在 runtime 上下文里注册 reactor。
            let _guard = rt.enter();
            UnixListener::bind(&socket).unwrap()
        };
        let cwds = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&cwds);
        rt.spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let (r, mut w) = stream.into_split();
                let mut lines = BufReader::new(r).lines();
                let Ok(Some(line)) = lines.next_line().await else {
                    continue;
                };
                let Ok(Request::GetUsageSummary { cwd, .. }) = decode_line::<Request>(&line) else {
                    continue;
                };
                seen.lock().unwrap().push(cwd);
                let reply = match &behavior {
                    Behavior::Rows(rows) => Reply::UsageSummary { rows: rows.clone() },
                    Behavior::ErrorReply(message) => Reply::Error {
                        message: message.clone(),
                    },
                    // 不写应答直接丢弃本连接 = 对端关闭,客户端得到 "daemon 断开"。
                    Behavior::HangUp => continue,
                };
                let _ = w.write_all(encode_line(&reply).as_bytes()).await;
            }
        });
        Self {
            socket,
            cwds,
            _dir: dir,
        }
    }
    pub(crate) fn socket(&self) -> &Path {
        &self.socket
    }
    pub(crate) fn cwds(&self) -> Vec<String> {
        self.cwds.lock().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel_host::testing::{TIMEOUT, runtime};
    use crate::usage_probe::{NOW_MS, payload, summary};
    use dozer_core::protocol::AgentKind;

    fn client_for(d: &FakeDozerd) -> dozer_client::Client {
        dozer_client::Client::new(d.socket().to_path_buf())
    }

    #[test]
    fn serves_preset_rows_through_the_real_client_and_records_the_cwd() {
        let rt = runtime();
        let rows = vec![(summary("c1", AgentKind::Claude, NOW_MS), payload(4))];
        let d = FakeDozerd::start(&rt, Behavior::Rows(rows));
        let got = rt
            .block_on(async {
                tokio::time::timeout(TIMEOUT, client_for(&d).get_usage_summary("/proj", None)).await
            })
            .unwrap()
            .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0.conversation_id, "c1");
        assert_eq!(got[0].1.turns, 4);
        assert_eq!(d.cwds(), vec!["/proj".to_string()]);
    }

    #[test]
    fn hang_up_and_error_reply_surface_as_client_errors() {
        let rt = runtime();
        for behavior in [Behavior::HangUp, Behavior::ErrorReply("boom".into())] {
            let d = FakeDozerd::start(&rt, behavior);
            let res = rt
                .block_on(async {
                    tokio::time::timeout(TIMEOUT, client_for(&d).get_usage_summary("/p", None))
                        .await
                })
                .unwrap();
            assert!(res.is_err(), "假服务不应答/报错时客户端必须拿到 Err");
        }
    }

    #[test]
    fn two_servers_do_not_share_a_socket() {
        let rt = runtime();
        let a = FakeDozerd::start(&rt, Behavior::Rows(vec![]));
        let b = FakeDozerd::start(&rt, Behavior::Rows(vec![]));
        assert_ne!(a.socket(), b.socket());
        assert!(
            a.socket().as_os_str().len() < 100,
            "UDS 路径过长会 bind 失败"
        );
    }
}
