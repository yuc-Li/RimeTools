//! 写入安全协议入口（PRD §6.3，IPC 契约 `docs/ipc.md` §5.1）。
//!
//! 顺序不可变：
//!
//! ```text
//! 读当前文件 → 指纹比对 → 暂存校验 → 写前备份
//!            → 同目录临时文件 + fsync → 提交前再校验 → 原子 rename → 新指纹
//! ```

use crate::atomic::StagedFile;
use crate::backup::{self, BackupOptions};
use crate::error::{map_io_error, Stage};
use rime_core::types::{AppError, AppResult, BackupReason, BackupRef, ErrorCode, Fingerprint};
use rime_core::{fingerprint_bytes, fingerprint_file, ParsedDict, WriteOptions};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// 指纹比对策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FingerprintPolicy {
    /// mtime + size + sha256 全等（默认，最保守）。
    ///
    /// 文件被 `touch`、同步工具改了时钟、内容被替换都会命中，报 `ExternalModified`。
    #[default]
    Strict,
    /// 只比内容（size + sha256），忽略 mtime 漂移。
    ///
    /// 适用于会保留内容但重写 mtime 的同步工具；内容不一致仍然拒绝写入。
    ContentOnly,
}

/// 写入选项。
#[derive(Debug, Clone)]
pub struct WriteSafetyOptions {
    /// 备份目录；`None` = 与源文件同目录（FR-6.1 参考工具兼容）。
    pub backup_dir: Option<PathBuf>,
    /// 指纹比对策略。
    pub fingerprint_policy: FingerprintPolicy,
    /// 是否校验暂存内容是合法 UTF-8（Rime 文件一律 UTF-8）。
    pub validate_staged_utf8: bool,
    /// 写入成功后清理旧备份的保留份数；`None` = 不自动清理。
    pub prune_keep_count: Option<u32>,
}

impl Default for WriteSafetyOptions {
    fn default() -> Self {
        Self {
            backup_dir: None,
            fingerprint_policy: FingerprintPolicy::Strict,
            validate_staged_utf8: true,
            prune_keep_count: None,
        }
    }
}

impl WriteSafetyOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// 指定备份目录。
    pub fn with_backup_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.backup_dir = Some(dir.into());
        self
    }

    /// 写入成功后按保留份数清理旧备份。
    pub fn with_prune_keep_count(mut self, keep_count: u32) -> Self {
        self.prune_keep_count = Some(keep_count);
        self
    }
}

/// 一次成功写入的结果。
#[derive(Debug, Clone)]
pub struct SaveOutcome {
    /// 实际写入字节数。
    pub bytes_written: u64,
    /// 写前备份（写入协议强制，必定存在）。
    pub backup: BackupRef,
    /// 写入后的新指纹（读自磁盘；前端必须以此刷新，禁止乐观更新）。
    pub fingerprint: Fingerprint,
}

