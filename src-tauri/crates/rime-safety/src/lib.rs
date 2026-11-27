//! 写入安全：原子写、写前备份、外部修改检测、备份列表与清理
//! （PRD FR-6 / §6.3「写入安全协议」，IPC 契约 `docs/ipc.md` §2 / §5.1）。
//!
//! ## 写入安全协议（顺序不可变，这是产品的核心护栏）
//!
//! ```text
//! 1. 读当前文件 → 生成指纹（mtime + size + sha256）
//! 2. 与调用方 expected_fingerprint 比对
//!      不一致 → Err(ExternalModified)，一个字节都不写
//! 3. 暂存内容校验（UTF-8；Rime 文件一律 UTF-8，编码坏掉比拒绝写入更危险）
//! 4. 写前备份 `<原文件名>_<yyyyMMdd>_<HHmmss>.bak`
//!      失败 → 中止写入（Err(BackupFailed) / PermissionDenied / DiskFull）
//! 5. 临时文件（与目标**同目录**，跨文件系统 rename 不是原子的）写入 + fsync
//! 6. 提交前再次确认目标文件未被外部改写（关闭「读 → rename」的窗口）
//! 7. 原子 rename 覆盖目标 + 目录 fsync
//! 8. 返回新指纹（读自磁盘，前端必须以此刷新）
//! ```
//!
//! ## 失败后原文件无损
//!
//! 第 1–7 步任何一步失败都会让原文件字节保持原样：
//! 校验与备份在写入前完成，临时文件失败即删除，rename 本身是原子的
//! （Unix `rename(2)` / Windows `MoveFileEx(MOVEFILE_REPLACE_EXISTING)`）。
//!
//! ## 错误映射（`docs/ipc.md` §1.1）
//!
//! | 场景 | ErrorCode |
//! | --- | --- |
//! | 指纹不一致 / 提交前被外部改写 | `ExternalModified` |
//! | 备份无法创建（目录不可建、目标被占用等） | `BackupFailed` |
//! | 临时文件 / fsync / rename 失败 | `AtomicWriteFailed` |
//! | 磁盘满（含配额超限） | `DiskFull` |
//! | 权限不足 / 只读文件系统 | `PermissionDenied` |
//! | 目标文件不存在 | `FileNotFound` |
//! | 暂存内容不是合法 UTF-8 | `CorruptEncoding` |
//!
//! ## 备份命名（FR-6.1，与参考工具 rime-config-tool 兼容）
//!
//! `custom_phrase.txt` → `custom_phrase.txt_20260126_153045.bak`。
//! 同一秒内多次备份时追加 `_N`（`_1`、`_2`…），保证不覆盖、不丢备份；
//! 这是唯一偏离基础格式的情形，`list_backups` 同时兼容两种形态。
//!
//! ## 隐私
//!
//! 日志与错误 `detail` 只记录路径、字节数与哈希，**不记录词库正文**
//! （PRD §4「日志不记录词库正文」）。
//!
//! ## 用法
//!
//! ```
//! use rime_core::fingerprint_file;
//! use rime_safety::{save_file, WriteSafetyOptions};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let dir = tempfile::tempdir()?;
//! # let path = dir.path().join("custom_phrase.txt");
//! # std::fs::write(&path, "# header\n一个\tig\t100\n")?;
//! let expected = fingerprint_file(&path)?;
//! let outcome = save_file(
//!     &path,
//!     &expected,
//!     "# header\n一个\tig\t200\n".as_bytes(),
//!     &WriteSafetyOptions::default(),
//! )?;
//! assert_eq!(outcome.fingerprint.sha256.len(), 64);
//! assert!(outcome.backup.path.exists());
//! # Ok(())
//! # }
//! ```

mod atomic;
mod error;

pub mod backup;
mod save;

pub use backup::{
    backup_file, create_backup, list_backups, prune_backups, resolve_backup_dir, BackupOptions,
    PruneOutcome,
};
pub use save::{
    current_fingerprint, fingerprint_matches, save_file, save_parsed_dict, verify_expected,
    FingerprintPolicy, SaveOutcome, WriteSafetyOptions,
};

/// crate 版本（诊断信息用）。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
