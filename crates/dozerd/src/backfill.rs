//! dozerd 启动时的历史 transcript 回填。

use crate::transcripts::TranscriptStore;
use dozer_core::protocol::AgentKind;
use std::path::PathBuf;

/// 对一批 `(agent, 文件路径)` 逐个 `ingest_session`,单个文件失败只记警告
/// 跳过,不中断其它文件——`backfill_all`(daemon 启动全量回填)和
/// `TranscriptStore::backfill_project`(单项目按需回填,Task 4)共用这同一个
/// 循环,只是喂给它的 `files` 来源不同(前者 `discover_all_transcript_files`,
/// 后者 `discover_project_transcript_files`)。返回成功摄取的文件数。
pub fn ingest_files(store: &TranscriptStore, files: Vec<(AgentKind, PathBuf)>) -> u32 {
    let mut imported = 0u32;
    for (agent, path) in files {
        match store.ingest_session(agent, &path) {
            Ok(()) => imported += 1,
            Err(e) => {
                tracing::warn!(error = %e, path = %path.display(), "回填摄取失败,跳过该文件");
            }
        }
    }
    imported
}

pub fn backfill_all(store: &TranscriptStore, files: Vec<(AgentKind, PathBuf)>) {
    ingest_files(store, files);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backfill_all_ingests_every_discovered_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let f1 = tmp.path().join("a.jsonl");
        let f2 = tmp.path().join("b.jsonl");
        std::fs::write(&f1, "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"一\"}}\n").unwrap();
        std::fs::write(&f2, "{\"type\":\"user\",\"uuid\":\"u2\",\"message\":{\"role\":\"user\",\"content\":\"二\"}}\n").unwrap();

        backfill_all(
            &store,
            vec![(AgentKind::Claude, f1), (AgentKind::Claude, f2)],
        );

        assert_eq!(store.get_conversation_turns("a", -1, 10).unwrap().len(), 1);
        assert_eq!(store.get_conversation_turns("b", -1, 10).unwrap().len(), 1);
    }

    #[test]
    fn backfill_all_skips_unreadable_file_without_panicking() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        backfill_all(
            &store,
            vec![(AgentKind::Claude, tmp.path().join("does-not-exist.jsonl"))],
        );
        // 不 panic 即通过;没有对应数据可断言。
    }

    #[test]
    fn ingest_files_returns_count_of_successfully_ingested_files() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let f1 = tmp.path().join("a.jsonl");
        std::fs::write(&f1, "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"一\"}}\n").unwrap();
        let missing = tmp.path().join("does-not-exist.jsonl");

        let n = ingest_files(
            &store,
            vec![(AgentKind::Claude, f1), (AgentKind::Claude, missing)],
        );
        assert_eq!(n, 1);
    }
}