/// 按写入安全协议保存文件内容。
///
/// - `source`：目标文件（若是指向文件的符号链接，会写入其真实目标，不替换链接）。
/// - `expected`：调用方打开文件时拿到的指纹；不一致即拒绝写入。
/// - `staged`：已经序列化好的完整文件内容。
///
/// # 保证
///
/// 返回 `Err` 时目标文件**一个字节都没有被修改**（磁盘满、权限不足、
/// 备份失败、外部修改、rename 失败均如此）。
pub fn save_file(
    source: &Path,
    expected: &Fingerprint,
    staged: &[u8],
    options: &WriteSafetyOptions,
) -> AppResult<SaveOutcome> {
    // 0) 符号链接要写到真实目标，否则会把链接本身替换成普通文件
    let target = resolve_write_target(source)?;

    // 1) 读当前文件并生成指纹（只读，不产生任何副作用）
    let (current, current_bytes) = read_current(&target)?;

    // 2) 外部修改检测：不一致立即中止，绝不覆盖
    ensure_matches(&target, &current, expected, options.fingerprint_policy)?;

    // 3) 暂存内容校验（编码坏掉比拒绝写入更危险）
    validate_staged(staged, options.validate_staged_utf8)?;

    // 4) 写前备份（强制；失败即中止）
    let backup_options = BackupOptions {
        dir: options.backup_dir.clone(),
        reason: BackupReason::BeforeWrite,
    };
    let backup = backup::create_backup(source, &current_bytes, &backup_options)?;

    // 5) 同目录临时文件写入 + fsync（rename 跨文件系统不是原子的）
    let perms = fs::metadata(&target)
        .ok()
        .map(|metadata| metadata.permissions());
    let staged_file = StagedFile::stage(&target, staged, perms)?;

    // 6) 提交前再确认一次：读文件与 rename 之间若被外部改写，放弃提交并保留备份
    let (before_commit, _) = read_current(&target)?;
    if !content_matches(&before_commit, &current) {
        return Err(
            external_modified_error(&target, &before_commit, &current).with_detail(format!(
                "提交前检测到文件已被外部改写，临时文件已丢弃；写前备份已保留：{}",
                backup.path.display()
            )),
        );
    }

    // 7) 原子 rename 覆盖
    staged_file.commit()?;

    // 8) 新指纹（读自磁盘，保证与前端看到的一致）
    let fingerprint = fingerprint_after_write(&target, staged);

    // 备份保留策略：写入已成功，清理失败只告警，不改变写入结果
    if let Some(keep_count) = options.prune_keep_count {
        match backup::prune_backups(source, options.backup_dir.as_deref(), keep_count) {
            Ok(outcome) if !outcome.deleted.is_empty() => {
                tracing::info!(
                    deleted = outcome.deleted.len(),
                    kept = outcome.kept,
                    "写入后已清理旧备份"
                )
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(code = ?e.code, "写入成功，但清理旧备份失败（不影响本次写入）")
            }
        }
    }

    tracing::info!(path = %target.display(), bytes = staged.len(), "原子写入完成");
    Ok(SaveOutcome {
        bytes_written: staged.len() as u64,
        backup,
        fingerprint,
    })
}

/// 计算文件当前指纹（打开文件时用；也是 `dict://external-change` 的数据源）。
pub fn current_fingerprint(source: &Path) -> AppResult<Fingerprint> {
    let target = resolve_write_target(source)?;
    Ok(read_current(&target)?.0)
}

/// 便捷入口：先用 `rime-core` 的行模型序列化，再走写入安全协议。
///
/// 序列化失败（列配置非法 / 空字段）会在**任何写入之前**返回
/// `FileFormatInvalid`，不会产生备份或临时文件——这正是协议里的
/// 「暂存校验」环节，绝不把半成品写到用户词库上。
pub fn save_parsed_dict(
    source: &Path,
    expected: &Fingerprint,
    dict: &ParsedDict,
    write_options: &WriteOptions,
    options: &WriteSafetyOptions,
) -> AppResult<SaveOutcome> {
    let staged = dict.serialize(write_options).map_err(|e| {
        AppError::new(
            ErrorCode::FileFormatInvalid,
            "词库序列化失败，未写入任何内容",
        )
        .with_detail(e.to_string())
        .with_hint("请检查文件格式与写入列配置（必须包含文本与编码列）。")
    })?;
    save_file(source, expected, staged.as_bytes(), options)
}

/// 只做外部修改检测，不写入。返回 `Err(ExternalModified)` 表示需要提示用户。
pub fn verify_expected(
    source: &Path,
    expected: &Fingerprint,
    policy: FingerprintPolicy,
) -> AppResult<()> {
    let target = resolve_write_target(source)?;
    let (current, _) = read_current(&target)?;
    ensure_matches(&target, &current, expected, policy)
}

/// 按策略比较两个指纹；`expected` 为打开时快照，`current` 为磁盘现状。
pub fn fingerprint_matches(
    expected: &Fingerprint,
    current: &Fingerprint,
    policy: FingerprintPolicy,
) -> bool {
    match policy {
        FingerprintPolicy::Strict => expected == current,
        FingerprintPolicy::ContentOnly => content_matches(expected, current),
    }
}

// ───────────────────────── 内部实现 ─────────────────────────

