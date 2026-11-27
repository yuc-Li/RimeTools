//! 原子写：同目录临时文件 → write → fsync → rename。
//!
//! ## 为什么必须同目录
//!
//! `rename(2)` 只在**同一文件系统**内是原子的。把临时文件放到 `/tmp`
//! （往往是 tmpfs）再 rename 到用户目录会退化成「复制 + 删除」，
//! 崩溃/断电时可能留下半截文件。因此临时文件固定创建在目标文件的父目录。
//!
//! ## 两段式
//!
//! [`StagedFile::stage`] 只负责把内容落盘并 fsync，不碰目标文件；
//! 调用方可以在提交前再做一次外部修改检测，最后 [`StagedFile::commit`] 原子替换。
//! `StagedFile` 在 drop 时清理未提交的临时文件，任何失败都不会残留垃圾。

use crate::error::{map_io_error, Stage};
use rime_core::types::{AppError, AppResult, ErrorCode};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 已落盘（fsync 完成）但尚未替换目标的临时文件。
pub(crate) struct StagedFile {
    path: PathBuf,
    target: PathBuf,
    dir: PathBuf,
    committed: bool,
}

impl StagedFile {
    /// 在 `target` 所在目录创建临时文件，写入 `bytes` 并 fsync。
    ///
    /// `perms` 用于保留原文件权限位（避免改写后 0600 变 0644 之类的意外放宽）。
    pub(crate) fn stage(
        target: &Path,
        bytes: &[u8],
        perms: Option<fs::Permissions>,
    ) -> AppResult<Self> {
        let dir = target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::AtomicWriteFailed,
                    "目标文件没有父目录，无法原子写入",
                )
                .with_detail(target.display().to_string())
            })?;
        let file_name = target.file_name().ok_or_else(|| {
            AppError::new(
                ErrorCode::AtomicWriteFailed,
                "目标路径不含文件名，无法原子写入",
            )
            .with_detail(target.display().to_string())
        })?;

        let (path, mut file) = create_temp_file(dir, file_name)?;
        // 从这一刻起，任何提前返回都会由 Drop 清理临时文件
        let staged = Self {
            path,
            target: target.to_path_buf(),
            dir: dir.to_path_buf(),
            committed: false,
        };

        file.write_all(bytes)
            .map_err(|e| map_io_error(Stage::TempWrite, &staged.path, &e))?;
        // sync_all = fsync（含元数据）。不 fsync 就 rename，崩溃后可能
        // 得到「名字变了但内容还是旧的/空的」文件。
        file.sync_all()
            .map_err(|e| map_io_error(Stage::TempSync, &staged.path, &e))?;
        if let Some(perms) = perms {
            fs::set_permissions(&staged.path, perms)
                .map_err(|e| map_io_error(Stage::TempCreate, &staged.path, &e))?;
        }
        drop(file);
        Ok(staged)
    }

    /// 临时文件路径（仅测试与诊断用）。
    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// 原子替换目标文件。成功后目录 fsync 尽力而为（失败不影响写入结果）。
    pub(crate) fn commit(mut self) -> AppResult<()> {
        fs::rename(&self.path, &self.target)
            .map_err(|e| map_io_error(Stage::Rename, &self.target, &e))?;
        self.committed = true;
        // rename 已生效；目录 fsync 让「改名」本身也持久化，失败仅告警。
        if let Err(e) = sync_dir(&self.dir) {
            tracing::warn!(
                dir = %self.dir.display(),
                error = %e,
                "目录 fsync 失败（rename 已生效，内容已写入）"
            );
        }
        Ok(())
    }
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        if !self.committed {
            // 尽力清理；失败只可能因为没有权限，此时也不该掩盖原始错误
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// 创建独占临时文件（`create_new` 保证不会误碰已有文件）。
fn create_temp_file(dir: &Path, file_name: &OsStr) -> AppResult<(PathBuf, File)> {
    for _ in 0..32 {
        let seq = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut name = OsString::from(".");
        name.push(file_name);
        name.push(format!(".rime-safety-{}-{seq}.tmp", std::process::id()));
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(map_io_error(Stage::TempCreate, &path, &e)),
        }
    }
    Err(AppError::new(
        ErrorCode::AtomicWriteFailed,
        "无法创建临时文件（同名临时文件过多）",
    )
    .with_detail(dir.display().to_string())
    .with_hint("请检查目录中是否有残留的 .rime-safety-*.tmp 文件并清理后重试。"))
}

/// Unix 上 fsync 目录；其他平台没有等价操作，直接成功。
#[cfg(unix)]
pub(crate) fn sync_dir(dir: &Path) -> std::io::Result<()> {
    File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
pub(crate) fn sync_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rime_core::types::ErrorCode;

    fn no_temp_left(dir: &Path) {
        for entry in fs::read_dir(dir).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert!(
                !name.contains(".rime-safety-") && !name.ends_with(".tmp"),
                "临时文件未清理：{name}"
            );
        }
    }

    #[test]
    fn temp_file_is_created_in_target_directory() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("xhup_dicts");
        fs::create_dir(&sub).unwrap();
        let target = sub.join("custom_phrase.txt");

        let staged = StagedFile::stage(&target, b"data", None).unwrap();
        assert_eq!(staged.path().parent().unwrap(), sub.as_path());
        assert!(staged.path().exists());
        drop(staged);
        no_temp_left(&sub);
    }

    #[test]
    fn commit_replaces_target_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        fs::write(&target, b"old").unwrap();

        StagedFile::stage(&target, b"new", None)
            .unwrap()
            .commit()
            .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        no_temp_left(dir.path());
    }

    /// 失败注入：目标在提交前被替换成目录 → rename 失败。
    /// 必须返回 `AtomicWriteFailed`、保留原目标、清理临时文件。
    #[test]
    fn rename_failure_maps_to_atomic_write_failed_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.txt");
        // rename 一个普通文件到一个目录上必然失败（EISDIR / EACCES / ENOTDIR）
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep"), b"keep").unwrap();

        let staged = StagedFile::stage(&target, b"new content", None).unwrap();
        let staging_path = staged.path().to_path_buf();
        let err = staged.commit().unwrap_err();

        assert_eq!(
            err.code,
            ErrorCode::AtomicWriteFailed,
            "detail={:?}",
            err.detail
        );
        assert!(!staging_path.exists(), "失败后必须清理临时文件");
        assert!(target.is_dir(), "目标不应被破坏");
        assert_eq!(fs::read(target.join("keep")).unwrap(), b"keep");
    }

    #[cfg(unix)]
    #[test]
    fn stage_preserves_requested_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("secret.txt");
        fs::write(&target, b"old").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();

        let perms = fs::metadata(&target).unwrap().permissions();
        StagedFile::stage(&target, b"new", Some(perms))
            .unwrap()
            .commit()
            .unwrap();
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
