//! dozerd 启动时的历史 transcript 回填。

use crate::transcripts::TranscriptStore;
use dozer_core::protocol::AgentKind;
use std::path::PathBuf;

pub fn backfill_all(store: &TranscriptStore, files: Vec<(AgentKind, PathBuf)>) {
    for (agent, path) in files {
        if let Err(e) = store.ingest_session(agent, &path) {
            tracing::warn!(error = %e, path = %path.display(), "启动回填摄取失败,跳过该文件");
        }
    }
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
}