/// 内容级一致（size + sha256）；mtime 漂移不算内容变化。
fn content_matches(a: &Fingerprint, b: &Fingerprint) -> bool {
    a.size == b.size && a.sha256 == b.sha256
}

fn ensure_matches(
    path: &Path,
    current: &Fingerprint,
    expected: &Fingerprint,
    policy: FingerprintPolicy,
) -> AppResult<()> {
    if fingerprint_matches(expected, current, policy) {
        return Ok(());
    }
    Err(external_modified_error(path, current, expected))
}

/// `ExternalModified` 的标准错误：消息与提示面向用户，detail 只含哈希与时间。
fn external_modified_error(path: &Path, current: &Fingerprint, expected: &Fingerprint) -> AppError {
    AppError::new(
        ErrorCode::ExternalModified,
        format!(
            "文件已被外部修改，写入已取消；原文件未被修改：{}",
            path.display()
        ),
    )
    .with_detail(format!(
        "期望 mtime={}ms size={} sha256={}；实际 mtime={}ms size={} sha256={}",
        expected.mtime_ms,
        expected.size,
        expected.sha256,
        current.mtime_ms,
        current.size,
        current.sha256
    ))
    .with_hint("请重新加载该文件后再保存；若确认以工具暂存为准，请先手动备份再覆盖，禁止静默覆盖。")
}

/// 读取文件字节并同时生成指纹（字节只读一次，避免哈希与备份内容不一致）。
fn read_current(path: &Path) -> AppResult<(Fingerprint, Vec<u8>)> {
    let bytes = fs::read(path).map_err(|e| map_io_error(Stage::Read, path, &e))?;
    let metadata = fs::metadata(path).map_err(|e| map_io_error(Stage::Read, path, &e))?;
    let fingerprint = Fingerprint {
        mtime_ms: mtime_millis(&metadata),
        // 用实际读到的字节数，保证 size 与 sha256 描述的是同一份内容
        size: bytes.len() as u64,
        sha256: fingerprint_bytes(&bytes),
    };
    Ok((fingerprint, bytes))
}

fn mtime_millis(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        // 早于 UNIX 纪元（极罕见）时退化为 0，避免整个打开流程失败
        .unwrap_or(0)
}

/// 符号链接解析：只在需要时 `canonicalize`，普通文件保持原路径
/// （Windows 的 canonicalize 会返回 `\\?\` 前缀，没必要给普通文件用）。
fn resolve_write_target(source: &Path) -> AppResult<PathBuf> {
    match fs::symlink_metadata(source) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::canonicalize(source).map_err(|e| map_io_error(Stage::Read, source, &e))
        }
        Ok(_) => Ok(source.to_path_buf()),
        Err(e) => Err(map_io_error(Stage::Read, source, &e)),
    }
}

fn validate_staged(staged: &[u8], validate_utf8: bool) -> AppResult<()> {
    if !validate_utf8 {
        return Ok(());
    }
    if let Err(e) = std::str::from_utf8(staged) {
        return Err(AppError::new(
            ErrorCode::CorruptEncoding,
            "暂存内容不是合法 UTF-8，写入已中止；原文件未被修改",
        )
        .with_detail(format!("首个非法字节偏移：{}", e.valid_up_to()))
        .with_hint("请检查编辑内容中的乱码字符后重试。"));
    }
    Ok(())
}

/// 写入后的指纹：优先读磁盘（保证与前端重查结果一致），
/// 极端情况下读不到时用刚写入的字节 + 当前 mtime 兜底。
fn fingerprint_after_write(target: &Path, staged: &[u8]) -> Fingerprint {
    match fingerprint_file(target) {
        Ok(fingerprint) => fingerprint,
        Err(e) => {
            tracing::warn!(path = %target.display(), error = %e, "写入后读取指纹失败，使用已写入字节兜底");
            Fingerprint {
                mtime_ms: fs::metadata(target)
                    .map(|metadata| mtime_millis(&metadata))
                    .unwrap_or(0),
                size: staged.len() as u64,
                sha256: fingerprint_bytes(staged),
            }
        }
    }
}
