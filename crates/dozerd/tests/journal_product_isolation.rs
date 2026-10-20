//! S6a 复审 R3(方案 A)读写一致:hook 侧写 journal 与 dozerd 摄取侧
//! 扫描必须读到同一个路径——两侧只经 dozer_core::agent_paths 取路径。
//! 本文件安装 Digger 身份(进程级,独立集成测试),以非 Dozer 身份走一遍
//! "hook 写入 → 摄取扫描发现",并断言 Dozer 的 ~/.dozer 下没有新增文件。

use dozer_core::agent_paths::opencode_project_dir_in;
use dozer_core::paths::{ProductDirs, ProductId};
use std::path::Path;

fn digger(root: &Path) -> ProductId {
    ProductId::custom(
        "digger",
        "DIGGER_SESSION_ID",
        "digger-hook",
        "diggerd.sock",
        "digger.db",
        ProductDirs::Fixed {
            config: root.join("config"),
            state: root.join("state"),
        },
    )
}

#[test]
fn hook_write_and_ingest_scan_read_the_same_product_path() {
    let root = tempfile::tempdir().unwrap();
    dozer_core::paths::install(digger(root.path())).expect("安装 Digger 身份");

    let home = tempfile::tempdir().unwrap();
    let dozer_home_snapshot: Vec<_> = std::fs::read_dir(home.path())
        .map(|it| {
            it.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();

    // hook 侧写入(与 dozer-hook::opencode::append_transcript_line_in 相同的
    // 生产路径构造——它现在转调 dozer_core::agent_paths::opencode_project_dir_in)
    let cwd = "/proj/digger-journal";
    let session_id = "sess-r3";
    let path =
        opencode_project_dir_in(home.path(), Path::new(cwd)).join(format!("{session_id}.jsonl"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{\"type\":\"user\"}\n").unwrap();

    // 断言1:文件确实落在 Digger 的根目录下
    assert!(
        path.to_string_lossy().contains(".digger/agents/opencode"),
        "{path:?}"
    );

    // 断言2:摄取扫描在同一 home 下能发现它
    let found = dozerd::transcripts::scan::discover_project_transcript_files_in(
        home.path(),
        Path::new(cwd),
    );
    let found_all = dozerd::transcripts::scan::discover_all_transcript_files_in(home.path());
    let expected = path.canonicalize().unwrap();
    assert!(
        found
            .iter()
            .any(|(_, p)| p.canonicalize().map(|c| c == expected).unwrap_or(false)),
        "按项目扫描应发现 hook 写入的 journal: {found:?}"
    );
    assert!(
        found_all
            .iter()
            .any(|(_, p)| p.canonicalize().map(|c| c == expected).unwrap_or(false)),
        "全量扫描应发现 hook 写入的 journal"
    );

    // 断言3:Dozer 的 journal 根(~/.dozer)完全没被碰过;home 顶层只出现
    // Digger 自己的根目录
    assert!(
        !home.path().join(".dozer").exists(),
        "Dozer 的 journal 根目录不得被 Digger 写入"
    );
    let after: Vec<_> = std::fs::read_dir(home.path())
        .map(|it| {
            it.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        after,
        vec![".digger".to_string()],
        "home 顶层只应有 Digger 的根目录"
    );
    let _ = dozer_home_snapshot;
}
