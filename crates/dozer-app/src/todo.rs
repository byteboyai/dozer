//! `.dozer/todo.md` 任务列表解析（Todo 面板 design，2026-08-06）：
//! 标准两态 checkbox（`- [ ]`/`- [x]`），git 可追踪，agent 可直接读写。
//! 解析风格镜像 `goal.rs`——手写、宽松，不引入 markdown 库；格式意外
//! （多级缩进、非 checkbox 正文）一律忽略，不因为文件"长得不标准"而失败。

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::todo_meta::DispatchRecord;

#[derive(Debug, Clone, PartialEq)]
pub struct TodoItem {
    pub text: String,
    pub done: bool,
}

pub fn todo_path(repo: &Path) -> PathBuf {
    repo.join(".dozer").join("todo.md")
}

/// 宽松解析：只认一级 `- [ ]`/`- [x]` 列表项（`trim` 后必须以这两个前缀
/// 之一开头——多级缩进的子项 `trim` 后前导空格会被吃掉，但因为前面还有
/// `- [ ]` 的兄弟节点占了行首，不会被误判成一级项，见 `parses_pending_
/// and_done_items` 与 `ignores_non_checkbox_lines_and_blank_file` 两个
/// 测试）；其余行（标题、正文）一律忽略，不因为格式意外而失败。文件不
/// 存在/为空 → 空列表，不是 `Option`（跟 `goal.rs::parse_goal` 不同——
/// todo 没有"整份文件代表一个目标"这种要么有要么没有的语义）。
pub fn parse_todo(md: &str) -> Vec<TodoItem> {
    let mut items = Vec::new();
    for line in md.lines() {
        // 只有顶格（列 0）的 `- [ ]`/`- [x]` 才算 Dozer 面板的一级任务；
        // 缩进的子任务属于 agent 自己的清单，不归面板管（见 design）。
        if let Some(rest) = line.strip_prefix("- [ ]") {
            items.push(TodoItem {
                text: rest.trim().to_string(),
                done: false,
            });
        } else if let Some(rest) = line.strip_prefix("- [x]") {
            items.push(TodoItem {
                text: rest.trim().to_string(),
                done: true,
            });
        }
    }
    items
}

