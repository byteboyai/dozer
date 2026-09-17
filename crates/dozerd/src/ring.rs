use std::collections::VecDeque;

/// 每会话滚屏缓冲容量：1 MiB（一期定值，规格 §4 终端数据面）
pub const SCROLLBACK_CAP: usize = 1 << 20;

/// 追加写、按容量自动逐出头部的字节环。offset 为单调递增的"历史总写入量"坐标系。
pub struct RingBuffer {
    cap: usize,
    buf: VecDeque<Box<[u8]>>,
    /// 当前缓冲内的字节总数(= 各 chunk 长度之和),等价于旧 `buf.len()`。
    len: usize,
    total: u64,
}

impl RingBuffer {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            buf: VecDeque::new(),
            len: 0,
            total: 0,
        }
    }

    pub fn push(&mut self, data: &[u8]) -> u64 {
        if !data.is_empty() {
            self.buf.push_back(data.to_vec().into_boxed_slice());
            self.len += data.len();
        }
        while self.len > self.cap {
            let overflow = self.len - self.cap;
            let front = self.buf.pop_front().expect("len > cap 时 buf 不可能为空");
            if front.len() <= overflow {
                // 整个 chunk 都在待逐出的范围内，整块丢弃。
                self.len -= front.len();
            } else {
                // 只有 chunk 前面 `overflow` 字节是该逐出的，裁掉前缀、
                // 保留尾部塞回队首；裁剪后 len 精确落在 cap 上，循环即退出。
                self.len -= overflow;
                self.buf
                    .push_front(front[overflow..].to_vec().into_boxed_slice());
            }
        }
        self.total += data.len() as u64;
        self.total
    }

    pub fn total_written(&self) -> u64 {
        self.total
    }

    /// 窗口起点的 offset（第一个仍在缓冲内的字节的历史坐标）
    fn window_start(&self) -> u64 {
        self.total - self.len as u64
    }

    pub fn snapshot(&self) -> (Vec<u8>, u64) {
        let mut out = Vec::with_capacity(self.len);
        for chunk in &self.buf {
            out.extend_from_slice(chunk);
        }
        (out, self.total)
    }

    pub fn read_from(&self, offset: u64) -> Option<Vec<u8>> {
        if offset > self.total {
            return None;
        }
        if offset < self.window_start() {
            return None; // 已被逐出，调用方应退回全量 snapshot
        }
        let skip = (offset - self.window_start()) as usize;
        let mut out = Vec::with_capacity(self.len - skip);
        let mut remaining = skip;
        for chunk in &self.buf {
            if remaining >= chunk.len() {
                remaining -= chunk.len();
                continue;
            }
            out.extend_from_slice(&chunk[remaining..]);
            remaining = 0;
        }
        Some(out)
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

    #[test]
    fn chunk_boundaries_preserve_byte_order_and_read_from() {
        let mut r = RingBuffer::new(16);
        r.push(b"abcd"); // chunk1: 4 字节
        r.push(b"efghijkl"); // chunk2: 8 字节
        r.push(b"mn"); // chunk3: 2 字节
        let (data, next) = r.snapshot();
        assert_eq!(data, b"abcdefghijklmn");
        assert_eq!(next, 14);
        // 跨 chunk 边界取尾部:从 chunk2 中间一路读到 chunk3 结尾。
        assert_eq!(r.read_from(6).unwrap(), b"ghijklmn");
        // 落在 chunk2/chunk3 接缝处。
        assert_eq!(r.read_from(12).unwrap(), b"mn");
        // 越界(> total)仍 None。
        assert!(r.read_from(15).is_none());
    }

    #[test]
    fn eviction_partially_trims_a_stale_chunk() {
        let mut r = RingBuffer::new(10);
        r.push(b"abcde"); // chunk1: 5 字节，len=5
        r.push(b"fghij"); // chunk2: 5 字节，len=10，未触发逐出
        r.push(b"klm"); // len=13，溢出 3；最老的 chunk1(5 字节)比溢出量大，
        // 应只裁掉 chunk1 的前 3 字节("abc")，保留"de"塞回队首，
        // 而不是把 chunk1 整块丢掉。
        let (data, next) = r.snapshot();
        assert_eq!(data, b"defghijklm");
        assert_eq!(next, 13);
        // 裁剪后的边界（原 chunk1 与 chunk2 的接缝挪到了"de"|"fghijklm"之间）
        // 也要能正确跨界读取。
        assert_eq!(r.read_from(5).unwrap(), b"fghijklm");
    }
}
