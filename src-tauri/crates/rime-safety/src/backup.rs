//! 写前备份、备份列表与按保留份数清理（PRD FR-6.1 / FR-6.3 / FR-6.4）。
//!
//! ## 命名（与参考工具 rime-config-tool 兼容）
//!
//! `原文件名_yyyyMMdd_HHmmss.bak`，例如：
//!
//! - `custom_phrase_double.txt_20260126_153045.bak`
//! - `xhup.user.dict.yaml_20260126_153045.bak`
//!
//! 同一秒内对同一文件多次备份时，为避免覆盖（=丢数据）追加序号：
//! `custom_phrase.txt_20260126_153045_1.bak`。列表与清理同时识别两种形态，
//! 排序键为 `(yyyyMMddHHmmss, 序号)`，因此同秒备份仍能正确区分新旧。

use crate::error::{map_io_error, Stage};
use chrono::{Local, NaiveDate, TimeZone};
use rime_core::types::{AppError, AppResult, BackupReason, BackupRef, ErrorCode};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// 备份选项。
#[derive(Debug, Clone)]
pub struct BackupOptions {
    /// 备份目录；`None` = 与源文件同目录（参考工具行为）。
    pub dir: Option<PathBuf>,
    /// 备份原因，写入 `BackupRef.reason`。
    pub reason: BackupReason,
}

impl Default for BackupOptions {
    fn default() -> Self {
        Self {
            dir: None,
            reason: BackupReason::BeforeWrite,
        }
    }
}

impl BackupOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// 指定备份目录（不存在会自动创建）。
    pub fn in_dir(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: Some(dir.into()),
            reason: BackupReason::BeforeWrite,
        }
    }

    pub fn for_reason(mut self, reason: BackupReason) -> Self {
        self.reason = reason;
        self
    }
}

/// 清理结果。
#[derive(Debug, Clone)]
pub struct PruneOutcome {
    /// 被删除的备份文件路径（按从旧到新的顺序）。
    pub deleted: Vec<PathBuf>,
    /// 清理后保留的份数。
    pub kept: usize,
}

/// 备份目录解析：未配置时与源文件同目录。
pub fn resolve_backup_dir(source: &Path, configured: Option<&Path>) -> PathBuf {
    match configured {
        Some(dir) => dir.to_path_buf(),
        None => source
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(".")),
    }
}

/// 用**调用方给定的内容**创建写前备份。
///
/// 之所以显式传入内容而不是自己重读：写入安全协议里备份必须严格等于
/// 「刚刚通过指纹校验的那份字节」，重读会引入新的时间窗口。
pub fn create_backup(
    source: &Path,
    contents: &[u8],
    options: &BackupOptions,
) -> AppResult<BackupRef> {
    let file_name = source.file_name().ok_or_else(|| {
        AppError::new(ErrorCode::Internal, "源文件路径缺少文件名，无法创建备份")
            .with_detail(source.display().to_string())
    })?;
    // 契约要求路径为合法 UTF-8（docs/ipc.md §1），非 UTF-8 文件名无法保证命名兼容
    let file_name = file_name.to_str().ok_or_else(|| {
        AppError::new(
            ErrorCode::Internal,
            "源文件名不是合法 UTF-8，已拒绝创建备份",
        )
        .with_detail(source.display().to_string())
    })?;

    let dir = resolve_backup_dir(source, options.dir.as_deref());
    if options.dir.is_some() {
        // 仅对用户显式配置的目录做创建；默认目录（源文件所在目录）必须已存在
        fs::create_dir_all(&dir).map_err(|e| map_io_error(Stage::BackupCreate, &dir, &e))?;
    }

    let now = Local::now();
    let stamp = now.format("%Y%m%d_%H%M%S").to_string();
    let mut seq = 0u32;
    loop {
        let name = if seq == 0 {
            format!("{file_name}_{stamp}.bak")
        } else {
            format!("{file_name}_{stamp}_{seq}.bak")
        };
        let path = dir.join(&name);
        match write_new_file(&path, contents) {
            Ok(()) => {
                tracing::info!(
                    path = %path.display(),
                    bytes = contents.len(),
                    "写前备份已创建"
                );
                return Ok(BackupRef {
                    id: name,
                    path,
                    source_path: source.to_path_buf(),
                    created_at: now.timestamp_millis(),
                    size: contents.len() as u64,
                    reason: options.reason,
                });
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                seq += 1;
                if seq > 999 {
                    return Err(AppError::new(
                        ErrorCode::BackupFailed,
                        "同一秒内备份请求过多，已中止写入；原文件未被修改",
                    )
                    .with_detail(dir.display().to_string())
                    .with_hint("请重试；如需保留更多备份，请在设置中调整备份保留份数。"));
                }
            }
            Err(e) => return Err(map_io_error(Stage::BackupCreate, &path, &e)),
        }
    }
}

