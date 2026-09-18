//! "创建项目"对话框:两 tab(本地新建 / Git URL 签出)状态机 + 视图。
//! 设计见 `docs/superpowers/specs/2026-09-18-new-project-creation-design.md`。
//! 渲染宿主是独立原生窗口 `platform::project_create_overlay::
//! ProjectCreateOverlay`,结构对照 `extensions::file_history` + 同名 overlay
//! 的既有分工:本模块只管状态/消息/视图/异步落盘逻辑,不碰 winit/wgpu。

use std::path::{Path, PathBuf};

/// 最终项目路径 = 根目录/项目名称。纯字符串拼接,不做存在性判断
/// (存在性判断是 [`validate_target_not_exists`] 的职责,分开是因为提交
/// 前两处都要单独调用:先拼路径给用户预览,再单独校验)。
pub(crate) fn target_path(root_dir: &str, name: &str) -> PathBuf {
    Path::new(root_dir).join(name)
}

/// 项目名称合法性:非空、首尾无空白、不含路径分隔符。
pub(crate) fn validate_project_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("项目名称不能为空".to_string());
    }
    if trimmed != name {
        return Err("项目名称首尾不能有空白字符".to_string());
    }
    if name.contains('/') || name.contains('\\') {
        return Err("项目名称不能包含 / 或 \\".to_string());
    }
    Ok(())
}

/// 目标目录不能已存在——创建/签出只认全新目录,"已存在则复用"是"打开
/// 项目"该管的语义(见 spec「数据流」一节)。
pub(crate) fn validate_target_not_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        Err(format!("目标目录已存在: {}", path.display()))
    } else {
        Ok(())
    }
}

/// 从远程仓库 URL 推导默认项目名称——取最后一段路径,去掉 `.git` 后缀。
/// 同时兼容 `https://host/group/repo.git`、`https://host/group/repo`、
/// scp 风格 `git@host:group/repo.git`、带结尾斜杠的 `.../repo/`。
pub(crate) fn derive_project_name_from_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let last_segment = trimmed.rsplit(['/', ':']).next().unwrap_or(trimmed);
    last_segment
        .strip_suffix(".git")
        .unwrap_or(last_segment)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_path_joins_root_and_name() {
        assert_eq!(
            target_path("/tmp/projects", "foo"),
            PathBuf::from("/tmp/projects/foo")
        );
    }

    #[test]
    fn validate_project_name_rejects_empty_and_whitespace_and_slash() {
        assert!(validate_project_name("").is_err());
        assert!(validate_project_name("   ").is_err());
        assert!(validate_project_name(" foo").is_err());
        assert!(validate_project_name("foo ").is_err());
        assert!(validate_project_name("a/b").is_err());
        assert!(validate_project_name("a\\b").is_err());
        assert!(validate_project_name("foo").is_ok());
    }

    #[test]
    fn validate_target_not_exists_rejects_existing_path() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(validate_target_not_exists(tmp.path()).is_err());
        assert!(validate_target_not_exists(&tmp.path().join("does-not-exist")).is_ok());
    }

    #[test]
    fn derive_project_name_from_url_handles_common_forms() {
        assert_eq!(
            derive_project_name_from_url("https://github.com/abc/foo.git"),
            "foo"
        );
        assert_eq!(
            derive_project_name_from_url("https://github.com/abc/foo"),
            "foo"
        );
        assert_eq!(
            derive_project_name_from_url("git@gitlab.com:abc/bar.git"),
            "bar"
        );
        assert_eq!(
            derive_project_name_from_url("https://gitee.com/abc/baz/"),
            "baz"
        );
    }
}
