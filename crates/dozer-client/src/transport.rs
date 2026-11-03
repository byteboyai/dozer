//! Shared bounded line transport.
use super::*;
/// 有状态的有界帧读取器。未完成帧保存在 `pending`,select 取消后仍能续读。
pub(super) struct FrameReader<R> {
    pub(super) reader: R,
    pub(super) pending: Vec<u8>,
}

impl<R: tokio::io::AsyncBufRead + Unpin> FrameReader<R> {
    pub(super) fn new(reader: R) -> Self {
        Self {
            reader,
            pending: Vec::new(),
        }
    }

    pub(super) async fn next_frame(&mut self, max: usize) -> std::io::Result<Option<String>> {
        loop {
            // We append only through the first newline in fill_buf, so a
            // complete frame always ends at pending.last(). Avoid quadratic
            // rescanning when a large frame arrives one byte at a time.
            if self.pending.last() == Some(&b'\n') {
                let mut line = std::mem::take(&mut self.pending);
                line.pop();
                return String::from_utf8(line)
                    .map(Some)
                    .map_err(|_| protocol_frame_error());
            }
            let available = self.reader.fill_buf().await?;
            if available.is_empty() {
                if self.pending.is_empty() {
                    return Ok(None);
                }
                let line = std::mem::take(&mut self.pending);
                return String::from_utf8(line)
                    .map(Some)
                    .map_err(|_| protocol_frame_error());
            }
            let consumed = if let Some(newline) = available.iter().position(|byte| *byte == b'\n') {
                if self.pending.len() + newline > max {
                    return Err(protocol_frame_error());
                }
                self.pending.extend_from_slice(&available[..=newline]);
                newline + 1
            } else {
                if self.pending.len() + available.len() > max {
                    return Err(protocol_frame_error());
                }
                self.pending.extend_from_slice(available);
                available.len()
            };
            self.reader.consume(consumed);
        }
    }
}

pub(super) fn protocol_frame_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "协议帧超过上限或不是 UTF-8",
    )
}

pub(super) fn encode_request_checked(req: &Request) -> Result<String> {
    dozer_core::protocol::encode_line_bounded(req, MAX_CLIENT_FRAME_BYTES)
}
