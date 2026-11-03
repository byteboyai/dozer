//! Service trait implementations; all RPCs use Client shared transport.
use super::*;
impl SessionService for Client {
    fn attach_stream(
        &self,
        id: String,
        offset: u64,
    ) -> BoxFuture<'_, Result<dozer_services::SessionStream, ServiceError>> {
        Box::pin(async move {
            Client::attach_stream(self, id, offset)
                .await
                .map_err(into_service_error)
        })
    }
    fn list_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionInfo>, ServiceError>> {
        Box::pin(async move { self.list().await.map_err(into_service_error) })
    }

    fn create(&self, spec: SessionSpec) -> BoxFuture<'_, Result<SessionInfo, ServiceError>> {
        let SessionSpec {
            name,
            command,
            args,
            cwd,
            cols,
            rows,
            project_id,
            agent,
        } = spec;
        Box::pin(async move {
            self.create_for_agent(&name, &command, &args, &cwd, cols, rows, project_id, agent)
                .await
                .map_err(into_service_error)
        })
    }

    fn attach(
        &self,
        id: String,
        from_offset: u64,
    ) -> BoxFuture<'_, Result<(Vec<u8>, u64, mpsc::UnboundedReceiver<TermEvent>), ServiceError>>
    {
        Box::pin(async move {
            self.attach(&id, from_offset)
                .await
                .map_err(into_service_error)
        })
    }

    fn write(&self, id: String, data: Vec<u8>) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move { self.write(&id, &data).await.map_err(into_service_error) })
    }

    fn resize(&self, id: String, cols: u16, rows: u16) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            self.resize(&id, cols, rows)
                .await
                .map_err(into_service_error)
        })
    }

    fn kill(&self, id: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move { self.kill(&id).await.map_err(into_service_error) })
    }

    fn close_with_summary(&self, id: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            self.close_with_summary(&id)
                .await
                .map_err(into_service_error)
        })
    }
}

/// 经 UDS 连到 dozerd 的 [`AppHostApi`] 实现。失败映射:dozerd 回的
/// `AppReply::Failed` 是 `Host`(带类别,含"宿主不可用"),其余连接/协议问题
/// (`Reply::Error`、意外变体、连不上)是 `Transport`。
impl AppHostApi for Client {
    async fn request(&self, request: AppRequest) -> std::result::Result<AppReply, AppApiError> {
        match self
            .roundtrip(&Request::App { request })
            .await
            .map_err(|e| AppApiError::Transport(format!("{e:#}")))?
        {
            Reply::App {
                reply: AppReply::Failed { failure },
            } => Err(AppApiError::Host(failure)),
            Reply::App { reply } => Ok(reply),
            other => Err(AppApiError::Transport(format!("意外应答: {other:?}"))),
        }
    }

    async fn subscribe(
        &self,
    ) -> std::result::Result<mpsc::UnboundedReceiver<AppChange>, AppApiError> {
        self.ensure_version_checked()
            .await
            .map_err(|e| AppApiError::Transport(format!("{e:#}")))?;
        let stream = tokio::time::timeout(CONNECT_TIMEOUT, UnixStream::connect(&self.socket))
            .await
            .map_err(|_| AppApiError::Transport("连接 daemon 超时".into()))?
            .map_err(|e| AppApiError::Transport(format!("{e}")))?;
        let (r, mut w) = stream.into_split();
        // `Subscribe` 不是一问一答:dozerd 在同一连接上先回 `Subscribed`,之后持续推 `Changed`/`Resync`。
        let request = encode_request_checked(&Request::App {
            request: AppRequest::Subscribe,
        })
        .map_err(|e| AppApiError::Transport(format!("{e}")))?;
        if let Err(e) = tokio::time::timeout(WRITE_TIMEOUT, w.write_all(request.as_bytes())).await {
            return Err(AppApiError::Transport(format!("写入超时: {e}")));
        }
        let mut reader = FrameReader::new(BufReader::new(r));
        let first = match tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            reader.next_frame(MAX_SERVER_FRAME_BYTES),
        )
        .await
        {
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) => return Err(AppApiError::Transport("daemon 断开".into())),
            Ok(Err(e)) => return Err(AppApiError::Transport(format!("{e}"))),
            Err(_) => return Err(AppApiError::Transport("订阅握手超时".into())),
        };
        match decode_line::<Reply>(&first) {
            Ok(Reply::App {
                reply: AppReply::Subscribed,
            }) => {}
            Ok(Reply::App {
                reply: AppReply::Failed { failure },
            }) => return Err(AppApiError::Host(failure)),
            Ok(Reply::Error { message }) => {
                return Err(AppApiError::Transport(format!("订阅失败: {message}")));
            }
            Ok(other) => return Err(AppApiError::Transport(format!("意外应答: {other:?}"))),
            Err(e) => return Err(AppApiError::Transport(format!("{e}"))),
        }
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let _keep_writer = w;
            loop {
                tokio::select! {
                    // 接收端被丢弃(例如面板不再需要订阅):停读循环,连接两端随后关闭,
                    // daemon 侧 handle_conn 才能在下一次 next_line() 收到 EOF 并退出。
                    _ = tx.closed() => break,
                    line = reader.next_frame(MAX_SERVER_FRAME_BYTES) => {
                        match line {
                            Ok(Some(line)) => {
                                let change = match decode_line::<Reply>(&line) {
                                    Ok(Reply::App { reply: AppReply::Changed { app } }) => {
                                        AppChange::Changed(app)
                                    }
                                    Ok(Reply::App { reply: AppReply::Resync }) => AppChange::Resync,
                                    Ok(Reply::Error { message }) if message.contains("lagged") => {
                                        AppChange::Resync
                                    }
                                    // 其余(含一次行的 `Subscribed` 回显)忽略。
                                    _ => continue,
                                };
                                if tx.send(change).is_err() {
                                    break;
                                }
                            }
                            _ => {
                                let _ = tx.send(AppChange::Disconnected);
                                break;
                            }
                        }
                    }
                }
            }
        });
        Ok(rx)
    }
}

