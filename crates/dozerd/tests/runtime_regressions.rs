//! 运行时审核反例：真实 UDS 的控制准入、任务幂等和迟到 Attach。

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use dozer_core::protocol::{AgentKind, Reply, Request, decode_line, encode_line};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

// server 的准入是进程级共享状态；锁覆盖各测试 runtime 的创建与销毁。
static SERIAL: Mutex<()> = Mutex::new(());

struct Fixture {
    socket: PathBuf,
    server: tokio::task::JoinHandle<anyhow::Result<()>>,
    registry: Arc<dozerd::registry::SessionRegistry>,
    projects: Arc<dozerd::projects::ProjectStore>,
    todos: Arc<dozerd::todo::TodoStore>,
    executions: Arc<dozerd::task_execution::TaskExecutionStore>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    async fn start() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("dz-runtime-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = dir.path().join("daemon.sock");
        let db = dir.path().join("test.db");
        let registry = Arc::new(dozerd::registry::SessionRegistry::new());
        let projects = Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap());
        let todos = Arc::new(dozerd::todo::TodoStore::new(&db).unwrap());
        let executions = Arc::new(dozerd::task_execution::TaskExecutionStore::open(&db).unwrap());
        let stores = dozerd::server::Stores {
            registry: registry.clone(),
            projects: projects.clone(),
            todos: todos.clone(),
            executions: executions.clone(),
            bookmarks: Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap()),
            code_health: Arc::new(dozerd::code_health::CodeHealthStore::new(&db).unwrap()),
            transcripts: Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap()),
            session_summaries: Arc::new(
                dozerd::session_summary::SessionSummaryStore::open(&db).unwrap(),
            ),
            summary_jobs: Arc::new(dozerd::summary_jobs::SummaryJobStore::open(&db).unwrap()),
            backfill_registry: Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new()),
            categories: Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap()),
            memories: Arc::new(dozerd::memory::MemoryStore::new(&db).unwrap()),
            file_edit_history: Arc::new(
                dozerd::file_edit_history::FileEditHistoryStore::new(&db).unwrap(),
            ),
            groups: dozerd::group_service::GroupService::for_tests(),
            apps: bytehost_apps::service::AppService::unavailable("runtime regression"),
            locale: Arc::new(
                dozerd::locale_settings::LocaleSettingsStore::open(&dir.path().join("locale.json"))
                    .unwrap(),
            ),
        };
        let server_socket = socket.clone();
        let ide_dir = dir.path().join("ide");
        let server = tokio::spawn(async move {
            dozerd::server::serve(
                &server_socket,
                ide_dir,
                stores,
                dozerd::task_poller::new_in_flight(),
                false,
            )
            .await
        });
        for _ in 0..200 {
            if socket.exists() {
                break;
            }
            assert!(!server.is_finished(), "server 在绑定前退出");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(socket.exists());
        Self {
            socket,
            server,
            registry,
            projects,
            todos,
            executions,
            _dir: dir,
        }
    }

    async fn connect(&self) -> BufReader<UnixStream> {
        BufReader::new(UnixStream::connect(&self.socket).await.unwrap())
    }

    async fn request(&self, request: Request) -> Reply {
        exchange(&mut self.connect().await, request).await
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn receive(stream: &mut BufReader<UnixStream>) -> Reply {
    let mut line = String::new();
    let n = tokio::time::timeout(Duration::from_secs(3), stream.read_line(&mut line))
        .await
        .expect("reply deadline")
        .unwrap();
    assert!(n > 0, "server 意外关闭连接");
    decode_line(&line).unwrap()
}

async fn exchange(stream: &mut BufReader<UnixStream>, request: Request) -> Reply {
    stream
        .get_mut()
        .write_all(encode_line(&request).as_bytes())
        .await
        .unwrap();
    receive(stream).await
}

#[test]
fn overloaded_ordinary_request_closes_only_its_connection() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut fixture = Fixture::start().await;
            let hello = || Request::Hello { protocol: 1 };
            let mut ordinary = Vec::new();
            for _ in 0..120 {
                let mut stream = fixture.connect().await;
                assert!(matches!(
                    exchange(&mut stream, hello()).await,
                    Reply::Hello { .. }
                ));
                ordinary.push(stream);
            }
            let mut rejected = fixture.connect().await;
            assert!(matches!(
                exchange(&mut rejected, hello()).await,
                Reply::Error { .. }
            ));
            let mut eof = String::new();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), rejected.read_line(&mut eof))
                    .await
                    .unwrap()
                    .unwrap(),
                0,
            );
            let mut malformed = fixture.connect().await;
            malformed.get_mut().write_all(b"not-json\n").await.unwrap();
            assert!(matches!(receive(&mut malformed).await, Reply::Error { .. }));
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), malformed.read_line(&mut eof))
                    .await
                    .unwrap()
                    .unwrap(),
                0,
                "错误首帧也必须释放控制连接",
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(100), &mut fixture.server)
                    .await
                    .is_err(),
                "普通 Hello 的 Busy 回复不能停止 daemon"
            );
            assert!(matches!(
                exchange(&mut ordinary[0], hello()).await,
                Reply::Hello { .. }
            ));
            assert!(matches!(
                fixture.request(Request::Shutdown).await,
                Reply::Ok
            ));
            tokio::time::timeout(Duration::from_secs(3), &mut fixture.server)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        });
}

