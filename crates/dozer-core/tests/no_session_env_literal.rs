//! S6a Task 2 守护测试:除 `ProductId::dozer()` 的定义与测试代码外,
//! 整个 workspace 的源码里不得再出现字面量 "DOZER_SESSION_ID"——
//! 会话环境变量名必须经 `dozer_core::paths::session_env()` 取得,
//! 否则 Digger 的嵌套会话会带着 Dozer 的变量名串线。

use std::path::{Path, PathBuf};

/// 从 workspace 根向下扫描:以 dozer-core 的位置为锚点找根。
fn workspace_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .nth(2)
        .expect("dozer-core 位于 <root>/crates/dozer-core")
        .to_path_buf()
}

/// 逐行判定:从出现 #[cfg(test)] 的那行起,之后的代码都算测试。
/// 本仓库约定测试模块写在文件末尾,#[cfg(test)] 之前全是生产代码;
/// 行级判定避免"文件里有测试模块 → 整个文件被豁免"的漏网。
fn offending_lines(text: &str) -> usize {
    let mut in_test = false;
    let mut hits = 0;
    for line in text.lines() {
        if line.trim_start().starts_with("#[cfg(test)]") {
            in_test = true;
        }
        if !in_test && line.contains("\"DOZER_SESSION_ID\"") {
            hits += 1;
        }
    }
    hits
}

#[test]
fn no_literal_session_env_name_outside_the_product_definition() {
    let root = workspace_root();
    let mut offenders: Vec<String> = Vec::new();
    let mut visited = 0;

    fn scan(root: &Path, dir: &Path, is_integration_test: bool, offenders: &mut Vec<String>, visited: &mut usize) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || !path.extension().is_some_and(|e| e == "rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            *visited += 1;
            let rel = path
                .strip_prefix(root)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| path.display().to_string());
            // 唯一合法产地:ProductId::dozer() 的定义
            if rel == "crates/dozer-core/src/paths.rs" {
                continue;
            }
            // crates/*/tests/ 下的集成测试整文件都是测试代码
            if is_integration_test {
                continue;
            }
            if offending_lines(&text) > 0 {
                offenders.push(rel);
            }
        }
    }

    for crate_dir in std::fs::read_dir(root.join("crates"))
        .expect("crates 目录")
        .flatten()
    {
        for sub in ["src", "examples", "benches"] {
            scan(&root, &crate_dir.path().join(sub), false, &mut offenders, &mut visited);
        }
        scan(&root, &crate_dir.path().join("tests"), true, &mut offenders, &mut visited);
    }

    assert!(
        offenders.is_empty(),
        "以下 {n} 个文件在产品定义与测试代码之外硬编码了会话变量名:\n{list}\n\
         请改用 dozer_core::paths::session_env()(共扫描 {visited} 个 .rs 文件)",
        n = offenders.len(),
        list = offenders.join("\n"),
        visited = visited,
    );
}
