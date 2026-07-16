use std::collections::VecDeque;

/// 每会话滚屏缓冲容量：1 MiB（一期定值，规格 §4 终端数据面）
pub const SCROLLBACK_CAP: usize = 1 << 20;

/// 追加写、按容量自动逐出头部的字节环。offset 为单调递增的"历史总写入量"坐标系。
pub struct RingBuffer {
    cap: usize,
    buf: VecDeque<u8>,
    total: u64,
}

impl RingBuffer {
    pub fn new(cap: usize) -> Self {
        Self { cap, buf: VecDeque::with_capacity(cap.min(64 * 1024)), total: 0 }
    }

    pub fn push(&mut self, data: &[u8]) -> u64 {
        self.buf.extend(data.iter().copied());
        while self.buf.len() > self.cap {
            self.buf.pop_front();
        }
        self.total += data.len() as u64;
        self.total
    }

    pub fn total_written(&self) -> u64 {
        self.total
    }

    /// 窗口起点的 offset（第一个仍在缓冲内的字节的历史坐标）
    fn window_start(&self) -> u64 {
        self.total - self.buf.len() as u64
    }

    pub fn snapshot(&self) -> (Vec<u8>, u64) {
        (self.buf.iter().copied().collect(), self.total)
    }

    pub fn read_from(&self, offset: u64) -> Option<Vec<u8>> {
        if offset > self.total {
            return None;
        }
        if offset < self.window_start() {
            return None; // 已被逐出，调用方应退回全量 snapshot
        }
        let skip = (offset - self.window_start()) as usize;
        Some(self.buf.iter().skip(skip).copied().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_accumulates_and_snapshot_returns_all_when_under_cap() {
        let mut r = RingBuffer::new(16);
        assert_eq!(r.push(b"hello"), 5);
        assert_eq!(r.push(b" world"), 11);
        let (data, next) = r.snapshot();
        assert_eq!(data, b"hello world");
        assert_eq!(next, 11);
        assert_eq!(r.total_written(), 11);
    }

    #[test]
    fn eviction_keeps_only_last_cap_bytes() {
        let mut r = RingBuffer::new(8);
        r.push(b"0123456789"); // 10 bytes into cap 8
        let (data, next) = r.snapshot();
        assert_eq!(data, b"23456789");
        assert_eq!(next, 10);
    }

    #[test]
    fn read_from_returns_tail_or_none_when_evicted() {
        let mut r = RingBuffer::new(8);
        r.push(b"0123456789");
        assert_eq!(r.read_from(6).unwrap(), b"6789");
        assert_eq!(r.read_from(10).unwrap(), b"" as &[u8]);
        assert!(r.read_from(1).is_none()); // offset 1 已被挤出（窗口起点=2）
    }

    #[test]
    fn oversized_push_keeps_last_cap_bytes() {
        let mut r = RingBuffer::new(4);
        r.push(b"abcdefgh");
        let (data, next) = r.snapshot();
        assert_eq!(data, b"efgh");
        assert_eq!(next, 8);
    }
}
