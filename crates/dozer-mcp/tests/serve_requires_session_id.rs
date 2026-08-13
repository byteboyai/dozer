use std::process::Command;

#[test]
fn serve_exits_nonzero_without_session_id_env() {
    let exe = env!("CARGO_BIN_EXE_dozer-mcp");
    let status = Command::new(exe)
        .arg("serve")
        .env_remove("DOZER_SESSION_ID")
        // 断开 stdin：万一哪天 env 守卫被摘掉，`serve` 会去挂 stdio
        // transport 读测试进程的 stdin，然后永远卡住。给个 /dev/null
        // 让它立刻 EOF 退出，退化成"测试失败"而不是"测试挂死"。
        .stdin(std::process::Stdio::null())
        .status()
        .expect("spawn dozer-mcp");
    assert!(!status.success());
}