/// 在 `content` 里找到与 `old_line` 逐字节相同的一行（第一次出现），
/// 替换成 `new_line`。找不到（文件已被 agent 并发改过）返回 `None`，
/// 调用方按"冲突，放弃这次写入，强制重读"处理，不是错误（见 design
/// 第 6 节）。
pub fn replace_todo_line(content: &str, old_line: &str, new_line: &str) -> Option<String> {
    let mut found = false;
    let mut out = String::with_capacity(content.len());
    for line in content.lines() {
        if !found && line == old_line {
            out.push_str(new_line);
            found = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    found.then_some(out)
}

/// 在最后一个 `- [ ]`/`- [x]` 行之后追加一条新任务；纯追加不依赖"找到
/// 匹配行"，冲突面比 `replace_todo_line` 小。文件里一条任务都没有时，
/// 追加在文件末尾（保留原有内容，末尾补一个换行再接新行，避免跟最后
/// 一行内容粘连）。
pub fn append_todo_item(content: &str, text: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let last_item_idx = lines
        .iter()
        .rposition(|l| l.trim_start().starts_with("- [ ]") || l.trim_start().starts_with("- [x]"));
    let insert_at = last_item_idx.map(|i| i + 1).unwrap_or(lines.len());
    let mut out = String::with_capacity(content.len() + text.len() + 8);
    for (i, line) in lines.iter().enumerate() {
        if i == insert_at {
            out.push_str("- [ ] ");
            out.push_str(text);
            out.push('\n');
        }
        out.push_str(line);
        out.push('\n');
    }
    if insert_at == lines.len() {
        out.push_str("- [ ] ");
        out.push_str(text);
        out.push('\n');
    }
    out
}

/// 派发记录/计划时间/完成时间在 GUI 本地 sidecar 里用这个 key 关联到
/// 具体某条任务——不给 markdown 行发明稳定 id（那需要往文件里塞隐藏
/// 标记，agent 编辑时容易破坏），代价是"改了任务文字会跟丢这条的全部
/// 本地元数据"，v1 接受（design 非目标）。用文本 `trim` 后算哈希，不
/// 要求无碰撞，只要求"实践中够用"，同 `AgentKind` 分组等既有哈希用途
/// 的验收标准。
pub fn todo_line_key(text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.trim().hash(&mut hasher);
    hasher.finish()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoState {
    Pending,
    InProgress,
    Done,
}

/// `done` 为真直接 `Done`（不管有没有派发记录——已完成的任务不需要
/// 再关心是谁做的）；否则看有没有派发记录，记录存在且目标 session
/// 仍存活（`target_alive`，调用方传 `ws.tabs.iter().any(|t| t.info.id
/// == dispatch.session_id && t.alive)`）→ `InProgress`；否则（没派发
/// 过，或派发目标已经退出）→ `Pending`。`plan_date`/`completed_at`
/// 不参与这个推导，跟三态是两件事（design 第 4/8 节）。
pub fn todo_display_state(
    item: &TodoItem,
    dispatch: Option<&DispatchRecord>,
    target_alive: bool,
) -> TodoState {
    if item.done {
        return TodoState::Done;
    }
    if dispatch.is_some() && target_alive {
        TodoState::InProgress
    } else {
        TodoState::Pending
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoFilter {
    All,
    Pending,
    InProgress,
    Done,
}

/// 纯前端过滤：状态相等匹配 + 关键字对 `TodoItem.text` 做大小写不敏感
/// 的子串匹配（空 `query` 不过滤）。作用在"已经解析+推导好状态"的
/// 内存列表上，不碰文件、不碰 sidecar（design 第 7 节）。
pub fn filter_todos<'a>(
    items: &'a [TodoItem],
    states: &[TodoState],
    filter: TodoFilter,
    query: &str,
) -> Vec<usize> {
    let query_lower = query.trim().to_lowercase();
    items
        .iter()
        .zip(states.iter())
        .enumerate()
        .filter(|(_, (_, state))| match filter {
            TodoFilter::All => true,
            TodoFilter::Pending => **state == TodoState::Pending,
            TodoFilter::InProgress => **state == TodoState::InProgress,
            TodoFilter::Done => **state == TodoState::Done,
        })
        .filter(|(_, (item, _))| {
            query_lower.is_empty() || item.text.to_lowercase().contains(&query_lower)
        })
        .map(|(i, _)| i)
        .collect()
}

/// `done` 翻转成 `completed_at` 该有的值：完成 → `Some(now)`，取消
/// 完成 → `None`。抽成纯函数是为了能不起 GUI/不碰文件单测这条转换
/// 规则本身。
pub fn completed_at_for_toggle(
    done: bool,
    now: std::time::SystemTime,
) -> Option<std::time::SystemTime> {
    done.then_some(now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn todo_path_is_dot_dozer() {
        assert_eq!(
            todo_path(Path::new("/repo")),
            PathBuf::from("/repo/.dozer/todo.md")
        );
    }

    #[test]
    fn parses_pending_and_done_items() {
        let md = "# Todo\n\n- [ ] 修复登录页闪烁\n- [x] 补 README 安装说明\n";
        let items = parse_todo(md);
        assert_eq!(
            items,
            vec![
                TodoItem {
                    text: "修复登录页闪烁".to_string(),
                    done: false
                },
                TodoItem {
                    text: "补 README 安装说明".to_string(),
                    done: true
                },
            ]
        );
    }

    #[test]
    fn ignores_non_checkbox_lines_and_blank_file() {
        let md = "# Todo\n\n正文说明，不是任务。\n- 普通列表项也不算\n  - [ ] 缩进的不算一级\n";
        assert_eq!(parse_todo(md), Vec::new());
        assert_eq!(parse_todo(""), Vec::new());
    }

    #[test]
    fn replace_todo_line_hits_and_replaces() {
        let content = "# Todo\n\n- [ ] 任务A\n- [ ] 任务B\n";
        let out = replace_todo_line(content, "- [ ] 任务A", "- [x] 任务A").unwrap();
        assert_eq!(out, "# Todo\n\n- [x] 任务A\n- [ ] 任务B\n");
    }

    #[test]
    fn replace_todo_line_misses_returns_none() {
        let content = "# Todo\n\n- [ ] 任务A\n";
        assert_eq!(replace_todo_line(content, "- [ ] 不存在的行", "x"), None);
    }

    #[test]
    fn replace_todo_line_only_replaces_first_match() {
        // 已知限制：文件里有多行完全相同的文本时，只替换第一次出现。
        let content = "- [ ] 重复\n- [ ] 重复\n";
        let out = replace_todo_line(content, "- [ ] 重复", "- [x] 重复").unwrap();
        assert_eq!(out, "- [x] 重复\n- [ ] 重复\n");
    }

    #[test]
    fn append_todo_item_to_empty_list() {
        let content = "# Todo\n";
        assert_eq!(
            append_todo_item(content, "新任务"),
            "# Todo\n- [ ] 新任务\n"
        );
    }

    #[test]
    fn append_todo_item_after_last_existing_item() {
        let content = "# Todo\n\n- [ ] 任务A\n- [x] 任务B\n";
        assert_eq!(
            append_todo_item(content, "任务C"),
            "# Todo\n\n- [ ] 任务A\n- [x] 任务B\n- [ ] 任务C\n"
        );
    }

    #[test]
    fn todo_line_key_ignores_surrounding_whitespace_but_not_content() {
        assert_eq!(todo_line_key("  任务A  "), todo_line_key("任务A"));
        assert_ne!(todo_line_key("任务A"), todo_line_key("任务B"));
    }

    fn item(done: bool) -> TodoItem {
        TodoItem {
            text: "任务".to_string(),
            done,
        }
    }

    fn record() -> DispatchRecord {
        DispatchRecord {
            session_id: "sess".to_string(),
            dispatched_at: std::time::SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn done_item_is_always_done_regardless_of_dispatch() {
        assert_eq!(
            todo_display_state(&item(true), None, false),
            TodoState::Done
        );
        assert_eq!(
            todo_display_state(&item(true), Some(&record()), true),
            TodoState::Done
        );
    }

    #[test]
    fn pending_without_dispatch_is_pending() {
        assert_eq!(
            todo_display_state(&item(false), None, false),
            TodoState::Pending
        );
    }

    #[test]
    fn pending_with_live_dispatch_is_in_progress() {
        assert_eq!(
            todo_display_state(&item(false), Some(&record()), true),
            TodoState::InProgress
        );
    }

    #[test]
    fn pending_with_dead_dispatch_falls_back_to_pending() {
        assert_eq!(
            todo_display_state(&item(false), Some(&record()), false),
            TodoState::Pending
        );
    }

    fn sample() -> (Vec<TodoItem>, Vec<TodoState>) {
        let items = vec![
            TodoItem {
                text: "修复登录页闪烁".to_string(),
                done: false,
            },
            TodoItem {
                text: "补 README 安装说明".to_string(),
                done: false,
            },
            TodoItem {
                text: "移除死代码".to_string(),
                done: true,
            },
        ];
        let states = vec![TodoState::Pending, TodoState::InProgress, TodoState::Done];
        (items, states)
    }

    #[test]
    fn filter_all_with_empty_query_keeps_everything() {
        let (items, states) = sample();
        assert_eq!(
            filter_todos(&items, &states, TodoFilter::All, ""),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn filter_by_state() {
        let (items, states) = sample();
        assert_eq!(filter_todos(&items, &states, TodoFilter::Done, ""), vec![2]);
        assert_eq!(
            filter_todos(&items, &states, TodoFilter::InProgress, ""),
            vec![1]
        );
    }

    #[test]
    fn filter_by_keyword_case_insensitive_substring() {
        let (items, states) = sample();
        assert_eq!(
            filter_todos(&items, &states, TodoFilter::All, "readme"),
            vec![1]
        );
        assert_eq!(
            filter_todos(&items, &states, TodoFilter::All, "登录"),
            vec![0]
        );
    }

    #[test]
    fn filter_combines_state_and_keyword() {
        let (items, states) = sample();
        // "README" 只在下标 1，且下标 1 是 InProgress——命中；换成 Done 就不命中了。
        assert_eq!(
            filter_todos(&items, &states, TodoFilter::InProgress, "readme"),
            vec![1]
        );
        assert!(filter_todos(&items, &states, TodoFilter::Done, "readme").is_empty());
    }

    #[test]
    fn done_gets_timestamp_undone_gets_none() {
        let now = SystemTime::now();
        assert_eq!(completed_at_for_toggle(true, now), Some(now));
        assert_eq!(completed_at_for_toggle(false, now), None);
    }
}
