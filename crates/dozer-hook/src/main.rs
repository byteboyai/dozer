mod payload;
use payload::HookPayload;
use std::io::Read;

fn main() {
    // 用法：dozer-hook <event-name>；stdin 为 agent hooks 传入的 JSON（可为空）
    let event = std::env::args().nth(1).unwrap_or_else(|| "unknown".into());
    let mut stdin = String::new();
    let _ = std::io::stdin().read_to_string(&mut stdin);
    let data = serde_json::from_str(&stdin).unwrap_or(serde_json::Value::Null);
    // P1f 前先打到 stdout；P1f 改为写 dozerd 的 UDS
    print!("{}", HookPayload::new(&event, data).to_json_line());
}