/// 从磁盘读取源文件后备份（手动「立即备份」/ 定期备份入口）。
pub fn backup_file(source: &Path, options: &BackupOptions) -> AppResult<BackupRef> {
    let contents = fs::read(source).map_err(|e| map_io_error(Stage::Read, source, &e))?;
    create_backup(source, &contents, options)
}

/// 列出某个源文件的全部备份，**新 → 旧**排序。
///
/// 目录不存在（还没备份过 / 已被清理）返回空列表，不视为错误。
pub fn list_backups(source: &Path, dir: Option<&Path>) -> AppResult<Vec<BackupRef>> {
    let source_name = source.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
        AppError::new(
            ErrorCode::Internal,
            "源文件路径缺少合法 UTF-8 文件名，无法匹配备份",
        )
        .with_detail(source.display().to_string())
    })?;
    let scan_dir = resolve_backup_dir(source, dir);

    let entries = match fs::read_dir(&scan_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(map_io_error(Stage::BackupList, &scan_dir, &e)),
    };

    let mut found: Vec<(BackupStamp, BackupRef)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| map_io_error(Stage::BackupList, &scan_dir, &e))?;
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            // 非 UTF-8 文件名不可能是本工具创建的备份，跳过
            continue;
        };
        let Some(stamp) = parse_backup_name(name, source_name) else {
            continue;
        };
        // 扫描期间被删除 / 权限不足的条目直接跳过，不因单个坏条目让整个列表失败
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let created_at = stamp
            .to_unix_millis()
            .or_else(|| mtime_millis(&metadata))
            .unwrap_or(0);
        found.push((
            stamp,
            BackupRef {
                id: name.to_owned(),
                path: entry.path(),
                source_path: source.to_path_buf(),
                created_at,
                size: metadata.len(),
                // 备份文件本身不携带原因，磁盘上的备份默认视为写前备份
                reason: BackupReason::BeforeWrite,
            },
        ));
    }

    found.sort_by_key(|(stamp, _)| std::cmp::Reverse(*stamp));
    Ok(found.into_iter().map(|(_, backup)| backup).collect())
}

/// 只保留最新 `keep_count` 份备份，删除更旧的（FR-6.4）。
///
/// `keep_count = 0` 会删除全部备份（显式调用才可能发生）。
pub fn prune_backups(
    source: &Path,
    dir: Option<&Path>,
    keep_count: u32,
) -> AppResult<PruneOutcome> {
    let all = list_backups(source, dir)?;
    let keep = keep_count as usize;
    if all.len() <= keep {
        return Ok(PruneOutcome {
            deleted: Vec::new(),
            kept: all.len(),
        });
    }

    let mut deleted = Vec::new();
    for backup in all.into_iter().skip(keep) {
        match fs::remove_file(&backup.path) {
            Ok(()) => deleted.push(backup.path),
            // 已被其他进程删掉：目标已达成，不算失败
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(map_io_error(Stage::BackupList, &backup.path, &e)),
        }
    }
    tracing::info!(deleted = deleted.len(), kept = keep, "已清理旧备份");
    Ok(PruneOutcome {
        deleted,
        kept: keep,
    })
}

// ───────────────────────── 内部实现 ─────────────────────────

/// 备份名解析结果，排序键为 `(yyyyMMddHHmmss, 同秒序号)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct BackupStamp {
    datetime_key: u64,
    seq: u32,
}

impl BackupStamp {
    /// 把文件名里的本地时间还原为 Unix 毫秒（供前端展示）。
    fn to_unix_millis(self) -> Option<i64> {
        let key = format!("{:014}", self.datetime_key);
        let year: i32 = key[0..4].parse().ok()?;
        let month: u32 = key[4..6].parse().ok()?;
        let day: u32 = key[6..8].parse().ok()?;
        let hour: u32 = key[8..10].parse().ok()?;
        let minute: u32 = key[10..12].parse().ok()?;
        let second: u32 = key[12..14].parse().ok()?;
        let naive = NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(hour, minute, second)?;
        // 本地时间在夏令时跳变处可能不存在/有歧义，取最早解释，失败则回退文件 mtime
        Local
            .from_local_datetime(&naive)
            .earliest()
            .map(|dt| dt.timestamp_millis())
    }
}

/// 解析 `原文件名_yyyyMMdd_HHmmss[_N].bak`；不匹配返回 `None`。
fn parse_backup_name(name: &str, source_name: &str) -> Option<BackupStamp> {
    let body = name.strip_suffix(".bak")?;
    let rest = body.strip_prefix(source_name)?.strip_prefix('_')?;
    let (datetime, seq_part) = split_timestamp(rest)?;

    let seq = match seq_part {
        None => 0,
        Some(digits) => digits.parse().ok()?,
    };
    // yyyyMMdd + HHmmss → 14 位可比较整数（无需时区处理）
    let datetime_key: u64 = format!("{}{}", &datetime[..8], &datetime[9..])
        .parse()
        .ok()?;
    Some(BackupStamp { datetime_key, seq })
}

