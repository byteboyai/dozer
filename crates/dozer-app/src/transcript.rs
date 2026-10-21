//! Claude Code transcript(JSONL)适配器（P1i）。spec 2026-08-20 起回合明细
//! 改为直接消费 dozerd 查询回来的 `TurnRecord`（`review_entries_from_turns`），
//! 不再在 app 侧碰磁盘 JSONL。本文件保留的 `latest_model_mode_and_activity`
//! 系列仍直接读 transcript 原始文本（Agent 卡片实时指示器用），不在本次
//! 迁移范围。纯函数，不碰 iced/IO。

use dozer_core::protocol::{ToolCallInfo, TurnRecord};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum ReviewEntry {
    /// 人类发言（导航锚点）。
    Human { text: String },
    /// AI 一个逻辑回合：正文 + 真实思考文本 + 结构化工具调用 + 紧跟它的
    /// 工具结果。上游 transcript 常把同一次回复拆成多条 `role:"ai"` 的
    /// `TurnRecord`(先几条只有 tool_use、没有正文,最后一条才是给人看
    /// 的回复文本),2026-08-22 起 `review_entries_from_turns` 会把这类
    /// "正文还没落地"的连续 `ai` 行折叠进同一个 `AiTurn`,直到出现带正文
    /// 的那一条才定稿——不然轨迹步骤会各自顶格显示成一堆只有"轨迹 ▸"、
    /// 没有正文的空气泡,和真正的回复正文断成两截。`ToolResult` 的折叠
    /// 规则不变:同一 `AiTurn` 之后、下一个 `Human`/`AiTurn` 之前的连续
    /// `ToolResult` 都算它的。
    AiTurn {
        text: String,
        thinking_text: Option<String>,
        tool_calls: Vec<ToolCallInfo>,
        tool_results: Vec<ToolResultEntry>,
        /// 这个逻辑回合(可能是好几条 `ai` 行折叠来的)累加的 token 用量——
        /// 每条 `ai` 行的四个字段是它自己那一次 API 调用的用量,折叠时逐项
        /// 相加就是这句回复的总花费,不是均摊/去重(同一逻辑回合的多次
        /// API 调用本来就都要计费)。
        tokens_in: u64,
        tokens_out: u64,
        tokens_cache_read: u64,
        tokens_cache_write: u64,
    },
    /// 孤儿兜底:前面没有 `AiTurn` 的 `ToolResult`(理论边界情况,如导出
    /// 片段从工具结果行开始)。正常情况下 `ToolResult` 都会被折叠进
    /// 上面 `AiTurn::tool_results`,这个顶层变体只在没有归属对象时才用。
    ToolResult {
        content: String,
        is_error: bool,
        call_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolResultEntry {
    pub content: String,
    pub is_error: bool,
    /// 这条结果对应的调用 id；拿不到时 None，前端回落下标近似。见
    /// docs/superpowers/specs/2026-09-25-tool-call-result-pairing-design.md。
    pub call_id: Option<String>,
}

/// dozerd 查询回来的回合明细 → 面板展示用的 `ReviewEntry`。顺序 fold 而
/// 不是逐条 map:
/// - `tool_result` 角色的行按位置邻接归到紧邻它前面那个 `AiTurn` 的
///   `tool_results`(`out.last_mut()` 还是同一个 `AiTurn` 就一直往里塞),
///   前面没有 `AiTurn`(比如会话/导出片段从工具结果开始)才落回顶层
///   `ReviewEntry::ToolResult`。
/// - `ai` 角色的行:如果紧邻前面那个 `AiTurn` 还没有正文(`text` 为空,
///   说明它到目前为止只是纯轨迹——思考/工具调用),就把当前行折叠进去
///   (正文换成当前行的,thinking_text/tool_calls 累加),而不是另起一个
///   顶层条目;一旦某个 `AiTurn` 有了正文就算定稿,后续再来的 `ai` 行
///   会开启新的顶层条目。这样"最终回复前的一串纯工具调用回合"会跟那句
///   回复正文合成一个条目,回复正文下面折叠的就是它的完整轨迹。
pub fn review_entries_from_turns(turns: &[TurnRecord]) -> Vec<ReviewEntry> {
    let mut out: Vec<ReviewEntry> = Vec::new();
    for t in turns {
        match t.role.as_str() {
            "human" => out.push(ReviewEntry::Human {
                text: t.content.clone(),
            }),
            // Codex 的用量记录(见 dozerd parse.rs::parse_codex_shaped_chunk)
            // 没有正文,只用于用量求和,不该出现在时间线里。
            "token_usage" => {}
            "tool_result" => {
                let entry = ToolResultEntry {
                    content: t.content.clone(),
                    is_error: t.is_error,
                    call_id: t.tool_result_call_id.clone(),
                };
                match out.last_mut() {
                    Some(ReviewEntry::AiTurn { tool_results, .. }) => {
                        tool_results.push(entry);
                    }
                    _ => out.push(ReviewEntry::ToolResult {
                        content: entry.content,
                        is_error: entry.is_error,
                        call_id: entry.call_id,
                    }),
                }
            }
            _ => {
                let pending = matches!(
                    out.last(),
                    Some(ReviewEntry::AiTurn { text, .. }) if text.is_empty()
                );
                if pending {
                    let Some(ReviewEntry::AiTurn {
                        text,
                        thinking_text,
                        tool_calls,
                        tokens_in,
                        tokens_out,
                        tokens_cache_read,
                        tokens_cache_write,
                        ..
                    }) = out.last_mut()
                    else {
                        unreachable!("just matched AiTurn above");
                    };
                    *text = t.content.clone();
                    match (thinking_text.as_mut(), &t.thinking_text) {
                        (Some(existing), Some(new_text)) => {
                            existing.push('\n');
                            existing.push_str(new_text);
                        }
                        (None, Some(new_text)) => *thinking_text = Some(new_text.clone()),
                        _ => {}
                    }
                    tool_calls.extend(t.tool_calls.iter().cloned());
                    *tokens_in += t.tokens_in;
                    *tokens_out += t.tokens_out;
                    *tokens_cache_read += t.tokens_cache_read;
                    *tokens_cache_write += t.tokens_cache_write;
                } else {
                    out.push(ReviewEntry::AiTurn {
                        text: t.content.clone(),
                        thinking_text: t.thinking_text.clone(),
                        tool_calls: t.tool_calls.clone(),
                        tool_results: Vec::new(),
                        tokens_in: t.tokens_in,
                        tokens_out: t.tokens_out,
                        tokens_cache_read: t.tokens_cache_read,
                        tokens_cache_write: t.tokens_cache_write,
                    });
                }
            }
        }
    }
    out
}

/// 从 transcript 尾部提取最后一次出现的 model id / permissionMode(后
/// 出现的覆盖先出现的,只关心最新值)。model 兼认三种互斥的行形状:
/// Claude(`message.model`)、Codebuddy(顶层 `providerData.model`,字段名
/// 核对自 `crates/dozer-hook/fixtures/codebuddy-transcript-sample.jsonl`
/// 真实样本)和 Codex(独立 `turn_context` 行的 `payload.model`,字段名
/// 核对自 2026-09-21 实读本机 `~/.codex/sessions/**/rollout-*.jsonl`)
/// ——同一份 transcript 只会是其中一种形状,三条路径共存不冲突,
/// 不需要按 `agent` 分派。`permissionMode` 只有 Claude 形状(`user`/
/// `assistant`/`permission-mode` 三种行的顶层)才有,Codebuddy 没有等价
/// 字段,不提取,`mode` 对 Codebuddy 恒 `None`(Agent 卡片视图据此不渲染
/// Mode 行,不是 bug)。解析失败的行跳过,不中断整体扫描(同
/// `parse_claude_shaped_jsonl` 的既有容错口径)。不像 `parse_transcript`
/// 那样建 `ReviewEntry` 列表,只回三个标量,给 Agent 卡片的实时刷新用
/// (每次 hook 事件都会重跑一次,故意做得比 `parse_transcript` 轻)。
/// 第三个标量 `activity` 是同一次扫描里顺带提取的"最后一句人类发言"
/// (Agent 卡片"当前工作内容"的兜底数据源,见 `workspace.rs::agent_card`
/// ——优先用 Todo 派发记录的任务标题,拿不到才落到这里),不新开一次
/// 读取:两者读的是同一份 transcript,分两次扫描纯属浪费 IO。`activity`
/// **只认人类发言,不认 AI 回复**(用户实测反馈:两者都认时,回合结束
/// 后"最后一行"常是 AI 的收尾总结,看不出这一轮到底在做什么——人类的
/// 原始请求比 AI 的总结更能说明"当前在干什么")。识别 Claude 形状
/// (`type:"user"`,`message.content` 是字符串)、CodeBuddy 形状
/// (`type:"message"`+`role:"user"`,`content[].type:"input_text"`)和
/// Codex 形状(`type:"response_item"`+`payload.type:"message"`+
/// `payload.role:"user"`)的
pub use dozer_panel_agent::transcript_tail::latest_model_mode_and_activity;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_entry_serializes_with_external_tag_shape() {
        let human = ReviewEntry::Human {
            text: "你好".into(),
        };
        assert_eq!(
            serde_json::to_string(&human).unwrap(),
            r#"{"Human":{"text":"你好"}}"#
        );

        let ai = ReviewEntry::AiTurn {
            text: "回复".into(),
            thinking_text: Some("先想想".into()),
            tool_calls: vec![ToolCallInfo {
                summary: "Edit README.md".into(),
                input_json: Some("{\"file_path\":\"README.md\"}".into()),
                id: None,
            }],
            tool_results: Vec::new(),
            tokens_in: 12,
            tokens_out: 34,
            tokens_cache_read: 0,
            tokens_cache_write: 0,
        };
        let json = serde_json::to_string(&ai).unwrap();
        assert!(json.starts_with(r#"{"AiTurn":"#));
        assert!(json.contains(r#""thinking_text":"#));
        assert!(json.contains(r#""Edit README.md""#));
        assert!(json.contains(r#""tokens_in":12"#));
        assert!(json.contains(r#""tokens_out":34"#));

        let tool = ReviewEntry::ToolResult {
            content: "boom".into(),
            is_error: true,
            call_id: None,
        };
        assert_eq!(
            serde_json::to_string(&tool).unwrap(),
            r#"{"ToolResult":{"content":"boom","is_error":true,"call_id":null}}"#
        );
    }

    #[test]
    fn review_entries_from_turns_maps_ai_turn_with_thinking_and_tool_calls() {
        use dozer_core::protocol::{ToolCallInfo, TurnRecord};
        let turns = vec![
            TurnRecord {
                turn_index: 0,
                role: "human".into(),
                content: "你好".into(),
                tool_calls: vec![],
                thinking: false,
                thinking_text: None,
                ts: None,
                is_error: false,
                ..Default::default()
            },
            TurnRecord {
                turn_index: 1,
                role: "ai".into(),
                content: "回复".into(),
                tool_calls: vec![ToolCallInfo {
                    summary: "Edit README.md".into(),
                    input_json: Some("{\"file_path\":\"README.md\"}".into()),
                    id: None,
                }],
                thinking: true,
                thinking_text: Some("先看看现有实现".into()),
                ts: None,
                is_error: false,
                tokens_in: 100,
                tokens_out: 50,
                tokens_cache_read: 5,
                tokens_cache_write: 2,
                tool_result_call_id: None,
            },
        ];
        let entries = review_entries_from_turns(&turns);
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0],
            ReviewEntry::Human {
                text: "你好".into()
            }
        );
        match &entries[1] {
            ReviewEntry::AiTurn {
                text,
                thinking_text,
                tool_calls,
                tool_results,
                tokens_in,
                tokens_out,
                tokens_cache_read,
                tokens_cache_write,
            } => {
                assert_eq!(text, "回复");
                assert_eq!(thinking_text.as_deref(), Some("先看看现有实现"));
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].summary, "Edit README.md");
                assert!(tool_results.is_empty());
                assert_eq!(*tokens_in, 100);
                assert_eq!(*tokens_out, 50);
                assert_eq!(*tokens_cache_read, 5);
                assert_eq!(*tokens_cache_write, 2);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn review_entries_from_turns_nests_consecutive_tool_results_under_preceding_ai_turn() {
        use dozer_core::protocol::TurnRecord;
        fn ai(content: &str) -> TurnRecord {
            TurnRecord {
                turn_index: 0,
                role: "ai".into(),
                content: content.into(),
                tool_calls: vec![],
                thinking: false,
                thinking_text: None,
                ts: None,
                is_error: false,
                ..Default::default()
            }
        }
        fn tool_result(content: &str, is_error: bool) -> TurnRecord {
            TurnRecord {
                turn_index: 0,
                role: "tool_result".into(),
                content: content.into(),
                tool_calls: vec![],
                thinking: false,
                thinking_text: None,
                ts: None,
                is_error,
                ..Default::default()
            }
        }
        let turns = vec![
            ai("第一轮"),
            tool_result("ok1", false),
            tool_result("boom", true),
            ai("第二轮"),
            tool_result("ok2", false),
        ];
        let entries = review_entries_from_turns(&turns);
        assert_eq!(entries.len(), 2);
        match &entries[0] {
            ReviewEntry::AiTurn {
                text, tool_results, ..
            } => {
                assert_eq!(text, "第一轮");
                assert_eq!(
                    tool_results,
                    &vec![
                        ToolResultEntry {
                            content: "ok1".into(),
                            is_error: false,
                            call_id: None
                        },
                        ToolResultEntry {
                            content: "boom".into(),
                            is_error: true,
                            call_id: None
                        },
                    ]
                );
            }
            other => panic!("{other:?}"),
        }
        match &entries[1] {
            ReviewEntry::AiTurn {
                text, tool_results, ..
            } => {
                assert_eq!(text, "第二轮");
                assert_eq!(
                    tool_results,
                    &vec![ToolResultEntry {
                        content: "ok2".into(),
                        is_error: false,
                        call_id: None
                    }]
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn review_entries_from_turns_merges_leading_trajectory_only_ai_turns_into_final_reply() {
        use dozer_core::protocol::{ToolCallInfo, TurnRecord};
        fn ai_step(summary: &str, tokens_in: u64, tokens_out: u64) -> TurnRecord {
            TurnRecord {
                turn_index: 0,
                role: "ai".into(),
                content: String::new(),
                tool_calls: vec![ToolCallInfo {
                    summary: summary.into(),
                    input_json: None,
                    id: None,
                }],
                thinking: false,
                thinking_text: None,
                ts: None,
                is_error: false,
                tokens_in,
                tokens_out,
                ..Default::default()
            }
        }
        fn ai_reply(content: &str, tokens_in: u64, tokens_out: u64) -> TurnRecord {
            TurnRecord {
                turn_index: 0,
                role: "ai".into(),
                content: content.into(),
                tool_calls: vec![],
                thinking: false,
                thinking_text: None,
                ts: None,
                is_error: false,
                tokens_in,
                tokens_out,
                ..Default::default()
            }
        }
        // 真实 transcript 里,一次逻辑回复常被拆成好几条 `ai` 行:前面几条
        // 只带 tool_use、没有正文,最后一条才是给人看的回复文本。这些"正文
        // 还没落地"的连续行应该合并成一个 `AiTurn`,而不是各自顶格显示。
        // token 用量(`tokens_in`/`tokens_out`)是每条 `ai` 行自己那一次 API
        // 调用的花费,折叠时应该逐项相加成这句回复的总花费,不是只取最后
        // 一条(那条往往文本很短、tokens_out 很低,单独看会严重低估成本)。
        let turns = vec![
            ai_step("git commit", 10, 5),
            ai_step("cargo build", 20, 8),
            ai_reply("Done.", 30, 3),
        ];
        let entries = review_entries_from_turns(&turns);
        assert_eq!(entries.len(), 1, "应合并成一个条目,而不是三个空气泡");
        match &entries[0] {
            ReviewEntry::AiTurn {
                text,
                tool_calls,
                tokens_in,
                tokens_out,
                ..
            } => {
                assert_eq!(text, "Done.");
                assert_eq!(tool_calls.len(), 2);
                assert_eq!(tool_calls[0].summary, "git commit");
                assert_eq!(tool_calls[1].summary, "cargo build");
                assert_eq!(*tokens_in, 60, "三条 ai 行的 tokens_in 应该相加:10+20+30");
                assert_eq!(*tokens_out, 16, "三条 ai 行的 tokens_out 应该相加:5+8+3");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn review_entries_from_turns_does_not_merge_across_a_finalized_ai_turn() {
        use dozer_core::protocol::TurnRecord;
        fn ai(content: &str) -> TurnRecord {
            TurnRecord {
                turn_index: 0,
                role: "ai".into(),
                content: content.into(),
                tool_calls: vec![],
                thinking: false,
                thinking_text: None,
                ts: None,
                is_error: false,
                ..Default::default()
            }
        }
        // 一旦某个 `AiTurn` 已经有正文(算定稿),后续 `ai` 行不该继续往
        // 它里面塞——即使它没有正文,也该另起一个新条目。
        let turns = vec![ai("先回一句"), ai("")];
        let entries = review_entries_from_turns(&turns);
        assert_eq!(entries.len(), 2);
        match &entries[0] {
            ReviewEntry::AiTurn { text, .. } => assert_eq!(text, "先回一句"),
            other => panic!("{other:?}"),
        }
        match &entries[1] {
            ReviewEntry::AiTurn { text, .. } => assert_eq!(text, ""),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn review_entries_from_turns_propagates_tool_result_call_id() {
        use dozer_core::protocol::{ToolCallInfo, TurnRecord};
        let turns = vec![
            TurnRecord {
                role: "ai".into(),
                content: "done".into(),
                tool_calls: vec![ToolCallInfo {
                    summary: "Edit README.md".into(),
                    input_json: None,
                    id: Some("toolu_1".into()),
                }],
                ..Default::default()
            },
            TurnRecord {
                role: "tool_result".into(),
                content: "ok".into(),
                tool_result_call_id: Some("toolu_1".into()),
                ..Default::default()
            },
        ];
        let entries = review_entries_from_turns(&turns);
        match &entries[0] {
            ReviewEntry::AiTurn {
                tool_calls,
                tool_results,
                ..
            } => {
                assert_eq!(tool_calls[0].id.as_deref(), Some("toolu_1"));
                assert_eq!(tool_results[0].call_id.as_deref(), Some("toolu_1"));
            }
            other => panic!("expected AiTurn, got {other:?}"),
        }
    }

    #[test]
    fn review_entries_from_turns_keeps_orphan_tool_result_at_top_level() {
        use dozer_core::protocol::TurnRecord;
        let turns = vec![TurnRecord {
            turn_index: 0,
            role: "tool_result".into(),
            content: "ok output".into(),
            tool_calls: vec![],
            thinking: false,
            thinking_text: None,
            ts: None,
            is_error: false,
            ..Default::default()
        }];
        let entries = review_entries_from_turns(&turns);
        assert_eq!(
            entries,
            vec![ReviewEntry::ToolResult {
                content: "ok output".into(),
                is_error: false,
                call_id: None,
            }]
        );
    }

    #[test]
    fn review_entries_from_turns_skips_token_usage_rows() {
        use dozer_core::protocol::TurnRecord;
        // Codex 的用量行(见 dozerd parse.rs::parse_codex_shaped_chunk)不是
        // 给人看的内容,只用于 dozerd 那边的 token 求和——审阅时间线必须
        // 跳过它,否则会插入一堆没有正文的空 AiTurn 气泡(role 既不是
        // "human" 也不是 "tool_result" 时,既有的兜底分支会把任何角色都
        // 当成 ai 文本处理)。
        let turns = vec![
            TurnRecord {
                turn_index: 0,
                role: "human".into(),
                content: "你好".into(),
                ..Default::default()
            },
            TurnRecord {
                turn_index: 1,
                role: "ai".into(),
                content: "回复".into(),
                ..Default::default()
            },
            TurnRecord {
                turn_index: 2,
                role: "token_usage".into(),
                content: String::new(),
                tokens_in: 100,
                tokens_out: 20,
                ..Default::default()
            },
        ];
        let entries = review_entries_from_turns(&turns);
        assert_eq!(
            entries.len(),
            2,
            "用量行不应该产生第三个条目(也不该被折叠进 AiTurn 覆盖掉正文)"
        );
        match &entries[1] {
            ReviewEntry::AiTurn { text, .. } => assert_eq!(text, "回复"),
            other => panic!("{other:?}"),
        }
    }
}