#[test]
fn only_matching_idempotency_key_and_input_return_existing_attempt() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let fixture = Fixture::start().await;
            let project = fixture
                .projects
                .open(fixture._dir.path().to_str().unwrap())
                .unwrap();
            let todo = fixture.todos.add(project.id, "audit task").unwrap();
            let todo = fixture
                .todos
                .assign_agent(todo.id, AgentKind::Claude)
                .unwrap();
            // 保持真实 Running 记录，不调用任何外部 agent。
            let first = dozerd::task_processor::start_task(
                &fixture.executions,
                &fixture.todos,
                &fixture.projects,
                &todo,
                Some("first input"),
                Some("key-a"),
            )
            .unwrap();
            let first_id = match &first {
                dozerd::task_execution::Admission::Started(attempt) => attempt.attempt_id(),
                _ => panic!("fixture 应创建一次执行"),
            };
            let reply = fixture
                .request(Request::StartTodoExecution {
                    id: todo.id,
                    human_reply: Some("first input".into()),
                    idempotency_key: Some("key-a".into()),
                })
                .await;
            match reply {
                Reply::TodoExecution {
                    execution: Some(execution),
                } => {
                    assert_eq!(execution.attempt_id, first_id);
                }
                other => panic!("同 key/同输入应返回原执行: {other:?}"),
            }
            for (key, input) in [
                (Some("key-a"), "changed input"),
                (Some("key-b"), "second input"),
                (None, "third input"),
            ] {
                let reply = fixture
                    .request(Request::StartTodoExecution {
                        id: todo.id,
                        human_reply: Some(input.into()),
                        idempotency_key: key.map(str::to_string),
                    })
                    .await;
                assert!(
                    matches!(reply, Reply::Error { .. }),
                    "冲突请求不能返回其他执行: {reply:?}"
                );
                assert_eq!(
                    fixture
                        .executions
                        .latest(todo.id)
                        .unwrap()
                        .unwrap()
                        .attempt_id,
                    first_id
                );
            }
        });
}

#[test]
fn late_attach_delivers_all_output_and_exits_exactly_once() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let fixture = Fixture::start().await;
            let session = fixture
                .registry
                .create(dozerd::session::SessionSpec {
                    name: "late attach".into(),
                    command: "/bin/sh".into(),
                    args: vec!["-c".into(), "printf FINAL".into()],
                    cwd: fixture._dir.path().to_string_lossy().into_owned(),
                    cols: 80,
                    rows: 24,
                    project_id: 1,
                })
                .unwrap();
            tokio::time::timeout(Duration::from_secs(2), async {
                while session.exit_status().is_none() {
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
            })
            .await
            .unwrap();
            let mut stream = fixture.connect().await;
            let reply = exchange(
                &mut stream,
                Request::Attach {
                    session_id: session.id().into(),
                    from_offset: 0,
                },
            )
            .await;
            let mut bytes = match reply {
                Reply::Attached { snapshot_b64, .. } => B64.decode(snapshot_b64).unwrap(),
                other => panic!("attach failed: {other:?}"),
            };
            loop {
                match receive(&mut stream).await {
                    Reply::Output { data_b64, .. } => bytes.extend(B64.decode(data_b64).unwrap()),
                    Reply::Exited { code, .. } => {
                        assert_eq!(code, Some(0));
                        break;
                    }
                    other => panic!("unexpected event: {other:?}"),
                }
            }
            assert_eq!(bytes, b"FINAL");
            let mut line = String::new();
            assert!(
                tokio::time::timeout(Duration::from_millis(100), stream.read_line(&mut line))
                    .await
                    .is_err(),
                "Exited 后不能再发正文或重复 Exited"
            );
        });
}

