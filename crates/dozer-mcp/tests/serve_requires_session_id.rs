use std::process::Command;

#[test]
fn serve_exits_nonzero_without_session_id_env() {
    let exe = env!("CARGO_BIN_EXE_dozer-mcp");
    let status = Command::new(exe)
        .arg("serve")
        .env_remove("DOZER_SESSION_ID")
        .status()
        .expect("spawn dozer-mcp");
    assert!(!status.success());
}
