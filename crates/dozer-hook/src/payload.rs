use serde::{Deserialize, Serialize};

/// dozer-hook 发往 dozerd 的事件信封（P1f 扩展 data 语义）
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct HookPayload {
    pub event: String,
    pub ts_ms: u64,
    pub data: serde_json::Value,
}

impl HookPayload {
    pub fn new(event: &str, data: serde_json::Value) -> Self {
        let ts_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock before epoch")
            .as_millis() as u64;
        Self {
            event: event.to_string(),
            ts_ms,
            data,
        }
    }

    /// 单行 JSON（JSON Lines 协议帧）
    pub fn to_json_line(&self) -> String {
        let mut s = serde_json::to_string(self).expect("payload serializes");
        s.push('\n');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_line_is_single_line_and_roundtrips() {
        let p = HookPayload::new("turn_completed", serde_json::json!({"session": "s1"}));
        let line = p.to_json_line();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        let back: HookPayload = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(back, p);
    }
}
