//! IO 错误 → 契约 `ErrorCode` 的统一映射。
//!
//! 映射策略：先看**跨平台可判定的语义错误**（磁盘满 / 无权限 / 只读文件系统），
//! 再看**失败阶段**决定非特异性错误的归属：备份阶段 → `BackupFailed`，
//! 临时文件 / fsync / rename → `AtomicWriteFailed`。

use rime_core::types::{AppError, ErrorCode};
use std::io;
use std::path::Path;

/// 失败阶段。只用于选择兜底 `ErrorCode`，不对外暴露。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    /// 读取目标文件 / 生成指纹。
    Read,
    /// 创建备份目录或备份文件。
    BackupCreate,
    /// 扫描 / 清理备份。
    BackupList,
    /// 创建同目录临时文件。
    TempCreate,
    /// 写临时文件内容。
    TempWrite,
    /// fsync 临时文件。
    TempSync,
    /// rename 覆盖目标。
    Rename,
}

/// 把 `std::io::Error` 映射为可直接返回前端的 `AppError`。
///
/// 所有消息都遵守「原因 + 下一步动作」的可用性要求（PRD §4），
/// 且明确告知「原文件未被修改」，避免用户恐慌。
pub(crate) fn map_io_error(stage: Stage, path: &Path, err: &io::Error) -> AppError {
    if is_disk_full(err) {
        return AppError::new(
            ErrorCode::DiskFull,
            "磁盘空间不足，写入已中止；原文件未被修改",
        )
        .with_detail(format!("{}：{err}", path.display()))
        .with_hint("请清理磁盘空间后重试。本次写入未生效，原文件保持原样。");
    }
    if is_permission_denied(err) || is_read_only_fs(err) {
        return AppError::new(
            ErrorCode::PermissionDenied,
            "没有写入权限，写入已中止；原文件未被修改",
        )
        .with_detail(format!("{}：{err}", path.display()))
        .with_hint("请检查文件或目录的权限（只读目录 / 只读文件系统），或改用可写目录后重试。");
    }

    match stage {
        Stage::Read => map_read_error(path, err),
        Stage::BackupCreate => backup_error(path, err),
        Stage::BackupList => AppError::new(ErrorCode::BackupFailed, "备份目录不可用")
            .with_detail(format!("{}：{err}", path.display()))
            .with_hint("请检查备份目录是否存在且可写；可在设置中关闭自定义备份目录改用默认位置。"),
        Stage::TempCreate | Stage::TempWrite | Stage::TempSync | Stage::Rename => {
            atomic_write_error(path, err)
        }
    }
}

/// 读取阶段的映射：文件不存在要说清楚，避免用户以为是工具坏了。
fn map_read_error(path: &Path, err: &io::Error) -> AppError {
    match err.kind() {
        io::ErrorKind::NotFound => AppError::new(
            ErrorCode::FileNotFound,
            format!("文件不存在，可能已被移动或删除：{}", path.display()),
        )
        .with_detail(err.to_string())
        .with_hint("请在设置中检查用户目录，重新扫描受管文件后再试。"),
        _ => AppError::new(
            ErrorCode::FileNotWritable,
            format!("无法读取文件：{}", path.display()),
        )
        .with_detail(err.to_string())
        .with_hint("请确认路径指向普通文件且当前用户可读。"),
    }
}

fn backup_error(path: &Path, err: &io::Error) -> AppError {
    AppError::new(
        ErrorCode::BackupFailed,
        "写前备份失败，已中止写入；原文件未被修改",
    )
    .with_detail(format!("{}：{err}", path.display()))
    .with_hint("请检查备份目录是否可写、同目录空间是否充足，然后重试。")
}

fn atomic_write_error(path: &Path, err: &io::Error) -> AppError {
    AppError::new(ErrorCode::AtomicWriteFailed, "原子写入失败，原文件未被修改")
        .with_detail(format!("{}：{err}", path.display()))
        .with_hint("请检查目标目录是否可写、磁盘是否可用；写前备份已保留，可先恢复备份再重试。")
}

