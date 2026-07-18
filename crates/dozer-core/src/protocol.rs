use serde::{Deserialize, Serialize, de::DeserializeOwned};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub command: String,
    pub cwd: String,
    pub alive: bool,
    pub created_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    ListSessions,
    CreateSession {
        name: String,
        command: String,
        args: Vec<String>,
        cwd: String,
        cols: u16,
        rows: u16,
    },
    Attach {
        session_id: String,
        from_offset: u64,
    },
    Write {
        session_id: String,
        data_b64: String,
    },
    Resize {
        session_id: String,
        cols: u16,
        rows: u16,
    },
    Kill {
        session_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Sessions {
        sessions: Vec<SessionInfo>,
    },
    Created {
        session: SessionInfo,
    },
    Attached {
        session_id: String,
        snapshot_b64: String,
        next_offset: u64,
    },
    Output {
        session_id: String,
        data_b64: String,
        offset: u64,
    },
    Exited {
        session_id: String,
        code: Option<i32>,
    },
    Ok,
    Error {
        message: String,
    },
}

pub fn encode_line<T: Serialize>(value: &T) -> String {
    let mut s = serde_json::to_string(value).expect("protocol types always serialize");
    s.push('\n');
    s
}

pub fn decode_line<T: DeserializeOwned>(line: &str) -> anyhow::Result<T> {
    Ok(serde_json::from_str(line)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrips_as_single_json_line() {
        let req = Request::CreateSession {
            name: "主线".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "cat".into()],
            cwd: "/tmp".into(),
            cols: 80,
            rows: 24,
        };
        let line = encode_line(&req);
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        let back: Request = decode_line(line.trim()).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn reply_tag_is_snake_case() {
        let line = encode_line(&Reply::Attached {
            session_id: "s1".into(),
            snapshot_b64: "aGk=".into(),
            next_offset: 2,
        });
        assert!(line.contains(r#""type":"attached""#));
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(decode_line::<Request>("not json").is_err());
    }
}