#[test]
fn v2_late_attach_reports_snapshot_and_exit_atomically() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let fixture = Fixture::start().await;
            let session = fixture
                .registry
                .create(dozerd::session::SessionSpec {
                    name: "short".into(),
                    command: "/bin/sh".into(),
                    args: vec!["-c".into(), "printf final; exit 7".into()],
                    cwd: "/tmp".into(),
                    cols: 80,
                    rows: 24,
                    project_id: 1,
                })
                .unwrap();
            assert_eq!(session.wait_for_exit().await, Some(7));
            // Legacy late-attach observes the same sealed output state; wait for its final Exited.
            let mut legacy = fixture.connect().await;
            assert!(matches!(
                exchange(
                    &mut legacy,
                    Request::Attach {
                        session_id: session.id().into(),
                        from_offset: 0
                    }
                )
                .await,
                Reply::Attached { .. }
            ));
            loop {
                if matches!(receive(&mut legacy).await, Reply::Exited { .. }) {
                    break;
                }
            }
            let reply = fixture
                .request(Request::AttachV2 {
                    session_id: session.id().into(),
                    from_offset: 0,
                    epoch: 42,
                })
                .await;
            match reply {
                Reply::Terminal {
                    event:
                        dozer_core::protocol::TerminalEvent::Snapshot {
                            epoch, data, state, ..
                        },
                } => {
                    assert_eq!(epoch, 42);
                    assert_eq!(data, b"final");
                    assert!(state.output_complete);
                    assert_eq!(state.exit_code, Some(7));
                }
                other => panic!("{other:?}"),
            }
            let client = dozer_client::Client::new(fixture.socket.clone());
            let mut stream = client.attach_stream(session.id().into(), 0).await.unwrap();
            let item = stream.events.recv().await.unwrap();
            assert_eq!(item.event.data_len(), 5);
            item.acknowledge();
            drop(item);
            assert!(stream.events.recv().await.is_none());
        });
}

#[test]
fn v2_resumes_inside_retained_window_and_resets_outside_it() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let fixture = Fixture::start().await;
            let session = fixture
                .registry
                .create(dozerd::session::SessionSpec {
                    name: "ring".into(),
                    command: "/bin/sh".into(),
                    args: vec![
                        "-c".into(),
                        "dd if=/dev/zero bs=65536 count=20 2>/dev/null".into(),
                    ],
                    cwd: "/tmp".into(),
                    cols: 80,
                    rows: 24,
                    project_id: 1,
                })
                .unwrap();
            session.wait_for_exit().await;
            let mut legacy = fixture.connect().await;
            let first = exchange(
                &mut legacy,
                Request::Attach {
                    session_id: session.id().into(),
                    from_offset: 0,
                },
            )
            .await;
            let Reply::Attached { .. } = first else {
                panic!("attach expected")
            };
            loop {
                if matches!(receive(&mut legacy).await, Reply::Exited { .. }) {
                    break;
                }
            }
            let (_, next_offset) = session.snapshot();
            assert!(next_offset > dozerd::ring::SCROLLBACK_CAP as u64);
            for (from, mode) in [
                (1, dozer_core::protocol::StreamMode::Reset),
                (next_offset - 16, dozer_core::protocol::StreamMode::Resume),
            ] {
                let reply = fixture
                    .request(Request::AttachV2 {
                        session_id: session.id().into(),
                        from_offset: from,
                        epoch: 123,
                    })
                    .await;
                let Reply::Terminal {
                    event:
                        dozer_core::protocol::TerminalEvent::Snapshot {
                            mode: actual,
                            start_offset,
                            next_offset: end,
                            data,
                            ..
                        },
                } = reply
                else {
                    panic!("snapshot expected")
                };
                assert_eq!(actual, mode);
                assert_eq!(end, next_offset);
                assert_eq!(end - start_offset, data.len() as u64);
                if mode == dozer_core::protocol::StreamMode::Resume {
                    assert_eq!(data.len(), 16);
                } else {
                    assert_eq!(data.len(), dozerd::ring::SCROLLBACK_CAP);
                }
            }
        });
}

#[test]
fn shutdown_continues_when_the_reply_peer_disconnects() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut fixture = Fixture::start().await;
            let session = fixture
                .registry
                .create(dozerd::session::SessionSpec {
                    name: "stop".into(),
                    command: "/bin/sh".into(),
                    args: vec!["-c".into(), "sleep 30".into()],
                    cwd: "/tmp".into(),
                    cols: 80,
                    rows: 24,
                    project_id: 1,
                })
                .unwrap();
            let mut stream = UnixStream::connect(&fixture.socket).await.unwrap();
            stream
                .write_all(encode_line(&Request::Shutdown).as_bytes())
                .await
                .unwrap();
            drop(stream);
            tokio::time::timeout(Duration::from_secs(5), &mut fixture.server)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(fixture.registry.is_draining());
            assert!(!session.info().alive);
            assert!(session.exit_status().is_some());
            assert!(!fixture.socket.exists());
        });
}
