//! 假 webview 表面:记录 host 通过 `evaluate_script` 推过来的每条 envelope,并能模拟"销毁/重建"。

#[derive(Default)]
pub(crate) struct FakeSurface {
    attached: bool,
    received: Vec<String>,
}

impl FakeSurface {
    pub(crate) fn attach(&mut self) {
        self.attached = true;
    }
    pub(crate) fn detach(&mut self) {
        self.attached = false;
    }
    pub(crate) fn is_attached(&self) -> bool {
        self.attached
    }
    pub(crate) fn receive(&mut self, envelope_json: String) {
        self.received.push(envelope_json);
    }
    /// 记录一条 `dispatch_script` 注入脚本:从中还原出 envelope JSON(去 JS 包装)。
    pub(crate) fn receive_script(&mut self, script: String) {
        // `dispatch_script` 形状:`...dispatch("<json>"...);`——取出第一个 JS
        // 字符串字面量并反转义回 JSON。
        let open = script.find('"').expect("dispatch 脚本含字符串字面量") + 1;
        let rest = &script[open..];
        let close = rest.rfind('"').expect("字符串字面量有收尾引号");
        let literal = &rest[..close];
        let json = serde_json::from_str::<String>(&format!("\"{literal}\""))
            .expect("dispatch 里的 envelope 是合法 JS 字符串");
        self.received.push(json);
    }
    pub(crate) fn count(&self) -> usize {
        self.received.len()
    }
    pub(crate) fn kinds(&self) -> Vec<String> {
        self.received
            .iter()
            .map(|json| {
                let v: serde_json::Value = serde_json::from_str(json).unwrap();
                v["payload"]["kind"].as_str().unwrap().to_string()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::usage::{UsageViewPayload, encode_usage_push};

    #[test]
    fn records_envelopes_and_reports_their_payload_kind() {
        let mut s = FakeSurface::default();
        assert!(!s.is_attached());
        s.attach();
        assert!(s.is_attached());
        s.receive(encode_usage_push(UsageViewPayload::Empty));
        assert_eq!(s.count(), 1);
        assert_eq!(s.kinds(), vec!["empty".to_string()]);
        s.detach();
        assert!(!s.is_attached());
        assert_eq!(s.count(), 1, "detach 不清空已收到的记录");
    }
}