/// 磁盘满：Unix `ENOSPC` / `EDQUOT`，Windows `ERROR_DISK_FULL` /
/// `ERROR_HANDLE_DISK_FULL` / `ERROR_DISK_QUOTA_EXCEEDED`。
///
/// 不用 `io::ErrorKind::StorageFull`，因为它是 1.83 才稳定的变体，
/// 而 workspace MSRV 是 1.77。
fn is_disk_full(err: &io::Error) -> bool {
    match err.raw_os_error() {
        #[cfg(unix)]
        Some(28) | Some(122) => true, // ENOSPC / EDQUOT
        #[cfg(windows)]
        Some(39) | Some(112) | Some(1295) => true, // 磁盘满 / 句柄磁盘满 / 配额超限
        _ => false,
    }
}

/// 只读文件系统：Unix `EROFS`，Windows `ERROR_WRITE_PROTECT`。
fn is_read_only_fs(err: &io::Error) -> bool {
    match err.raw_os_error() {
        #[cfg(unix)]
        Some(30) => true, // EROFS
        #[cfg(windows)]
        Some(19) => true, // ERROR_WRITE_PROTECT
        _ => false,
    }
}

/// `ErrorKind::PermissionDenied` 覆盖 EACCES / EPERM（Unix）与
/// `ERROR_ACCESS_DENIED`（Windows）。
fn is_permission_denied(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::PermissionDenied
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path() -> &'static Path {
        Path::new("/tmp/rime-safety-test/custom_phrase.txt")
    }

    fn io_err(code: i32) -> io::Error {
        io::Error::from_raw_os_error(code)
    }

    #[cfg(unix)]
    #[test]
    fn enospc_is_disk_full_at_every_stage() {
        for stage in [
            Stage::Read,
            Stage::BackupCreate,
            Stage::TempCreate,
            Stage::TempWrite,
            Stage::TempSync,
            Stage::Rename,
        ] {
            let err = map_io_error(stage, path(), &io_err(28));
            assert_eq!(err.code, ErrorCode::DiskFull, "stage={stage:?}");
            assert!(
                err.message.contains("原文件未被修改"),
                "message={}",
                err.message
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn enospc_at_fsync_is_still_disk_full() {
        // fsync 才暴露 ENOSPC 是真实场景（延迟分配文件系统）
        assert_eq!(
            map_io_error(Stage::TempSync, path(), &io_err(28)).code,
            ErrorCode::DiskFull
        );
    }

    #[cfg(unix)]
    #[test]
    fn eacces_maps_to_permission_denied() {
        for stage in [
            Stage::BackupCreate,
            Stage::TempCreate,
            Stage::TempWrite,
            Stage::TempSync,
            Stage::Rename,
        ] {
            assert_eq!(
                map_io_error(stage, path(), &io_err(13)).code, // EACCES
                ErrorCode::PermissionDenied,
                "stage={stage:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn erofs_maps_to_permission_denied() {
        assert_eq!(
            map_io_error(Stage::TempCreate, path(), &io_err(30)).code, // EROFS
            ErrorCode::PermissionDenied
        );
    }

    #[cfg(unix)]
    #[test]
    fn edquot_maps_to_disk_full() {
        assert_eq!(
            map_io_error(Stage::TempWrite, path(), &io_err(122)).code,
            ErrorCode::DiskFull
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_disk_full_codes() {
        for code in [39, 112, 1295] {
            assert_eq!(
                map_io_error(Stage::TempWrite, path(), &io_err(code)).code,
                ErrorCode::DiskFull,
                "code={code}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_access_denied_maps_to_permission_denied() {
        assert_eq!(
            map_io_error(Stage::TempCreate, path(), &io_err(5)).code,
            ErrorCode::PermissionDenied
        );
    }

    #[test]
    fn backup_stage_fallback_is_backup_failed() {
        let e = io::Error::other("backup dir vanished");
        for stage in [Stage::BackupCreate, Stage::BackupList] {
            assert_eq!(
                map_io_error(stage, path(), &e).code,
                ErrorCode::BackupFailed,
                "stage={stage:?}"
            );
        }
    }

    #[test]
    fn atomic_stage_fallback_is_atomic_write_failed() {
        let e = io::Error::other("rename race");
        for stage in [
            Stage::TempCreate,
            Stage::TempWrite,
            Stage::TempSync,
            Stage::Rename,
        ] {
            assert_eq!(
                map_io_error(stage, path(), &e).code,
                ErrorCode::AtomicWriteFailed,
                "stage={stage:?}"
            );
        }
    }

    #[test]
    fn missing_file_maps_to_file_not_found() {
        let e = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(
            map_io_error(Stage::Read, path(), &e).code,
            ErrorCode::FileNotFound
        );
    }
}
