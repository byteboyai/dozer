//! 文本保存(文件预览重构 Phase B Task 4):同目录临时文件 + rename 原子替换,
//! 并保持原文件的 BOM / 换行约定。
//!
//! 编辑器(CodeMirror)内部正文统一用 `\n` 分隔、且 `fetch(...).text()` 会
//! 丢掉 UTF-8 BOM。保存时按**磁盘原件**的约定复原:原文件有 BOM 就补回,
//! 原来以 CRLF 为主就转回 CRLF。

use std::io::Write;
use std::path::Path;

/// 从原文件字节采样出的文本约定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileTextFormat {
    pub bom: bool,
    pub crlf: bool,
}

/// 检测原文件的 BOM / 换行约定(只扫前 64KiB,足够;大文件也不会因此变慢)。
pub fn detect_format(original: &[u8]) -> FileTextFormat {
    let bom = original.starts_with(&[0xEF, 0xBB, 0xBF]);
    let sample = &original[..original.len().min(64 * 1024)];
    let mut crlf = 0usize;
    let mut lf_only = 0usize;
    let mut i = 0;
    while i < sample.len() {
        if sample[i] == b'\r' && sample.get(i + 1) == Some(&b'\n') {
            crlf += 1;
            i += 2;
            continue;
        }
        if sample[i] == b'\n' {
            lf_only += 1;
        }
        i += 1;
    }
    FileTextFormat {
        bom,
        crlf: crlf > lf_only,
    }
}

/// 把编辑器正文(以 `\n` 分隔)按 `fmt` 编码成待写盘字节。
pub fn encode_for_save(text: &str, fmt: FileTextFormat) -> Vec<u8> {
    // 先归一化任何残留 CRLF,避免重复转换;再按需转回 CRLF。
    let normalized = text.replace("\r\n", "\n");
    let body = if fmt.crlf {
        normalized.replace('\n', "\r\n")
    } else {
        normalized
    };
    let mut out = Vec::with_capacity(body.len() + 3);
    if fmt.bom {
        out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    out.extend_from_slice(body.as_bytes());
    out
}

/// 原子保存:读取原文件约定 → 编码 → 写同目录临时文件 → flush → rename 覆盖。
/// 任何一步失败原文件保持不变。
pub fn save_text_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let fmt = std::fs::read(path)
        .map(|bytes| detect_format(&bytes))
        .unwrap_or_default();
    let bytes = encode_for_save(text, fmt);

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "dozer-save".to_string());
    let mut tmp = None;
    let mut write_result = Err(std::io::Error::other("无法创建保存临时文件"));
    for attempt in 0..16u32 {
        let candidate = dir.join(format!(
            ".{file_name}.dozer-tmp-{}-{attempt}",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut file) => {
                tmp = Some(candidate);
                write_result = (|| -> std::io::Result<()> {
                    file.write_all(&bytes)?;
                    file.flush()?;
                    file.sync_all()?;
                    Ok(())
                })();
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                write_result = Err(error);
                break;
            }
        }
    }
    let tmp = tmp.unwrap_or_else(|| {
        dir.join(format!(
            ".{file_name}.dozer-tmp-{}-exhausted",
            std::process::id()
        ))
    });
    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn detects_bom_and_crlf() {
        assert_eq!(
            detect_format(b"\xEF\xBB\xBFa\r\nb\r\n"),
            FileTextFormat {
                bom: true,
                crlf: true
            }
        );
        assert_eq!(
            detect_format(b"a\nb\n"),
            FileTextFormat {
                bom: false,
                crlf: false
            }
        );
    }

    #[test]
    fn mixed_endings_prefer_majority() {
        // 2 个 CRLF vs 1 个 LF → CRLF。
        assert!(detect_format(b"a\r\nb\r\nc\n").crlf);
        // 2 个 LF vs 1 个 CRLF → LF。
        assert!(!detect_format(b"a\nb\nc\r\n").crlf);
    }

    #[test]
    fn encode_round_trips_bom_and_crlf() {
        let fm = FileTextFormat {
            bom: true,
            crlf: true,
        };
        let bytes = encode_for_save("a\nb\n", fm);
        assert_eq!(bytes, b"\xEF\xBB\xBFa\r\nb\r\n");
        // 已是 CRLF 的输入不会变成 CRCRLF。
        let bytes = encode_for_save("a\r\nb\r\n", fm);
        assert_eq!(bytes, b"\xEF\xBB\xBFa\r\nb\r\n");
    }

    #[test]
    fn encode_plain_lf() {
        assert_eq!(
            encode_for_save("x\ny", FileTextFormat::default()),
            b"x\ny".to_vec()
        );
    }

    #[test]
    fn atomic_save_preserves_original_format_and_replaces_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"\xEF\xBB\xBFfn main() {}\r\n").unwrap();
        }
        save_text_atomic(&path, "fn main() {\n    let x = 1;\n}\n").unwrap();
        let saved = std::fs::read(&path).unwrap();
        assert!(saved.starts_with(&[0xEF, 0xBB, 0xBF]), "BOM 保留");
        let text = String::from_utf8(saved).unwrap();
        assert!(text.contains("let x = 1;"));
        assert!(!text.contains("\n\n"), "换行应为 CRLF");
        assert!(text.matches("\r\n").count() >= 3);
        // 临时文件不残留。
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("dozer-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "临时文件应已 rename 走");
    }

    #[test]
    fn atomic_save_creates_when_missing_with_lf() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.txt");
        save_text_atomic(&path, "hello\n").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello\n");
    }
}
