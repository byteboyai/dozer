//! MCP 工具特性分组(S6e-1)。未开启的工具不出现在 `tools/list` 里,
//! 调用它们得到"未知工具"。

/// 工具分组。`Default`/`none()` 全关,`all()` 全开。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct McpFeatures {
    pub memory: bool,
    pub summary: bool,
    pub todos: bool,
    pub preview: bool,
    pub file_edit: bool,
}

impl McpFeatures {
    pub fn all() -> Self {
        Self {
            memory: true,
            summary: true,
            todos: true,
            preview: true,
            file_edit: true,
        }
    }

    pub fn none() -> Self {
        Self::default()
    }

    /// 当前开启的工具名(与服务器注册的十二个工具一一对应)。
    pub fn tool_names(self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if self.memory {
            names.extend(["write_memory", "list_memories", "get_memory"]);
        }
        if self.summary {
            names.push("submit_session_summary");
        }
        if self.todos {
            names.extend(["list_todos", "add_todo", "toggle_todo", "edit_todo_text"]);
        }
        if self.preview {
            names.extend(["get_preview_context", "preview_navigate", "locate_in_file"]);
        }
        if self.file_edit {
            names.push("apply_precise_edit");
        }
        names
    }

    /// 全部工具名(忽略开关),供测试核对覆盖完备性。
    pub fn all_tool_names() -> Vec<&'static str> {
        Self::all().tool_names()
    }
}
