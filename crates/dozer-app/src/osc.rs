//! OSC 7（cwd）/ OSC 133（命令边界+退出码）观察式扫描器。
//!
//! 有状态：序列可跨 `feed` 调用截断续接。只观察不剥离——alacritty
//! 静默消费未知 OSC，透传不影响渲染。单序列超过 `OSC_CAP` 即放弃
//! （防恶意/损坏流撑爆内存），后续序列不受影响。

use std::path::PathBuf;

/// 单条 OSC 序列净荷上限（`]` 与终止符之间的字节数）。
const OSC_CAP: usize = 2048;

#[derive(Debug, PartialEq)]
pub enum OscEvent {
    /// OSC 7：`file://host/path`（path 已 percent-decode）。
    Cwd(PathBuf),
    /// OSC 133;C：命令开始执行。
    CmdStart,
    /// OSC 133;D;<code>：命令结束，附退出码（缺省 0）。
    CmdExit(i32),
}

enum State {
    Ground,
    Esc,
    Osc,
    /// OSC 内遇到 ESC：期待 `\`（ST 终止符）。
    OscEsc,
}

pub struct OscScanner {
    state: State,
    buf: Vec<u8>,
}

impl OscScanner {
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            buf: Vec::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<OscEvent> {
        let mut out = Vec::new();
        for &b in bytes {
            self.state = match self.state {
                State::Ground => {
                    if b == 0x1b {
                        State::Esc
                    } else {
                        State::Ground
                    }
                }
                State::Esc => {
                    if b == b']' {
                        self.buf.clear();
                        State::Osc
                    } else {
                        State::Ground
                    }
                }
                State::Osc => match b {
                    0x07 => {
                        self.finish(&mut out);
                        State::Ground
                    }
                    0x1b => State::OscEsc,
                    _ => {
                        if self.buf.len() >= OSC_CAP {
                            self.buf.clear();
                            State::Ground // 超长：放弃本序列
                        } else {
                            self.buf.push(b);
                            State::Osc
                        }
                    }
                },
                State::OscEsc => {
                    if b == b'\\' {
                        self.finish(&mut out);
                    }
                    State::Ground
                }
            };
        }
        out
    }

    fn finish(&mut self, out: &mut Vec<OscEvent>) {
        let s = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf.clear();
        if let Some(rest) = s.strip_prefix("7;") {
            // host 段到第一个 '/' 为止；余下是路径
            if let Some(url) = rest.strip_prefix("file://")
                && let Some(slash) = url.find('/')
                && let Some(path) = crate::assets::percent_decode(&url[slash..])
            {
                out.push(OscEvent::Cwd(PathBuf::from(path)));
            }
        } else if let Some(rest) = s.strip_prefix("133;") {
            match rest.as_bytes().first() {
                Some(b'C') => out.push(OscEvent::CmdStart),
                Some(b'D') => {
                    let code = rest
                        .get(2..)
                        .and_then(|c| c.parse::<i32>().ok())
                        .unwrap_or(0);
                    out.push(OscEvent::CmdExit(code));
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc7_bel_and_st_terminators() {
        let mut s = OscScanner::new();
        let ev = s.feed(b"\x1b]7;file://mac.local/Users/c/proj\x07");
        assert_eq!(ev, vec![OscEvent::Cwd("/Users/c/proj".into())]);
        let ev = s.feed(b"\x1b]7;file://mac.local/tmp/a%20b\x1b\\");
        assert_eq!(ev, vec![OscEvent::Cwd("/tmp/a b".into())]);
    }

    #[test]
    fn osc133_command_marks() {
        let mut s = OscScanner::new();
        let ev = s.feed(b"\x1b]133;C\x07ls output\x1b]133;D;0\x07\x1b]133;D;127\x07");
        assert_eq!(
            ev,
            vec![
                OscEvent::CmdStart,
                OscEvent::CmdExit(0),
                OscEvent::CmdExit(127)
            ]
        );
    }

    #[test]
    fn sequence_split_across_feeds_resumes() {
        let mut s = OscScanner::new();
        assert!(s.feed(b"\x1b]7;file://h/Us").is_empty());
        let ev = s.feed(b"ers/c\x07");
        assert_eq!(ev, vec![OscEvent::Cwd("/Users/c".into())]);
    }

    #[test]
    fn oversize_sequence_abandoned() {
        let mut s = OscScanner::new();
        let mut big = b"\x1b]7;file://h/".to_vec();
        big.extend(std::iter::repeat_n(b'x', 4096));
        big.extend(b"\x07\x1b]133;C\x07");
        let ev = s.feed(&big);
        assert_eq!(ev, vec![OscEvent::CmdStart], "超长序列放弃，后续序列不受损");
    }

    #[test]
    fn cjk_and_other_escapes_produce_nothing() {
        let mut s = OscScanner::new();
        assert!(
            s.feed("你好\x1b[31m红\x1b[0m\x1b]0;title\x07".as_bytes())
                .is_empty()
        );
    }
}
