//! 有界 JSONL 批次。只提交完整记录，超长单行不整行无限分配。
use anyhow::Result;
use std::io::Read;

pub(super) const IO_CHUNK: usize = 1024 * 1024;
pub(super) const MAX_BATCH: usize = 8 * 1024 * 1024;
const MAX_RECORDS: usize = 500;

pub(super) fn read_batch(reader: &mut impl Read, available: u64) -> Result<String> {
    let mut limited = reader.take(available.min((MAX_BATCH + 1) as u64));
    let mut buffer = Vec::new();
    let mut block = vec![0; IO_CHUNK];
    let mut complete = 0;
    let mut records = 0;
    loop {
        let n = limited.read(&mut block)?;
        if n == 0 {
            break;
        }
        let base = buffer.len();
        buffer.extend_from_slice(&block[..n]);
        for (index, byte) in block[..n].iter().enumerate() {
            if *byte == b'\n' && base + index < MAX_BATCH {
                complete = base + index + 1;
                records += 1;
                if records == MAX_RECORDS {
                    buffer.truncate(complete);
                    return Ok(String::from_utf8(buffer)?);
                }
            }
        }
        if buffer.len() > MAX_BATCH {
            anyhow::ensure!(complete > 0, "单条 transcript 记录超过 8MiB 上限");
            break;
        }
    }
    buffer.truncate(complete);
    Ok(String::from_utf8(buffer)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_records_and_partial_tail() {
        let text = "{}\n".repeat(600) + "partial";
        let mut bytes = text.as_bytes();
        assert_eq!(
            read_batch(&mut bytes, text.len() as u64).unwrap(),
            "{}\n".repeat(500)
        );
        assert_eq!(read_batch(&mut &b"{}\npartial"[..], 10).unwrap(), "{}\n");
    }
    #[test]
    fn accepts_record_across_io_chunks_but_rejects_oversized_line() {
        let text = "x".repeat(2 * IO_CHUNK) + "\n";
        assert_eq!(
            read_batch(&mut text.as_bytes(), text.len() as u64).unwrap(),
            text
        );
        let long = vec![b'x'; MAX_BATCH + 1];
        assert!(read_batch(&mut long.as_slice(), long.len() as u64).is_err());
        assert!(read_batch(&mut &b"half"[..], 4).unwrap().is_empty());
    }
}