/// 切出 `yyyyMMdd_HHmmss` 与可选的 `_N` 序号。
fn split_timestamp(rest: &str) -> Option<(&str, Option<&str>)> {
    let bytes = rest.as_bytes();
    if bytes.len() < 15 {
        return None;
    }
    if !bytes[..8].iter().all(u8::is_ascii_digit)
        || bytes[8] != b'_'
        || !bytes[9..15].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let datetime = &rest[..15];
    let tail = &rest[15..];
    if tail.is_empty() {
        return Some((datetime, None));
    }
    let seq = tail.strip_prefix('_')?;
    if seq.is_empty() || !seq.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((datetime, Some(seq)))
}

/// 独占创建（`create_new`）并 fsync，失败时清理半截文件。
///
/// 不用 `fs::copy`：`copy` 会截断/覆盖同名文件，违背「绝不丢备份」。
fn write_new_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let result = file.write_all(contents).and_then(|()| file.sync_all());
    drop(file);
    if let Err(e) = result {
        let _ = fs::remove_file(path);
        return Err(e);
    }
    if let Some(dir) = path.parent() {
        // 备份文件名的持久化尽量保证；失败不影响本次备份可用性
        let _ = crate::atomic::sync_dir(dir);
    }
    Ok(())
}

fn mtime_millis(metadata: &fs::Metadata) -> Option<i64> {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_reference_tool_compatible_names() {
        let stamp = parse_backup_name("custom_phrase.txt_20260126_153045.bak", "custom_phrase.txt")
            .expect("应解析成功");
        assert_eq!(stamp.datetime_key, 20260126153045);
        assert_eq!(stamp.seq, 0);

        let stamped = parse_backup_name(
            "xhup.user.dict.yaml_20260126_153045_7.bak",
            "xhup.user.dict.yaml",
        )
        .expect("同秒序号应解析成功");
        assert_eq!(stamped.datetime_key, 20260126153045);
        assert_eq!(stamped.seq, 7);
    }

    #[test]
    fn rejects_foreign_names() {
        for (name, source) in [
            ("custom_phrase.txt_20260126.bak", "custom_phrase.txt"),
            ("custom_phrase.txt_20260126_153045.txt", "custom_phrase.txt"),
            ("other.txt_20260126_153045.bak", "custom_phrase.txt"),
            ("custom_phrase.txt_20260126_153045.bak", "other.txt"),
            ("custom_phrase.txt_2026012_153045.bak", "custom_phrase.txt"),
            (
                "custom_phrase.txt_20260126_153045_abc.bak",
                "custom_phrase.txt",
            ),
            ("custom_phrase.txt.bak", "custom_phrase.txt"),
        ] {
            assert!(
                parse_backup_name(name, source).is_none(),
                "不应把 {name} 当作 {source} 的备份"
            );
        }
    }

    #[test]
    fn same_second_backups_do_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("custom_phrase.txt");
        fs::write(&source, b"v0").unwrap();

        let opts = BackupOptions::default();
        let mut names = Vec::new();
        for i in 1..=5 {
            let backup = create_backup(&source, format!("v{i}").as_bytes(), &opts).unwrap();
            names.push(backup.id);
        }
        // 唯一性：同一秒内不能互相覆盖
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "备份名重复：{names:?}");

        for name in &names {
            assert!(
                parse_backup_name(name, "custom_phrase.txt").is_some(),
                "命名不合规：{name}"
            );
            assert!(name.ends_with(".bak"));
        }
    }

    #[test]
    fn prune_keeps_newest_and_reports_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("custom_phrase.txt");
        fs::write(&source, b"v0").unwrap();

        let opts = BackupOptions::default();
        for i in 1..=4 {
            fs::write(&source, format!("v{i}")).unwrap();
            create_backup(&source, format!("v{i}").as_bytes(), &opts).unwrap();
        }

        let outcome = prune_backups(&source, None, 2).unwrap();
        assert_eq!(outcome.deleted.len(), 2);
        assert_eq!(outcome.kept, 2);

        let remaining = list_backups(&source, None).unwrap();
        assert_eq!(remaining.len(), 2);
        assert_eq!(
            fs::read(&remaining[0].path).unwrap(),
            b"v4",
            "最新备份必须保留"
        );
        assert_eq!(fs::read(&remaining[1].path).unwrap(), b"v3");
    }

    #[test]
    fn list_is_newest_first_even_within_same_second() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("a.txt");
        fs::write(&source, b"x").unwrap();
        let opts = BackupOptions::default();
        for content in [b"v1".as_slice(), b"v2".as_slice(), b"v3".as_slice()] {
            create_backup(&source, content, &opts).unwrap();
        }
        let list = list_backups(&source, None).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(fs::read(&list[0].path).unwrap(), b"v3");
        assert_eq!(fs::read(&list[2].path).unwrap(), b"v1");
    }

    #[test]
    fn list_of_missing_directory_is_empty_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("a.txt");
        fs::write(&source, b"x").unwrap();
        let missing = dir.path().join("no-such-backup-dir");
        assert!(list_backups(&source, Some(&missing)).unwrap().is_empty());
    }
}