impl UsageService for Client {
    fn usage_summary(
        &self,
        cwd: String,
        since_ts: Option<u64>,
    ) -> BoxFuture<'_, Result<UsageRows, ServiceError>> {
        Box::pin(async move {
            self.get_usage_summary(&cwd, since_ts)
                .await
                .map_err(into_service_error)
        })
    }
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    #[tokio::test]
    async fn read_frame_rejects_oversized_input_without_newline() {
        let bytes = vec![b'x'; 64];
        let mut reader = FrameReader::new(BufReader::new(&bytes[..]));
        let err = reader.next_frame(16).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn read_frame_returns_lines_and_eof() {
        let bytes = b"a\n\nlast\n";
        let mut reader = FrameReader::new(BufReader::new(&bytes[..]));
        assert_eq!(reader.next_frame(16).await.unwrap().as_deref(), Some("a"));
        assert_eq!(reader.next_frame(16).await.unwrap().as_deref(), Some(""));
        assert_eq!(
            reader.next_frame(16).await.unwrap().as_deref(),
            Some("last")
        );
        assert!(reader.next_frame(16).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn frame_reader_preserves_partial_frame_when_cancelled() {
        let (mut tx, rx) = tokio::io::duplex(1024);
        let mut reader = FrameReader::new(BufReader::new(rx));
        tx.write_all(b"{\"type\":").await.unwrap();
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            reader.next_frame(1024),
        )
        .await;
        tx.write_all(b"\"list_sessions\"}\n").await.unwrap();
        let frame = reader.next_frame(1024).await.unwrap().unwrap();
        assert_eq!(frame, "{\"type\":\"list_sessions\"}");
    }
}

#[cfg(test)]
mod shutdown_tests {
    use super::*;

    #[test]
    fn interpret_shutdown_reply_ok_maps_to_success() {
        let outcome: std::result::Result<Result<Reply>, tokio::time::error::Elapsed> =
            Ok(Ok(Reply::Ok));
        assert!(interpret_shutdown_reply(outcome).is_ok());
    }

    #[test]
    fn interpret_shutdown_reply_unexpected_reply_is_error() {
        let outcome: std::result::Result<Result<Reply>, tokio::time::error::Elapsed> =
            Ok(Ok(Reply::Sessions { sessions: vec![] }));
        assert!(interpret_shutdown_reply(outcome).is_err());
    }

    #[test]
    fn interpret_shutdown_reply_inner_error_propagates() {
        let outcome: std::result::Result<Result<Reply>, tokio::time::error::Elapsed> =
            Ok(Err(anyhow!("daemon 错误: dozerd 正在停止中")));
        let err = interpret_shutdown_reply(outcome).unwrap_err();
        assert!(err.to_string().contains("正在停止中"));
    }

    #[tokio::test]
    async fn interpret_shutdown_reply_timeout_is_error() {
        let elapsed = tokio::time::timeout(
            std::time::Duration::from_millis(0),
            std::future::pending::<Result<Reply>>(),
        )
        .await
        .unwrap_err();
        let err = interpret_shutdown_reply(Err(elapsed)).unwrap_err();
        assert!(err.to_string().contains("超时"));
    }
}

#[cfg(test)]
mod app_subscribe_tests {
    use super::*;
    use bytehost_apps::id::AppId;

    fn temp_sock(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "dz-app-sub-{tag}-{}.sock",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// 假服务端:读到 `Subscribe` 后回 `Subscribed`,再推 2 条 `Changed`,然后关连接。
    async fn fake_server(listener: tokio::net::UnixListener) {
        // 握手是第一条连接;订阅流是下一条。先应答握手,再 accept 订阅流。
        let (stream, _) = listener.accept().await.unwrap();
        let (r, mut w) = stream.into_split();
        let mut lines = BufReader::new(r).lines();
        let _ = lines.next_line().await.unwrap();
        let hello = Reply::Hello {
            protocol: dozer_core::protocol::PROTOCOL_VERSION,
            product: dozer_core::paths::product().name.to_string(),
        };
        w.write_all(encode_line(&hello).as_bytes()).await.unwrap();

        let (stream, _) = listener.accept().await.unwrap();
        let (r, mut w) = stream.into_split();
        let mut lines = BufReader::new(r).lines();
        let _ = lines.next_line().await.unwrap();
        w.write_all(
            encode_line(&Reply::App {
                reply: AppReply::Subscribed,
            })
            .as_bytes(),
        )
        .await
        .unwrap();
        for name in ["a", "b"] {
            let reply = Reply::App {
                reply: AppReply::Changed {
                    app: AppId::new(name).unwrap(),
                },
            };
            w.write_all(encode_line(&reply).as_bytes()).await.unwrap();
        }
        drop(w);
    }

    #[tokio::test]
    async fn app_subscribe_yields_changes_then_disconnected() {
        let sock = temp_sock("ok");
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        let server = tokio::spawn(fake_server(listener));

        let mut rx = Client::new(sock.clone()).subscribe().await.unwrap();
        assert_eq!(
            rx.recv().await,
            Some(AppChange::Changed(AppId::new("a").unwrap()))
        );
        assert_eq!(
            rx.recv().await,
            Some(AppChange::Changed(AppId::new("b").unwrap()))
        );
        assert_eq!(rx.recv().await, Some(AppChange::Disconnected));
        assert_eq!(rx.recv().await, None);
        server.await.unwrap();
        let _ = std::fs::remove_file(&sock);
    }

    /// 接收端被丢弃后读任务必须退出(否则连接与 fd 滞留):服务端只发第一条
    /// `Changed` 后就什么都不做,客户端 drop 接收端,随后读任务应因 `tx.closed()` 退出——
    /// 表现为服务端下一行读取得到 EOF(连接被关)。
    #[tokio::test]
    async fn dropping_the_receiver_closes_the_connection() {
        let sock = temp_sock("drop");
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        let server = tokio::spawn(async move {
            // 先应答握手(第一条连接)
            {
                let (stream, _) = listener.accept().await.unwrap();
                let (r, mut w) = stream.into_split();
                let mut lines = BufReader::new(r).lines();
                let _ = lines.next_line().await.unwrap();
                let hello = Reply::Hello {
                    protocol: dozer_core::protocol::PROTOCOL_VERSION,
                    product: dozer_core::paths::product().name.to_string(),
                };
                w.write_all(encode_line(&hello).as_bytes()).await.unwrap();
            }
            // 再走订阅流
            let (stream, _) = listener.accept().await.unwrap();
            let (r, mut w) = stream.into_split();
            let mut lines = BufReader::new(r).lines();
            let _ = lines.next_line().await.unwrap();
            w.write_all(
                encode_line(&Reply::App {
                    reply: AppReply::Subscribed,
                })
                .as_bytes(),
            )
            .await
            .unwrap();
            // 客户端此后会 drop 接收端;连接应最终 EOF。
            let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
                .await
                .expect("读任务应在丢弃接收端后退出")
                .unwrap();
            assert!(line.is_none(), "连接应被关闭(EOF),得到 {line:?}");
        });

        let rx = Client::new(sock.clone()).subscribe().await.unwrap();
        drop(rx);
        server.await.unwrap();
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn app_subscribe_propagates_a_failed_reply() {
        let sock = temp_sock("fail");
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        tokio::spawn(async move {
            // 先应答握手(第一条连接)
            {
                let (stream, _) = listener.accept().await.unwrap();
                let (r, mut w) = stream.into_split();
                let mut lines = BufReader::new(r).lines();
                let _ = lines.next_line().await.unwrap();
                let hello = Reply::Hello {
                    protocol: dozer_core::protocol::PROTOCOL_VERSION,
                    product: dozer_core::paths::product().name.to_string(),
                };
                w.write_all(encode_line(&hello).as_bytes()).await.unwrap();
            }
            // 再走订阅流
            let (stream, _) = listener.accept().await.unwrap();
            let (r, mut w) = stream.into_split();
            let mut lines = BufReader::new(r).lines();
            let _ = lines.next_line().await.unwrap();
            let failure = bytehost_apps::proto::AppFailure::new(
                bytehost_apps::proto::AppErrorKind::Unavailable,
                "宿主不可用",
            );
            w.write_all(
                encode_line(&Reply::App {
                    reply: AppReply::Failed { failure },
                })
                .as_bytes(),
            )
            .await
            .unwrap();
        });

        let err = Client::new(sock.clone()).subscribe().await.unwrap_err();
        assert_eq!(
            err.kind(),
            Some(bytehost_apps::proto::AppErrorKind::Unavailable)
        );
        let _ = std::fs::remove_file(&sock);
    }
}

impl TodoService for Client {
    fn list_todos(&self, project_id: i64) -> BoxFuture<'_, Result<Vec<TodoInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_todos(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn start_todo_execution(
        &self,
        id: i64,
        human_reply: Option<String>,
        idempotency_key: Option<String>,
    ) -> BoxFuture<'_, Result<TaskExecutionInfo, ServiceError>> {
        Box::pin(async move {
            Client::start_todo_execution(
                self,
                id,
                human_reply.as_deref(),
                idempotency_key.as_deref(),
            )
            .await
            .map_err(into_service_error)
        })
    }
    fn get_todo_execution(
        &self,
        id: i64,
    ) -> BoxFuture<'_, Result<Option<TaskExecutionInfo>, ServiceError>> {
        Box::pin(async move {
            Client::get_todo_execution(self, id)
                .await
                .map_err(into_service_error)
        })
    }
    fn list_categories(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<CategoryInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_categories(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn add_todo(&self, project_id: i64, text: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::add_todo(self, project_id, &text)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn toggle_todo(&self, id: i64, done: bool) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::toggle_todo(self, id, done)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn edit_todo_text(&self, id: i64, text: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::edit_todo_text(self, id, &text)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn reorder_todo(
        &self,
        id: i64,
        after_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::reorder_todo(self, id, after_id)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn set_todo_plan_date(
        &self,
        id: i64,
        plan_date: Option<String>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::set_todo_plan_date(self, id, plan_date.as_deref())
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn set_todo_status(
        &self,
        id: i64,
        status: dozer_core::protocol::TodoStoredStatus,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::set_todo_status(self, id, status)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn set_todo_category(
        &self,
        id: i64,
        category_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::set_todo_category(self, id, category_id)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn add_category(
        &self,
        project_id: i64,
        parent_id: Option<i64>,
        name: String,
    ) -> BoxFuture<'_, Result<CategoryInfo, ServiceError>> {
        Box::pin(async move {
            Client::add_category(self, project_id, parent_id, &name)
                .await
                .map_err(into_service_error)
        })
    }
    fn rename_category(&self, id: i64, name: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::rename_category(self, id, &name)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn delete_category(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::delete_category(self, id)
                .await
                .map_err(into_service_error)
        })
    }
    fn reparent_category(
        &self,
        id: i64,
        new_parent_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::reparent_category(self, id, new_parent_id)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn move_category_sibling(
        &self,
        id: i64,
        direction: CategoryMoveDirection,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::move_category_sibling(self, id, direction)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
}

impl MemoryService for Client {
    fn list_memories(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::MemoryInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_memories(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn get_memory(
        &self,
        project_id: i64,
        id: i64,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::MemoryDetail, ServiceError>> {
        Box::pin(async move {
            Client::get_memory(self, project_id, id)
                .await
                .map_err(into_service_error)
        })
    }
    fn write_memory(
        &self,
        project_id: i64,
        title: String,
        kind: String,
        description: String,
        body: String,
        actor: String,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::MemoryDetail, ServiceError>> {
        Box::pin(async move {
            Client::write_memory(self, project_id, &title, &kind, &description, &body, &actor)
                .await
                .map_err(into_service_error)
        })
    }
    fn delete_memory(
        &self,
        project_id: i64,
        id: i64,
        actor: String,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::delete_memory(self, project_id, id, &actor)
                .await
                .map_err(into_service_error)
        })
    }
}

impl ProjectService for Client {
    fn rename_project(
        &self,
        id: i64,
        name: String,
    ) -> BoxFuture<'_, Result<Option<dozer_core::protocol::ProjectInfo>, ServiceError>> {
        Box::pin(async move {
            Client::rename_project(self, id, &name)
                .await
                .map_err(into_service_error)
        })
    }
    fn remove_project(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::remove_project(self, id)
                .await
                .map_err(into_service_error)
        })
    }
    fn open_project(
        &self,
        path: String,
    ) -> BoxFuture<'_, Result<Option<dozer_core::protocol::ProjectInfo>, ServiceError>> {
        Box::pin(async move {
            Client::open_project(self, &path)
                .await
                .map_err(into_service_error)
        })
    }
    fn list_projects(
        &self,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::ProjectInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_projects(self)
                .await
                .map_err(into_service_error)
        })
    }
}

impl TranscriptService for Client {
    fn delete_project_transcripts(&self, cwd: String) -> BoxFuture<'_, Result<u32, ServiceError>> {
        Box::pin(async move {
            Client::delete_project_transcripts(self, &cwd)
                .await
                .map_err(into_service_error)
        })
    }
    fn backfill_project_transcripts(
        &self,
        cwd: String,
    ) -> BoxFuture<'_, Result<u32, ServiceError>> {
        Box::pin(async move {
            Client::backfill_project_transcripts(self, &cwd)
                .await
                .map_err(into_service_error)
        })
    }
    fn backfill_project_transcripts_report(
        &self,
        cwd: String,
    ) -> BoxFuture<'_, Result<dozer_model::BackfillReport, ServiceError>> {
        Box::pin(async move {
            Client::backfill_project_transcripts_report(self, &cwd)
                .await
                .map_err(into_service_error)
        })
    }
    fn submit_summary_batch(
        &self,
        cwd: String,
        provider: Option<dozer_core::protocol::AgentKind>,
        model: Option<String>,
    ) -> BoxFuture<'_, Result<(i64, u32), ServiceError>> {
        Box::pin(async move {
            Client::submit_summary_batch(self, &cwd, provider, model.as_deref())
                .await
                .map_err(into_service_error)
        })
    }
    fn get_summary_batch(
        &self,
        batch_id: i64,
    ) -> BoxFuture<'_, Result<Option<dozer_core::protocol::SummaryBatchInfo>, ServiceError>> {
        Box::pin(async move {
            Client::get_summary_batch(self, batch_id)
                .await
                .map_err(into_service_error)
        })
    }
}

