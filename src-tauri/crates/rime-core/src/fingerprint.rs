//! 文件指纹：用于「外部修改检测」（写入安全协议的一环）。
//!
//! 契约见 `docs/ipc.md` 的 `Fingerprint`：mtime + size + sha256 三元组。

use crate::types::Fingerprint;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

#[derive(Debug, thiserror::Error)]
pub enum FingerprintError {
    #[error("读取文件失败：{path}：{source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("文件修改时间早于 UNIX 纪元，无法生成指纹")]
    InvalidMtime,
}

/// 对内存字节计算 sha256 十六进制串。
pub fn fingerprint_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    // 手写 hex，避免额外依赖
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// 对文件生成指纹。
pub fn fingerprint_file(path: &Path) -> Result<Fingerprint, FingerprintError> {
    let meta = fs::metadata(path).map_err(|e| FingerprintError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let bytes = fs::read(path).map_err(|e| FingerprintError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let mtime = meta
        .modified()
        .map_err(|_| FingerprintError::InvalidMtime)?
        .duration_since(UNIX_EPOCH)
        .map_err(|_| FingerprintError::InvalidMtime)?;
    Ok(Fingerprint {
        mtime_ms: mtime.as_millis() as i64,
        size: meta.len(),
        sha256: fingerprint_bytes(&bytes),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_of_known_input() {
        // echo -n "" | sha256sum
        assert_eq!(
            fingerprint_bytes(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // echo -n "abc" | sha256sum
        assert_eq!(
            fingerprint_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn detects_content_change() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        fs::write(&p, b"hello").unwrap();
        let f1 = fingerprint_file(&p).unwrap();
        fs::write(&p, b"hello!").unwrap();
        let f2 = fingerprint_file(&p).unwrap();
        assert_ne!(f1.sha256, f2.sha256);
        assert_ne!(f1.size, f2.size);
    }
}