impl GroupChatService for Client {
    fn list_groups(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::GroupInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_groups(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn list_group_messages(
        &self,
        group_id: i64,
        after_rev: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<(Vec<dozer_core::protocol::GroupMessageInfo>, i64), ServiceError>>
    {
        Box::pin(async move {
            Client::list_group_messages(self, group_id, after_rev, limit)
                .await
                .map_err(into_service_error)
        })
    }
    fn create_group(
        &self,
        project_id: i64,
        topic: String,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::GroupInfo, ServiceError>> {
        Box::pin(async move {
            Client::create_group(self, project_id, &topic)
                .await
                .map_err(into_service_error)
        })
    }
    fn delete_group(&self, group_id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::delete_group(self, group_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn add_group_member(
        &self,
        group_id: i64,
        agent: dozer_core::protocol::AgentKind,
        handle: String,
        role_prompt: String,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::GroupInfo, ServiceError>> {
        Box::pin(async move {
            Client::add_group_member(self, group_id, agent, &handle, &role_prompt)
                .await
                .map_err(into_service_error)
        })
    }
    fn update_group_member(
        &self,
        member_id: i64,
        handle: String,
        role_prompt: String,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::GroupInfo, ServiceError>> {
        Box::pin(async move {
            Client::update_group_member(self, member_id, &handle, &role_prompt)
                .await
                .map_err(into_service_error)
        })
    }
    fn remove_group_member(
        &self,
        member_id: i64,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::GroupInfo, ServiceError>> {
        Box::pin(async move {
            Client::remove_group_member(self, member_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn post_group_message(
        &self,
        group_id: i64,
        text: String,
    ) -> BoxFuture<'_, Result<dozer_services::PostedMessages, ServiceError>> {
        Box::pin(async move {
            Client::post_group_message(self, group_id, &text)
                .await
                .map_err(into_service_error)
        })
    }
    fn cancel_group(
        &self,
        group_id: i64,
        scope: dozer_core::protocol::GroupCancelScope,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::cancel_group(self, group_id, scope)
                .await
                .map_err(into_service_error)
        })
    }
    fn retry_group_message(
        &self,
        message_id: i64,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::GroupMessageInfo, ServiceError>> {
        Box::pin(async move {
            Client::retry_group_message(self, message_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn push_group_message_to_todo(
        &self,
        message_id: i64,
        text: String,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::TodoInfo, ServiceError>> {
        Box::pin(async move {
            Client::push_group_message_to_todo(self, message_id, &text)
                .await
                .map_err(into_service_error)
        })
    }
}

impl ConversationService for Client {
    fn backfill_state(
        &self,
    ) -> BoxFuture<'_, Result<Option<dozer_model::BackfillState>, ServiceError>> {
        Box::pin(async move {
            if !self
                .runtime_capabilities()
                .await
                .map_err(into_service_error)?
                .backfill_state
            {
                return Ok(None);
            }
            match self
                .roundtrip(&Request::GetBackfillState)
                .await
                .map_err(into_service_error)?
            {
                Reply::BackfillState { state } => Ok(Some(state)),
                reply => Err(ServiceError::new(
                    ServiceErrorKind::Protocol,
                    format!("unexpected reply: {reply:?}"),
                )),
            }
        })
    }

    fn get_conversation_turns(
        &self,
        conversation_id: String,
        after_turn_index: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<TurnRecord>, ServiceError>> {
        Box::pin(async move {
            Client::get_conversation_turns(self, &conversation_id, after_turn_index, limit)
                .await
                .map_err(into_service_error)
        })
    }

    fn list_conversations_with_summaries(
        &self,
        cwd: String,
        agent: Option<dozer_core::protocol::AgentKind>,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<ConversationRows, ServiceError>> {
        Box::pin(async move {
            Client::list_conversations_with_summaries(self, &cwd, agent, limit, offset)
                .await
                .map_err(into_service_error)
        })
    }
}

impl SummaryService for Client {
    fn submit_summary_job(
        &self,
        conversation_id: String,
        force: bool,
    ) -> BoxFuture<'_, Result<i64, ServiceError>> {
        Box::pin(async move {
            Client::submit_summary_job(
                self,
                &conversation_id,
                None,
                dozer_model::SummaryTrigger::Manual,
                None,
                None,
                force,
            )
            .await
            .map_err(into_service_error)
        })
    }

    fn get_summary_job(
        &self,
        job_id: i64,
    ) -> BoxFuture<'_, Result<Option<dozer_model::SummaryJobInfo>, ServiceError>> {
        Box::pin(async move {
            Client::get_summary_job(self, job_id)
                .await
                .map_err(into_service_error)
        })
    }
}

impl CodeHealthService for Client {
    fn get_code_health_report(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Option<dozer_core::protocol::CodeHealthReportInfo>, ServiceError>>
    {
        Box::pin(async move {
            Client::get_code_health_report(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn list_code_health_reports(
        &self,
        project_id: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::CodeHealthReportInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_code_health_reports(self, project_id, limit)
                .await
                .map_err(into_service_error)
        })
    }
    fn save_code_health_report(
        &self,
        project_id: i64,
        info: dozer_core::protocol::CodeHealthReportInfo,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::save_code_health_report(self, project_id, &info)
                .await
                .map_err(into_service_error)
        })
    }
}

impl ContextService for Client {
    fn list_context_items(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::ContextItemInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_context_items(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn add_context_item(
        &self,
        project_id: i64,
        entity_kind: String,
        entity_ref: String,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::ContextItemInfo, ServiceError>> {
        Box::pin(async move {
            Client::add_context_item(self, project_id, &entity_kind, &entity_ref)
                .await
                .map_err(into_service_error)
        })
    }
    fn remove_context_item(
        &self,
        project_id: i64,
        id: i64,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::remove_context_item(self, project_id, id)
                .await
                .map_err(into_service_error)
        })
    }
}

impl EditHistoryService for Client {
    fn list_file_edit_history(
        &self,
        project_id: i64,
        path_filter: Option<String>,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::FileEditHistoryInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_file_edit_history(self, project_id, path_filter.as_deref(), limit)
                .await
                .map_err(into_service_error)
        })
    }
    fn apply_precise_edit(
        &self,
        edit: PreciseEdit,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::MutationOutcome, ServiceError>> {
        Box::pin(async move {
            let PreciseEdit {
                project_id,
                path,
                start_line,
                start_col,
                end_line,
                end_col,
                expected_text,
                new_text,
                summary,
                actor,
                session_id,
            } = edit;
            Client::apply_precise_edit(
                self,
                project_id,
                &path,
                start_line,
                start_col,
                end_line,
                end_col,
                &expected_text,
                &new_text,
                &summary,
                &actor,
                &session_id,
            )
            .await
            .map_err(into_service_error)
        })
    }
}

impl BookmarkService for Client {
    fn list_bookmarks(
        &self,
        project_id: Option<i64>,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::BookmarkInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_bookmarks(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn add_bookmark(
        &self,
        scope: dozer_core::protocol::BookmarkScope,
        project_id: Option<i64>,
        url: String,
        title: String,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::add_bookmark(self, scope, project_id, &url, &title)
                .await
                .map_err(into_service_error)
        })
    }
    fn remove_bookmark(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::remove_bookmark(self, id)
                .await
                .map_err(into_service_error)
        })
    }
}

impl LocaleService for Client {
    fn get_locale(&self) -> BoxFuture<'_, Result<LocaleSnapshot, ServiceError>> {
        Box::pin(async move { Client::get_locale(self).await.map_err(into_service_error) })
    }
    fn set_locale(
        &self,
        preference: LocalePreference,
        expected_revision: u64,
        system_candidates: Vec<String>,
    ) -> BoxFuture<'_, Result<LocaleWriteOutcome, ServiceError>> {
        Box::pin(async move {
            Client::set_locale(self, preference, expected_revision, system_candidates)
                .await
                .map_err(into_service_error)
        })
    }
}

#[cfg(test)]
mod service_error_tests {
    use super::*;
    use dozer_services::{ServiceError, ServiceErrorKind};

    #[test]
    fn io_errors_are_unavailable() {
        let e = anyhow::Error::from(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "refused",
        ));
        assert_eq!(into_service_error(e).kind, ServiceErrorKind::Unavailable);
    }

    #[test]
    fn typed_service_errors_pass_through_with_the_same_text() {
        let e = anyhow::Error::new(ServiceError::new(
            ServiceErrorKind::Rejected,
            "daemon 错误: boom",
        ));
        let s = into_service_error(e);
        assert_eq!(
            (s.kind, s.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
    }

    #[test]
    fn anything_else_is_protocol() {
        let s = into_service_error(anyhow::anyhow!("意外应答: Ok"));
        assert_eq!(s.kind, ServiceErrorKind::Protocol);
        assert_eq!(s.message, "意外应答: Ok");
    }
}

#[cfg(test)]
mod todo_service_tests {
    use super::*;
    use dozer_services::{ServiceErrorKind, TodoService};

    pub(super) fn temp_sock(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "dz-todo-svc-{tag}-{}.sock",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// 先应答协议握手(Hello),再读一行业务请求回 `reply`;`None` =
    /// 业务请求不应答直接挂断(握手仍正常放行,这样"挂断"只影响业务
    /// 请求,语义与旧用例一致)。
    pub(super) fn serve_once(tag: &str, reply: Option<Reply>) -> PathBuf {
        let sock = temp_sock(tag);
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        tokio::spawn(async move {
            // 每个业务调用都是一条新连接(握手一条 + 业务一条),假服务端
            // 必须持续 accept;且要按请求类型应答:Hello 请求回同版本握手,
            // 业务请求回配置的 reply(否则业务连接会把握手应答错当业务应答)。
            while let Ok((stream, _)) = listener.accept().await {
                let reply = reply.clone();
                tokio::spawn(async move {
                    let (r, mut w) = stream.into_split();
                    let mut lines = BufReader::new(r).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let is_hello = serde_json::from_str::<serde_json::Value>(&line)
                            .ok()
                            .and_then(|v| {
                                v.get("type").and_then(|t| t.as_str()).map(|t| t == "hello")
                            })
                            .unwrap_or(false);
                        let out = if is_hello {
                            Reply::Hello {
                                protocol: dozer_core::protocol::PROTOCOL_VERSION,
                                product: dozer_core::paths::product().name.to_string(),
                            }
                        } else {
                            match reply.clone() {
                                Some(reply) => reply,
                                // None = 业务请求不应答直接挂断(旧语义)
                                None => return,
                            }
                        };
                        let _ = w.write_all(encode_line(&out).as_bytes()).await;
                    }
                });
            }
        });
        sock
    }

    #[tokio::test]
    async fn list_todos_comes_back_as_ok() {
        let sock = serve_once("list", Some(Reply::Todos { todos: vec![] }));
        let got = TodoService::list_todos(&Client::new(sock.clone()), 7).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn a_write_returns_unit_on_success() {
        let sock = serve_once("write", Some(Reply::Ok));
        let got = TodoService::delete_category(&Client::new(sock.clone()), 3).await;
        assert_eq!(got, Ok(()));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("hangup", None);
        let e = TodoService::list_todos(&Client::new(sock.clone()), 7)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Unavailable, "daemon 断开")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once(
            "reject",
            Some(Reply::Error {
                message: "boom".into(),
            }),
        );
        let e = TodoService::delete_category(&Client::new(sock.clone()), 3)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod locale_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_core::protocol::Locale;
    use dozer_services::{LocaleService, ServiceErrorKind};

    fn snap(rev: u64, loc: &str) -> LocaleSnapshot {
        LocaleSnapshot {
            schema_version: 1,
            preference: LocalePreference::Explicit {
                locale: Locale(loc.into()),
            },
            active_locale: Locale(loc.into()),
            revision: rev,
        }
    }

    #[tokio::test]
    async fn get_locale_comes_back_as_snapshot() {
        let sock = serve_once(
            "locale-get",
            Some(Reply::Locale {
                snapshot: snap(2, "zh-CN"),
            }),
        );
        let got = LocaleService::get_locale(&Client::new(sock.clone())).await;
        assert_eq!(got, Ok(snap(2, "zh-CN")));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn set_locale_returns_applied_outcome() {
        let sock = serve_once(
            "locale-set",
            Some(Reply::LocaleWrite {
                outcome: LocaleWriteOutcome::Applied {
                    snapshot: snap(1, "zh-CN"),
                },
            }),
        );
        let got = LocaleService::set_locale(
            &Client::new(sock.clone()),
            LocalePreference::Explicit {
                locale: Locale("zh-CN".into()),
            },
            0,
            vec![],
        )
        .await;
        assert_eq!(
            got,
            Ok(LocaleWriteOutcome::Applied {
                snapshot: snap(1, "zh-CN")
            })
        );
        let _ = std::fs::remove_file(&sock);
    }

    /// 旧 daemon 不认识 `GetLocale`,回 `Reply::Error` → 客户端分类为 `Rejected`,
    /// 不无限重试、不 panic。
    #[tokio::test]
    async fn old_daemon_error_reply_is_rejected() {
        let sock = serve_once(
            "locale-old",
            Some(Reply::Error {
                message: "协议错误: unknown variant `get_locale`".into(),
            }),
        );
        let e = LocaleService::get_locale(&Client::new(sock.clone()))
            .await
            .unwrap_err();
        assert_eq!(e.kind, ServiceErrorKind::Rejected);
        let _ = std::fs::remove_file(&sock);
    }

    /// 旧 daemon 直接断连 → `Unavailable`。
    #[tokio::test]
    async fn old_daemon_hang_up_is_unavailable() {
        let sock = serve_once("locale-hangup", None);
        let e = LocaleService::get_locale(&Client::new(sock.clone()))
            .await
            .unwrap_err();
        assert_eq!(e.kind, ServiceErrorKind::Unavailable);
        let _ = std::fs::remove_file(&sock);
    }

    /// 意外应答形状 → `Protocol`,不当作空结果吞掉。
    #[tokio::test]
    async fn unexpected_reply_is_protocol() {
        let sock = serve_once("locale-wrong", Some(Reply::Ok));
        let e = LocaleService::get_locale(&Client::new(sock.clone()))
            .await
            .unwrap_err();
        assert_eq!(e.kind, ServiceErrorKind::Protocol);
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod project_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_services::{MemoryService, ProjectService, ServiceErrorKind, TranscriptService};
    #[tokio::test]
    async fn list_memories_comes_back_as_ok() {
        let sock = serve_once("mem-list", Some(Reply::Memories { memories: vec![] }));
        let got = MemoryService::list_memories(&Client::new(sock.clone()), 7).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn a_unit_write_returns_unit_on_success() {
        let sock = serve_once("proj-remove", Some(Reply::Ok));
        let got = ProjectService::remove_project(&Client::new(sock.clone()), 3).await;
        assert_eq!(got, Ok(()));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn a_count_is_carried_back_unchanged() {
        let sock = serve_once(
            "proj-backfill",
            Some(Reply::BackfillDone {
                imported_files: 3,
                failed_total: 0,
                failures: Vec::new(),
            }),
        );
        let got = TranscriptService::backfill_project_transcripts(
            &Client::new(sock.clone()),
            "/p".into(),
        )
        .await;
        assert_eq!(got, Ok(3));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn failure_details_are_carried_back_through_the_service() {
        let failures = vec![dozer_model::BackfillFailure {
            path: "/h/a.jsonl".into(),
            reason: "坏文件".into(),
        }];
        let sock = serve_once(
            "proj-bf-rep",
            Some(Reply::BackfillDone {
                imported_files: 2,
                failed_total: 1,
                failures: failures.clone(),
            }),
        );
        let got = TranscriptService::backfill_project_transcripts_report(
            &Client::new(sock.clone()),
            "/p".into(),
        )
        .await
        .unwrap();
        assert_eq!(got.imported_files, 2);
        assert_eq!(got.failed_total, 1);
        assert_eq!(got.failures, failures);
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("proj-hangup", None);
        let e = MemoryService::list_memories(&Client::new(sock.clone()), 7)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Unavailable, "daemon 断开")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once(
            "proj-reject",
            Some(Reply::Error {
                message: "boom".into(),
            }),
        );
        let e = ProjectService::remove_project(&Client::new(sock.clone()), 3)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
        let _ = std::fs::remove_file(&sock);
    }

    fn project_info(id: i64, path: &str) -> dozer_core::protocol::ProjectInfo {
        dozer_core::protocol::ProjectInfo {
            id,
            path: path.into(),
            name: "demo".into(),
            last_active_ms: 1,
            created_ms: 2,
            updated_ms: 3,
        }
    }

    #[tokio::test]
    async fn open_project_carries_some_and_none_back_unchanged() {
        let info = project_info(5, "/p/demo");
        let sock = serve_once(
            "pc-open-some",
            Some(Reply::Project {
                project: Some(info.clone()),
            }),
        );
        let got = ProjectService::open_project(&Client::new(sock.clone()), "/p/demo".into()).await;
        assert_eq!(got, Ok(Some(info)));
        let _ = std::fs::remove_file(&sock);

        let sock = serve_once("pc-open-none", Some(Reply::Project { project: None }));
        let got = ProjectService::open_project(&Client::new(sock.clone()), "/p/none".into()).await;
        assert_eq!(got, Ok(None));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn list_projects_comes_back_as_ok() {
        let info = project_info(5, "/p/demo");
        let sock = serve_once(
            "pc-list",
            Some(Reply::Projects {
                projects: vec![info.clone()],
            }),
        );
        let got = ProjectService::list_projects(&Client::new(sock.clone())).await;
        assert_eq!(got, Ok(vec![info]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn new_methods_map_hang_up_and_error_reply_like_the_others() {
        let sock = serve_once("pc-hangup", None);
        let e = ProjectService::list_projects(&Client::new(sock.clone()))
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Unavailable, "daemon 断开")
        );
        let _ = std::fs::remove_file(&sock);

        let sock = serve_once(
            "pc-reject",
            Some(Reply::Error {
                message: "boom".into(),
            }),
        );
        let e = ProjectService::open_project(&Client::new(sock.clone()), "/p".into())
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod group_chat_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_services::{GroupChatService, ServiceErrorKind};

    #[tokio::test]
    async fn list_groups_comes_back_as_ok() {
        let sock = serve_once("gc-list", Some(Reply::Groups { groups: vec![] }));
        let got = GroupChatService::list_groups(&Client::new(sock.clone()), 7).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn a_unit_write_returns_unit_on_success() {
        let sock = serve_once("gc-delete", Some(Reply::Ok));
        let got = GroupChatService::delete_group(&Client::new(sock.clone()), 3).await;
        assert_eq!(got, Ok(()));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("gc-hangup", None);
        let e = GroupChatService::list_groups(&Client::new(sock.clone()), 7)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Unavailable, "daemon 断开")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once(
            "gc-reject",
            Some(Reply::Error {
                message: "boom".into(),
            }),
        );
        let e = GroupChatService::delete_group(&Client::new(sock.clone()), 3)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn a_tuple_result_is_carried_back_unchanged() {
        let sock = serve_once(
            "gc-msgs",
            Some(Reply::GroupMessages {
                messages: vec![],
                latest_rev: 42,
            }),
        );
        let got =
            GroupChatService::list_group_messages(&Client::new(sock.clone()), 5, 3, 200).await;
        assert_eq!(got, Ok((vec![], 42)));
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod conversation_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_services::{ConversationService, ServiceErrorKind};

    #[tokio::test]
    async fn rows_come_back_as_ok() {
        let sock = serve_once(
            "conv-ok",
            Some(Reply::ConversationsWithSummaries { rows: vec![] }),
        );
        let got = ConversationService::list_conversations_with_summaries(
            &Client::new(sock.clone()),
            "/p".into(),
            None,
            500,
            0,
        )
        .await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("conv-hangup", None);
        let e = ConversationService::list_conversations_with_summaries(
            &Client::new(sock.clone()),
            "/p".into(),
            None,
            500,
            0,
        )
        .await
        .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Unavailable, "daemon 断开")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once(
            "conv-reject",
            Some(Reply::Error {
                message: "boom".into(),
            }),
        );
        let e = ConversationService::list_conversations_with_summaries(
            &Client::new(sock.clone()),
            "/p".into(),
            None,
            500,
            0,
        )
        .await
        .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod code_health_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_services::{CodeHealthService, ServiceErrorKind};

    #[tokio::test]
    async fn get_returns_the_option_unchanged() {
        let sock = serve_once("ch-get", Some(Reply::CodeHealthReport { report: None }));
        let got = CodeHealthService::get_code_health_report(&Client::new(sock.clone()), 7).await;
        assert_eq!(got, Ok(None));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn list_comes_back_as_ok() {
        let sock = serve_once(
            "ch-list",
            Some(Reply::CodeHealthReports { reports: vec![] }),
        );
        let got =
            CodeHealthService::list_code_health_reports(&Client::new(sock.clone()), 7, 2).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("ch-hangup", None);
        let e = CodeHealthService::get_code_health_report(&Client::new(sock.clone()), 7)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Unavailable, "daemon 断开")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once(
            "ch-reject",
            Some(Reply::Error {
                message: "boom".into(),
            }),
        );
        let e = CodeHealthService::list_code_health_reports(&Client::new(sock.clone()), 7, 2)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod context_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_services::{ContextService, ServiceErrorKind};

    #[tokio::test]
    async fn list_comes_back_as_ok() {
        let sock = serve_once("ctx-list", Some(Reply::ContextItems { items: vec![] }));
        let got = ContextService::list_context_items(&Client::new(sock.clone()), 7).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn remove_returns_unit_on_success() {
        let sock = serve_once("ctx-remove", Some(Reply::Ok));
        let got = ContextService::remove_context_item(&Client::new(sock.clone()), 7, 3).await;
        assert_eq!(got, Ok(()));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("ctx-hangup", None);
        let e = ContextService::list_context_items(&Client::new(sock.clone()), 7)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Unavailable, "daemon 断开")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once(
            "ctx-reject",
            Some(Reply::Error {
                message: "boom".into(),
            }),
        );
        let e = ContextService::remove_context_item(&Client::new(sock.clone()), 7, 3)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn add_carries_the_item_back_unchanged() {
        let item = dozer_core::protocol::ContextItemInfo {
            id: 5,
            project_id: 7,
            entity_kind: "file".into(),
            entity_ref: "src/a.rs".into(),
            created_ms: 9,
        };
        let sock = serve_once("ctx-add", Some(Reply::ContextItem { item: item.clone() }));
        let got = ContextService::add_context_item(
            &Client::new(sock.clone()),
            7,
            "file".into(),
            "src/a.rs".into(),
        )
        .await;
        assert_eq!(got, Ok(item));
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod edit_history_service_tests {
    use super::todo_service_tests::{serve_once, temp_sock};
    use super::*;
    use dozer_core::protocol::MutationOutcome;
    use dozer_services::{EditHistoryService, PreciseEdit, ServiceErrorKind};
    use std::sync::{Arc, Mutex};

    /// 读一行业务请求,记下它,再回 `reply`。返回 (socket 路径, 记录请求行的共享槽)。
    /// 复审 R2:客户端先握手再发业务请求(两条连接),这里先应答握手。
    fn serve_capture(tag: &str, reply: Reply) -> (PathBuf, Arc<Mutex<Option<String>>>) {
        let sock = temp_sock(tag);
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        let seen = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&seen);
        tokio::spawn(async move {
            // 握手连接
            {
                let (stream, _) = listener.accept().await.unwrap();
                let (r, mut w) = stream.into_split();
                let mut lines = BufReader::new(r).lines();
                let _ = lines.next_line().await;
                let hello = Reply::Hello {
                    protocol: dozer_core::protocol::PROTOCOL_VERSION,
                    product: dozer_core::paths::product().name.to_string(),
                };
                let _ = w.write_all(encode_line(&hello).as_bytes()).await;
            }
            // 业务连接
            let (stream, _) = listener.accept().await.unwrap();
            let (r, mut w) = stream.into_split();
            let mut lines = BufReader::new(r).lines();
            if let Ok(Some(line)) = lines.next_line().await {
                *slot.lock().unwrap() = Some(line);
            }
            let _ = w.write_all(encode_line(&reply).as_bytes()).await;
        });
        (sock, seen)
    }

    fn edit() -> PreciseEdit {
        PreciseEdit {
            project_id: 7,
            path: "src/a.rs".into(),
            start_line: 1,
            start_col: 2,
            end_line: 3,
            end_col: 4,
            expected_text: "NEW".into(),
            new_text: "OLD".into(),
            summary: "撤销:改".into(),
            actor: "dozer".into(),
            session_id: "dozer-gui".into(),
        }
    }

    #[tokio::test]
    async fn list_comes_back_as_ok() {
        let sock = serve_once("eh-list", Some(Reply::FileEditHistory { entries: vec![] }));
        let got =
            EditHistoryService::list_file_edit_history(&Client::new(sock.clone()), 7, None, 200)
                .await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn list_sends_the_filter_and_limit_it_was_given() {
        let (sock, seen) = serve_capture("eh-list-req", Reply::FileEditHistory { entries: vec![] });
        let _ = EditHistoryService::list_file_edit_history(
            &Client::new(sock.clone()),
            7,
            Some("src/a.rs".into()),
            200,
        )
        .await;
        let line = seen.lock().unwrap().clone().expect("服务端应收到请求");
        match decode_line::<Request>(&line).unwrap() {
            Request::ListFileEditHistory {
                project_id,
                path_filter,
                limit,
            } => {
                assert_eq!(
                    (project_id, path_filter.as_deref(), limit),
                    (7, Some("src/a.rs"), 200)
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn apply_sends_every_field_in_the_right_place() {
        let (sock, seen) = serve_capture(
            "eh-apply-req",
            Reply::MutationResult {
                outcome: MutationOutcome::NotFound,
            },
        );
        let got = EditHistoryService::apply_precise_edit(&Client::new(sock.clone()), edit()).await;
        assert_eq!(got, Ok(MutationOutcome::NotFound), "outcome 原样带回");
        let line = seen.lock().unwrap().clone().expect("服务端应收到请求");
        match decode_line::<Request>(&line).unwrap() {
            Request::ApplyPreciseEdit {
                project_id,
                path,
                start_line,
                start_col,
                end_line,
                end_col,
                expected_text,
                new_text,
                summary,
                actor,
                session_id,
            } => {
                assert_eq!(
                    (
                        project_id,
                        path.as_str(),
                        start_line,
                        start_col,
                        end_line,
                        end_col
                    ),
                    (7, "src/a.rs", 1, 2, 3, 4)
                );
                assert_eq!((expected_text.as_str(), new_text.as_str()), ("NEW", "OLD"));
                assert_eq!(
                    (summary.as_str(), actor.as_str(), session_id.as_str()),
                    ("撤销:改", "dozer", "dozer-gui")
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("eh-hangup", None);
        let e =
            EditHistoryService::list_file_edit_history(&Client::new(sock.clone()), 7, None, 200)
                .await
                .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Unavailable, "daemon 断开")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once(
            "eh-reject",
            Some(Reply::Error {
                message: "boom".into(),
            }),
        );
        let e = EditHistoryService::apply_precise_edit(&Client::new(sock.clone()), edit())
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod bookmark_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_services::{BookmarkService, ServiceErrorKind};

    #[tokio::test]
    async fn list_comes_back_as_ok() {
        let sock = serve_once("bm-list", Some(Reply::Bookmarks { bookmarks: vec![] }));
        let got = BookmarkService::list_bookmarks(&Client::new(sock.clone()), Some(7)).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn add_and_remove_return_unit_on_success() {
        let sock = serve_once("bm-add", Some(Reply::Ok));
        let got = BookmarkService::add_bookmark(
            &Client::new(sock.clone()),
            dozer_core::protocol::BookmarkScope::Global,
            None,
            "http://a.com".into(),
            "A".into(),
        )
        .await;
        assert_eq!(got, Ok(()));
        let _ = std::fs::remove_file(&sock);

        let sock = serve_once("bm-remove", Some(Reply::Ok));
        let got = BookmarkService::remove_bookmark(&Client::new(sock.clone()), 3).await;
        assert_eq!(got, Ok(()));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("bm-hangup", None);
        let e = BookmarkService::list_bookmarks(&Client::new(sock.clone()), None)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Unavailable, "daemon 断开")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once(
            "bm-reject",
            Some(Reply::Error {
                message: "boom".into(),
            }),
        );
        let e = BookmarkService::remove_bookmark(&Client::new(sock.clone()), 3)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod session_protocol_golden {
    //! S6c-1 协议编码黄金测试:会话相关请求/应答的 `encode_line` 输出必须与
    //! 抽 `SessionService` 之前逐字一致。黄金串是在现状上跑出后写死的;
    //! 改动协议形状会让这里失败,必须显式更新并说明兼容影响。

    use super::*;
    use dozer_core::protocol::AgentState;

    fn sample_session() -> SessionInfo {
        SessionInfo {
            id: "sess-42".into(),
            name: "claude · demo".into(),
            command: "claude".into(),
            cwd: "/tmp/demo".into(),
            alive: true,
            created_ms: 1_700_000_000_000,
            agent_state: AgentState::Running,
            transcript_path: Some("/tmp/demo/t.jsonl".into()),
            project_id: Some(7),
            agent: AgentKind::Claude,
        }
    }

    #[test]
    fn session_protocol_encoding_golden() {
        let samples: [(&str, String, &str); 7] = [
            (
                "create_session",
                encode_line(&Request::CreateSession {
                    name: "claude · demo".into(),
                    command: "claude".into(),
                    args: vec!["--foo".into(), "bar baz".into()],
                    cwd: "/tmp/demo".into(),
                    cols: 120,
                    rows: 40,
                    project_id: 7,
                    agent: AgentKind::Claude,
                }),
                "{\"type\":\"create_session\",\"name\":\"claude · demo\",\"command\":\"claude\",\"args\":[\"--foo\",\"bar baz\"],\"cwd\":\"/tmp/demo\",\"cols\":120,\"rows\":40,\"project_id\":7,\"agent\":\"claude\"}\n",
            ),
            (
                "attach",
                encode_line(&Request::Attach {
                    session_id: "sess-42".into(),
                    from_offset: 4096,
                }),
                "{\"type\":\"attach\",\"session_id\":\"sess-42\",\"from_offset\":4096}\n",
            ),
            (
                "write",
                encode_line(&Request::Write {
                    session_id: "sess-42".into(),
                    data_b64: "aGVsbG8=".into(),
                }),
                "{\"type\":\"write\",\"session_id\":\"sess-42\",\"data_b64\":\"aGVsbG8=\"}\n",
            ),
            (
                "resize",
                encode_line(&Request::Resize {
                    session_id: "sess-42".into(),
                    cols: 132,
                    rows: 43,
                }),
                "{\"type\":\"resize\",\"session_id\":\"sess-42\",\"cols\":132,\"rows\":43}\n",
            ),
            (
                "kill",
                encode_line(&Request::Kill {
                    session_id: "sess-42".into(),
                }),
                "{\"type\":\"kill\",\"session_id\":\"sess-42\"}\n",
            ),
            (
                "created",
                encode_line(&Reply::Created {
                    session: sample_session(),
                }),
                "{\"type\":\"created\",\"session\":{\"id\":\"sess-42\",\"name\":\"claude · demo\",\"command\":\"claude\",\"cwd\":\"/tmp/demo\",\"alive\":true,\"created_ms\":1700000000000,\"agent_state\":\"running\",\"transcript_path\":\"/tmp/demo/t.jsonl\",\"project_id\":7,\"agent\":\"claude\"}}\n",
            ),
            (
                "attached",
                encode_line(&Reply::Attached {
                    session_id: "sess-42".into(),
                    snapshot_b64: "G1szMW1yZWQ=".into(),
                    next_offset: 8192,
                }),
                "{\"type\":\"attached\",\"session_id\":\"sess-42\",\"snapshot_b64\":\"G1szMW1yZWQ=\",\"next_offset\":8192}\n",
            ),
        ];
        for (name, actual, expected) in samples {
            assert_eq!(actual, expected, "会话协议编码变了: {name}");
        }
    }
}

#[cfg(test)]
mod session_service_tests {
    use super::todo_service_tests::temp_sock;
    use super::*;
    use dozer_core::protocol::AgentState;
    use dozer_model::SessionEvent;
    use dozer_services::{SessionService, SessionSpec};
    use std::any::TypeId;

    #[test]
    fn session_event_is_the_same_type_as_term_event() {
        assert_eq!(
            TypeId::of::<TermEvent>(),
            TypeId::of::<SessionEvent>(),
            "TermEvent 必须是 dozer_model::SessionEvent 的重新导出"
        );
    }

    fn sample_session() -> SessionInfo {
        SessionInfo {
            id: "sess-42".into(),
            name: "claude".into(),
            command: "claude".into(),
            cwd: "/tmp/demo".into(),
            alive: true,
            created_ms: 1,
            agent_state: AgentState::Idle,
            transcript_path: None,
            project_id: Some(7),
            agent: AgentKind::Claude,
        }
    }

    /// 假服务端:按请求类型应答,并把收到的业务请求回传给测试断言。
    fn serve_session_end_to_end(tag: &str) -> (PathBuf, mpsc::UnboundedReceiver<Request>) {
        let sock = temp_sock(tag);
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        let (tx, rx) = mpsc::unbounded_channel::<Request>();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let (r, mut w) = stream.into_split();
                    let mut lines = BufReader::new(r).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let req: Request = decode_line(&line).unwrap();
                        if matches!(req, Request::Hello { .. }) {
                            let _ = w
                                .write_all(
                                    encode_line(&Reply::Hello {
                                        protocol: dozer_core::protocol::PROTOCOL_VERSION,
                                        product: dozer_core::paths::product().name.to_string(),
                                    })
                                    .as_bytes(),
                                )
                                .await;
                            continue;
                        }
                        let reply = match &req {
                            Request::ListSessions => Reply::Sessions {
                                sessions: vec![sample_session()],
                            },
                            Request::CreateSession { .. } => Reply::Created {
                                session: sample_session(),
                            },
                            Request::Attach { .. } => Reply::Attached {
                                session_id: "sess-42".into(),
                                snapshot_b64: B64.encode(b"snap"),
                                next_offset: 12,
                            },
                            _ => Reply::Ok,
                        };
                        let _ = tx.send(req);
                        let _ = w.write_all(encode_line(&reply).as_bytes()).await;
                    }
                });
            }
        });
        (sock, rx)
    }

    #[tokio::test]
    async fn client_implements_session_service_end_to_end() {
        let (sock, mut seen) = serve_session_end_to_end("sess-e2e");
        let client = Client::new(sock.clone());
        let svc: &dyn SessionService = &client;

        let sessions = svc.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 1);

        let info = svc
            .create(SessionSpec {
                name: "claude".into(),
                command: "claude".into(),
                args: vec!["--foo".into()],
                cwd: "/tmp/demo".into(),
                cols: 120,
                rows: 40,
                project_id: 7,
                agent: AgentKind::Claude,
            })
            .await
            .unwrap();
        assert_eq!(info.id, "sess-42");

        let (snapshot, next, rx) = svc.attach("sess-42".into(), 0).await.unwrap();
        assert_eq!((snapshot.as_slice(), next), (&b"snap"[..], 12));
        drop(rx); // 关闭 attach 读任务

        svc.write("sess-42".into(), b"hello".to_vec())
            .await
            .unwrap();
        svc.resize("sess-42".into(), 132, 43).await.unwrap();
        svc.kill("sess-42".into()).await.unwrap();
        svc.close_with_summary("sess-42".into()).await.unwrap();

        let mut requests = Vec::new();
        while let Ok(req) = seen.try_recv() {
            requests.push(req);
        }
        assert!(matches!(requests[0], Request::ListSessions), "{requests:?}");
        assert!(
            matches!(
                &requests[1],
                Request::CreateSession {
                    name,
                    command,
                    args,
                    cwd,
                    cols: 120,
                    rows: 40,
                    project_id: 7,
                    agent: AgentKind::Claude,
                } if name == "claude" && command == "claude" && args == &vec!["--foo".to_string()]
                    && cwd == "/tmp/demo"
            ),
            "{requests:?}"
        );
        assert!(
            matches!(&requests[2], Request::Attach { session_id, from_offset: 0 } if session_id == "sess-42"),
            "{requests:?}"
        );
        assert!(
            matches!(&requests[3], Request::Write { session_id, data_b64 } if session_id == "sess-42" && data_b64 == "aGVsbG8="),
            "{requests:?}"
        );
        assert!(
            matches!(&requests[4], Request::Resize { session_id, cols: 132, rows: 43 } if session_id == "sess-42"),
            "{requests:?}"
        );
        assert!(
            matches!(&requests[5], Request::Kill { session_id } if session_id == "sess-42"),
            "{requests:?}"
        );
        assert!(
            matches!(&requests[6], Request::CloseWithSummary { session_id } if session_id == "sess-42"),
            "{requests:?}"
        );
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod conversation_protocol_golden {
    use super::*;
    use dozer_core::protocol::{SummaryJobInfo, SummaryJobStatus, SummaryTrigger, TurnRecord};

    #[test]
    fn conversation_protocol_encoding_golden() {
        let turn: TurnRecord = serde_json::from_value(serde_json::json!({
            "turn_index": 3, "role": "human", "content": "你好 👋\n第二行",
            "thinking": false, "ts": 42
        }))
        .unwrap();
        let job = SummaryJobInfo {
            job_id: 17,
            conversation_id: "会话-1".into(),
            source_session_id: None,
            trigger: SummaryTrigger::Manual,
            provider: AgentKind::Claude,
            requested_model: None,
            source_revision: "revision-2".into(),
            pipeline_version: "v2".into(),
            generation: 1,
            status: SummaryJobStatus::Failed,
            attempt: 2,
            error_kind: Some(dozer_core::protocol::SummaryErrorKind::Timeout),
            error_detail: Some("超时\n请重试".into()),
            created_ts_ms: 100,
            updated_ts_ms: 200,
            phase: Some("extract:1/2".into()),
        };
        let actual = [
            encode_line(&Request::GetConversationTurns {
                conversation_id: "会话-1".into(),
                after_turn_index: 2,
                limit: 200,
            }),
            encode_line(&Request::SubmitSummaryJob {
                conversation_id: "会话-1".into(),
                source_session_id: None,
                trigger: SummaryTrigger::Manual,
                provider: None,
                model: None,
                force: true,
            }),
            encode_line(&Request::GetSummaryJob { job_id: 17 }),
            encode_line(&Reply::ConversationTurns {
                conversation_id: "会话-1".into(),
                turns: vec![turn],
            }),
            encode_line(&Reply::SummaryJobSubmitted { job_id: 17 }),
            encode_line(&Reply::SummaryJob { job: Some(job) }),
        ]
        .concat();
        assert_eq!(
            actual,
            include_str!("../tests/fixtures/conversation_protocol_golden.txt")
        );
    }
}

#[cfg(test)]
mod conversation_detail_service_tests {
    use super::*;
    use dozer_services::SummaryService;

    fn turn(index: i64) -> TurnRecord {
        serde_json::from_value(serde_json::json!({
            "turn_index": index, "role": "human", "content": "你好 👋\n第二行",
            "thinking": false, "ts": 42
        }))
        .unwrap()
    }

    /// 真 UDS 与 Hello 握手;捕获所有业务请求以核对接缝是否改写参数。
    fn serve(
        replies: Vec<Reply>,
    ) -> (
        tempfile::TempDir,
        Client,
        mpsc::UnboundedReceiver<Request>,
        tokio::task::JoinHandle<()>,
    ) {
        let dir = tempfile::Builder::new()
            .prefix("s7a")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = dir.path().join("s.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut replies = replies.into_iter();
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let (r, mut w) = stream.into_split();
                let line = BufReader::new(r)
                    .lines()
                    .next_line()
                    .await
                    .unwrap()
                    .unwrap();
                let request: Request = decode_line(&line).unwrap();
                let reply = if matches!(request, Request::Hello { .. }) {
                    Reply::Hello {
                        protocol: dozer_core::protocol::PROTOCOL_VERSION,
                        product: dozer_core::paths::product().name.to_string(),
                    }
                } else {
                    tx.send(request).unwrap();
                    replies.next().expect("one reply per business request")
                };
                w.write_all(encode_line(&reply).as_bytes()).await.unwrap();
            }
        });
        (dir, Client::new(socket), rx, task)
    }

    #[tokio::test]
    async fn client_implements_get_conversation_turns_end_to_end() {
        let (_dir, client, mut seen, task) = serve(vec![
            Reply::ConversationTurns {
                conversation_id: "会话".into(),
                turns: vec![turn(8)],
            },
            Reply::Error {
                message: "拒绝读取".into(),
            },
            Reply::Ok,
        ]);
        let service: &dyn ConversationService = &client;
        assert_eq!(
            service
                .get_conversation_turns("会话".into(), 7, 200)
                .await
                .unwrap(),
            vec![turn(8)]
        );
        assert!(
            matches!(seen.recv().await.unwrap(), Request::GetConversationTurns {
            conversation_id, after_turn_index: 7, limit: 200,
        } if conversation_id == "会话")
        );
        let error = service
            .get_conversation_turns("会话".into(), -1, 1)
            .await
            .unwrap_err();
        assert_eq!(error.kind, ServiceErrorKind::Rejected);
        assert!(error.message.contains("拒绝读取"));
        assert_eq!(
            service
                .get_conversation_turns("会话".into(), 0, 0)
                .await
                .unwrap_err()
                .kind,
            ServiceErrorKind::Protocol
        );
        task.abort();
        let offline = Client::new(_dir.path().join("missing.sock"));
        assert_eq!(
            ConversationService::get_conversation_turns(&offline, "x".into(), -1, 200)
                .await
                .unwrap_err()
                .kind,
            ServiceErrorKind::Unavailable
        );
    }

    #[tokio::test]
    async fn client_implements_summary_service_end_to_end() {
        let job: dozer_model::SummaryJobInfo = serde_json::from_value(serde_json::json!({
            "job_id": 17, "conversation_id": "会话", "source_session_id": null,
            "trigger": "manual", "provider": "claude", "requested_model": null,
            "source_revision": "r1", "pipeline_version": "v2", "generation": 1,
            "status": "running", "attempt": 1, "error_kind": null,
            "created_ts_ms": 100, "updated_ts_ms": 200
        }))
        .unwrap();
        let (_dir, client, mut seen, task) = serve(vec![
            Reply::SummaryJobSubmitted { job_id: 17 },
            Reply::SummaryJobSubmitted { job_id: 18 },
            Reply::SummaryJob {
                job: Some(job.clone()),
            },
            Reply::SummaryJob { job: None },
            Reply::Error {
                message: "不能提交".into(),
            },
            Reply::Ok,
            Reply::Error {
                message: "不能查询".into(),
            },
            Reply::Ok,
        ]);
        let service: &dyn SummaryService = &client;
        for (force, expected) in [(false, 17), (true, 18)] {
            assert_eq!(
                service
                    .submit_summary_job("会话".into(), force)
                    .await
                    .unwrap(),
                expected
            );
            assert!(
                matches!(seen.recv().await.unwrap(), Request::SubmitSummaryJob {
                conversation_id, source_session_id: None,
                trigger: dozer_model::SummaryTrigger::Manual, provider: None, model: None,
                force: received_force,
            } if conversation_id == "会话" && received_force == force)
            );
        }
        assert_eq!(service.get_summary_job(17).await.unwrap(), Some(job));
        assert!(matches!(
            seen.recv().await.unwrap(),
            Request::GetSummaryJob { job_id: 17 }
        ));
        assert_eq!(service.get_summary_job(99).await.unwrap(), None);
        assert!(matches!(
            seen.recv().await.unwrap(),
            Request::GetSummaryJob { job_id: 99 }
        ));
        assert_eq!(
            service
                .submit_summary_job("会话".into(), true)
                .await
                .unwrap_err()
                .kind,
            ServiceErrorKind::Rejected
        );
        assert_eq!(
            service
                .submit_summary_job("会话".into(), true)
                .await
                .unwrap_err()
                .kind,
            ServiceErrorKind::Protocol
        );
        assert_eq!(
            service.get_summary_job(17).await.unwrap_err().kind,
            ServiceErrorKind::Rejected
        );
        assert_eq!(
            service.get_summary_job(17).await.unwrap_err().kind,
            ServiceErrorKind::Protocol
        );
        task.abort();
        let offline = Client::new(_dir.path().join("missing.sock"));
        assert_eq!(
            SummaryService::submit_summary_job(&offline, "x".into(), true)
                .await
                .unwrap_err()
                .kind,
            ServiceErrorKind::Unavailable
        );
        assert_eq!(
            SummaryService::get_summary_job(&offline, 17)
                .await
                .unwrap_err()
                .kind,
            ServiceErrorKind::Unavailable
        );
    }
}
