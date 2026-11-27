//! 写入安全协议集成测试。
//!
//! 核心断言（PRD §8「回归护栏」）：**任何失败路径下原文件必须字节不变**。
//! 所有文件都落在 `tempfile::tempdir()`，绝不触碰真实 Rime 用户目录。

use rime_core::fingerprint_file;
use rime_core::types::{BackupReason, ErrorCode, Fingerprint};
use rime_safety::{
    current_fingerprint, fingerprint_matches, list_backups, save_file, save_parsed_dict,
    verify_expected, FingerprintPolicy, WriteSafetyOptions,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;

const ORIGINAL: &str = "# Rime table\n# coding: utf-8\n\n一个\tig\t100\n";
const UPDATED: &str = "# Rime table\n# coding: utf-8\n\n一个\tig\t200\n";

/// 建一个临时目录 + 目标文件，返回 (目录守卫, 文件路径, 打开时指纹)。
fn fixture(content: &str) -> (TempDir, PathBuf, Fingerprint) {
    let dir = tempfile::tempdir().expect("创建临时目录");
    let path = dir.path().join("custom_phrase.txt");
    fs::write(&path, content).expect("写初始文件");
    let fingerprint = fingerprint_file(&path).expect("初始指纹");
    (dir, path, fingerprint)
}

fn assert_bytes(path: &Path, expected: &[u8]) {
    assert_eq!(
        fs::read(path).expect("读取文件"),
        expected,
        "原文件字节必须保持不变：{}",
        path.display()
    );
}

fn assert_no_temp_files(dir: &Path) {
    for entry in fs::read_dir(dir).expect("列目录") {
        let name = entry
            .expect("目录项")
            .file_name()
            .to_string_lossy()
            .into_owned();
        assert!(
            !name.contains(".rime-safety-") && !name.ends_with(".tmp"),
            "临时文件未清理：{name}"
        );
    }
}

fn assert_no_backups(path: &Path) {
    assert!(
        list_backups(path, None).expect("列备份").is_empty(),
        "拒绝写入时不应产生备份"
    );
}

// ───────────────────────── 正常写入 ─────────────────────────

#[test]
fn normal_write_updates_content_creates_backup_and_returns_disk_fingerprint() {
    let (dir, path, expected) = fixture(ORIGINAL);

    let outcome = save_file(
        &path,
        &expected,
        UPDATED.as_bytes(),
        &WriteSafetyOptions::default(),
    )
    .expect("正常写入应成功");

    // 内容正确
    assert_bytes(&path, UPDATED.as_bytes());
    // 新指纹与磁盘一致（前端以此刷新）
    assert_eq!(
        outcome.fingerprint,
        fingerprint_file(&path).expect("写后指纹")
    );
    assert_eq!(outcome.fingerprint.sha256.len(), 64);
    assert_eq!(outcome.bytes_written, UPDATED.len() as u64);

    // 备份存在，且内容 = 写入前的旧内容
    assert!(outcome.backup.path.exists(), "写前备份必须存在");
    assert_bytes(&outcome.backup.path, ORIGINAL.as_bytes());
    assert_eq!(outcome.backup.source_path, path);
    assert_eq!(outcome.backup.reason, BackupReason::BeforeWrite);
    assert_eq!(outcome.backup.size, ORIGINAL.len() as u64);
    assert!(outcome.backup.created_at > 0, "备份时间戳必须有效");
    assert!(outcome.backup.id.ends_with(".bak"));

    // 备份列表能查到同一份
    let backups = list_backups(&path, None).expect("列备份");
    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0].path, outcome.backup.path);

    // 无临时文件残留
    assert_no_temp_files(dir.path());
}

#[test]
fn new_fingerprint_reflects_written_bytes() {
    let (_dir, path, expected) = fixture(ORIGINAL);
    let outcome = save_file(
        &path,
        &expected,
        UPDATED.as_bytes(),
        &WriteSafetyOptions::default(),
    )
    .unwrap();

    assert_eq!(outcome.fingerprint.size, UPDATED.len() as u64);
    assert_eq!(
        outcome.fingerprint.sha256,
        rime_core::fingerprint_bytes(UPDATED.as_bytes())
    );
    assert!(outcome.fingerprint != expected);
}

// ───────────────────── 失败注入 1：外部修改 ─────────────────────

#[test]
fn external_modification_is_rejected_without_writing_any_byte() {
    let (dir, path, expected) = fixture(ORIGINAL);

    // 模拟用户手工编辑 / 方案更新覆盖
    let external = "# Rime table\n# coding: utf-8\n\n一个\tig\t100\n外部修改\twbr\t300\n";
    fs::write(&path, external).expect("外部修改");

    let err = save_file(
        &path,
        &expected,
        UPDATED.as_bytes(),
        &WriteSafetyOptions::default(),
    )
    .expect_err("指纹不一致必须拒绝写入");

    assert_eq!(err.code, ErrorCode::ExternalModified);
    assert!(err.message.contains("外部修改"), "message={}", err.message);
    assert!(err.hint.is_some(), "必须给出下一步建议");

    // 一个字节都不能写
    assert_bytes(&path, external.as_bytes());
    // 也不能顺手产生备份 / 临时文件
    assert_no_backups(&path);
    assert_no_temp_files(dir.path());
}

#[test]
fn verify_expected_detects_external_change_without_writing() {
    let (_dir, path, expected) = fixture(ORIGINAL);
    // 未变化 → OK
    verify_expected(&path, &expected, FingerprintPolicy::Strict).expect("内容未变应通过");

    fs::write(&path, UPDATED).expect("外部修改");
    let err = verify_expected(&path, &expected, FingerprintPolicy::Strict)
        .expect_err("内容变化必须被检测到");
    assert_eq!(err.code, ErrorCode::ExternalModified);

    // 内容变了但 mtime 相同也不放过（sha256 兜底）
    let current = current_fingerprint(&path).expect("当前指纹");
    assert!(!fingerprint_matches(
        &expected,
        &current,
        FingerprintPolicy::ContentOnly
    ));
}

#[test]
fn mtime_only_drift_is_rejected_by_strict_but_allowed_by_content_policy() {
    let (_dir, path, expected) = fixture(ORIGINAL);

    // 内容不变、mtime 变（同步工具 / touch 场景）
    std::thread::sleep(Duration::from_millis(20));
    fs::write(&path, ORIGINAL).expect("重写相同内容");
    let drifted = fingerprint_file(&path).expect("漂移后指纹");

    if drifted.mtime_ms != expected.mtime_ms {
        let err = save_file(
            &path,
            &expected,
            UPDATED.as_bytes(),
            &WriteSafetyOptions::default(),
        )
        .expect_err("Strict 策略下 mtime 漂移应拒绝");
        assert_eq!(err.code, ErrorCode::ExternalModified);
        assert_bytes(&path, ORIGINAL.as_bytes());
    } else {
        // 该文件系统 mtime 精度不足，无法构造差异；内容策略仍会被覆盖测试
    }

    // 内容一致：ContentOnly 允许写入
    let opts = WriteSafetyOptions {
        fingerprint_policy: FingerprintPolicy::ContentOnly,
        ..Default::default()
    };
    save_file(&path, &expected, UPDATED.as_bytes(), &opts).expect("ContentOnly 下内容一致应允许");
    assert_bytes(&path, UPDATED.as_bytes());
}

// ───────────────── 失败注入 2：备份失败 / 备份目录不可写 ─────────────────

#[test]
fn backup_failure_aborts_write_and_keeps_original() {
    let (dir, path, expected) = fixture(ORIGINAL);

    // 备份「目录」其实是一个普通文件 → 无法创建备份
    let bogus = dir.path().join("backup-is-a-file");
    fs::write(&bogus, b"not a directory").expect("占位文件");

    let options = WriteSafetyOptions::new().with_backup_dir(&bogus);
    let err = save_file(&path, &expected, UPDATED.as_bytes(), &options)
        .expect_err("备份失败必须中止写入");

    assert_eq!(err.code, ErrorCode::BackupFailed, "detail={:?}", err.detail);
    assert_bytes(&path, ORIGINAL.as_bytes());
    assert_no_temp_files(dir.path());
}

#[test]
fn unwritable_backup_directory_aborts_write() {
    let (dir, path, expected) = fixture(ORIGINAL);
    let backup_dir = dir.path().join("backups");
    fs::create_dir(&backup_dir).expect("创建备份目录");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if running_as_root() {
            eprintln!("跳过：root 不受目录权限限制");
            return;
        }
        fs::set_permissions(&backup_dir, fs::Permissions::from_mode(0o555)).expect("设为只读");
        let _guard = ModeGuard::new(backup_dir.clone(), 0o700);

        let options = WriteSafetyOptions::new().with_backup_dir(&backup_dir);
        let err = save_file(&path, &expected, UPDATED.as_bytes(), &options)
            .expect_err("备份目录不可写必须中止写入");
        assert_eq!(
            err.code,
            ErrorCode::PermissionDenied,
            "detail={:?}",
            err.detail
        );
        assert_bytes(&path, ORIGINAL.as_bytes());
        assert_no_temp_files(dir.path());
    }

    #[cfg(not(unix))]
    {
        let _ = (path, expected);
    }
}

// ───────────────────── 失败注入 3：只读目录 ─────────────────────

#[cfg(unix)]
#[test]
fn read_only_directory_yields_permission_denied_and_keeps_original() {
    if running_as_root() {
        eprintln!("跳过：root 不受目录权限限制");
        return;
    }
    let (dir, path, expected) = fixture(ORIGINAL);

    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).expect("目录设为只读");
        // 守卫先于 tempdir 析构，恢复权限避免 TempDir 清理失败
        let _guard = ModeGuard::new(dir.path().to_path_buf(), 0o700);

        let err = save_file(
            &path,
            &expected,
            UPDATED.as_bytes(),
            &WriteSafetyOptions::default(),
        )
        .expect_err("只读目录必须拒绝写入");
        assert_eq!(
            err.code,
            ErrorCode::PermissionDenied,
            "detail={:?}",
            err.detail
        );
        assert!(err.message.contains("权限"), "message={}", err.message);
        assert_bytes(&path, ORIGINAL.as_bytes());
        assert_no_temp_files(dir.path());
    }
    // 恢复权限后目录仍可用
    assert_no_temp_files(dir.path());
}

// ───────────────────── 其他拒绝路径 ─────────────────────

#[test]
fn missing_file_returns_file_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gone.txt");
    let expected = Fingerprint {
        mtime_ms: 0,
        size: 0,
        sha256: String::new(),
    };

    let err = save_file(
        &path,
        &expected,
        UPDATED.as_bytes(),
        &WriteSafetyOptions::default(),
    )
    .expect_err("文件不存在应报错");
    assert_eq!(err.code, ErrorCode::FileNotFound);
}

#[test]
fn invalid_utf8_staged_content_is_rejected_before_backup() {
    let (_dir, path, expected) = fixture(ORIGINAL);
    let err = save_file(
        &path,
        &expected,
        b"\xff\xfe\x00bad",
        &WriteSafetyOptions::default(),
    )
    .expect_err("非法 UTF-8 必须拒绝");
    assert_eq!(err.code, ErrorCode::CorruptEncoding);
    assert_bytes(&path, ORIGINAL.as_bytes());
    assert_no_backups(&path);
}

// ───────────────────── 备份保留策略 ─────────────────────

#[test]
fn prune_keep_count_is_enforced_after_write() {
    let (_dir, path, mut expected) = fixture(ORIGINAL);
    let options = WriteSafetyOptions::new().with_prune_keep_count(2);

    for i in 0..3u32 {
        let content = format!("# Rime table\n# coding: utf-8\n\n一个\tig\t{}\n", 100 + i);
        let outcome = save_file(&path, &expected, content.as_bytes(), &options).expect("写入");
        expected = outcome.fingerprint;
    }

    let backups = list_backups(&path, None).expect("列备份");
    assert_eq!(backups.len(), 2, "keep_count=2 必须只保留 2 份");
    assert_bytes(
        &path,
        b"# Rime table\n# coding: utf-8\n\n\xe4\xb8\x80\xe4\xb8\xaa\tig\t102\n",
    );
}

// ───────────────────── Unix 专属：权限 / 符号链接 ─────────────────────

#[cfg(unix)]
struct ModeGuard {
    path: PathBuf,
    mode: u32,
}

#[cfg(unix)]
impl ModeGuard {
    fn new(path: PathBuf, mode: u32) -> Self {
        Self { path, mode }
    }
}

#[cfg(unix)]
impl Drop for ModeGuard {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(self.mode));
    }
}

#[cfg(unix)]
fn running_as_root() -> bool {
    use std::os::unix::fs::MetadataExt;
    // /proc/self 的 owner uid 即 euid（Linux）；其他 Unix 无 /proc 时按非 root 处理
    fs::metadata("/proc/self")
        .map(|m| m.uid() == 0)
        .unwrap_or(false)
}

#[cfg(unix)]
#[test]
fn atomic_write_preserves_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, path, expected) = fixture(ORIGINAL);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("设为 0600");

    save_file(
        &path,
        &expected,
        UPDATED.as_bytes(),
        &WriteSafetyOptions::default(),
    )
    .expect("写入");

    let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "改写后必须保留原权限位");
}

#[cfg(unix)]
#[test]
fn writes_through_symlink_without_replacing_it() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real-dict.yaml");
    fs::write(&real, ORIGINAL).expect("写真实文件");
    let link = dir.path().join("link-dict.yaml");
    std::os::unix::fs::symlink(&real, &link).expect("创建符号链接");

    let expected = fingerprint_file(&link).expect("链接指纹");
    let outcome = save_file(
        &link,
        &expected,
        UPDATED.as_bytes(),
        &WriteSafetyOptions::default(),
    )
    .expect("通过符号链接写入");

    assert_bytes(&real, UPDATED.as_bytes());
    assert!(
        fs::symlink_metadata(&link)
            .expect("链接元数据")
            .file_type()
            .is_symlink(),
        "符号链接本身不能被替换成普通文件"
    );
    assert_bytes(&outcome.backup.path, ORIGINAL.as_bytes());
}
// ───────────────────── 与 rime-core 行模型的衔接 ─────────────────────

#[test]
fn parsed_dict_serialization_is_saved_through_the_protocol() {
    use rime_core::types::FileFormat;
    use rime_core::{Line, ParsedDict};

    let (dir, path, expected) = fixture(ORIGINAL);
    let mut dict = ParsedDict::parse(ORIGINAL, FileFormat::CustomPhrase).expect("解析词库");
    let index = dict.find_entry("一个", "ig").expect("找到词条");
    if let Line::Entry(entry) = &mut dict.lines[index] {
        entry.set_weight(Some(200));
    }
    let (staged, write_options) = dict.serialize_default().expect("序列化");
    assert!(staged.contains("一个\tig\t200"), "staged={staged}");

    let outcome = save_parsed_dict(
        &path,
        &expected,
        &dict,
        &write_options,
        &WriteSafetyOptions::default(),
    )
    .expect("保存");

    assert_bytes(&path, staged.as_bytes());
    assert_bytes(&outcome.backup.path, ORIGINAL.as_bytes());
    assert_eq!(outcome.fingerprint, fingerprint_file(&path).unwrap());
    assert_no_temp_files(dir.path());
}

#[test]
fn invalid_serialize_options_abort_before_backup_or_write() {
    use rime_core::types::{ColumnKind, FileFormat};
    use rime_core::{Newline, ParsedDict, WriteOptions};

    let (_dir, path, expected) = fixture(ORIGINAL);
    let dict = ParsedDict::parse(ORIGINAL, FileFormat::CustomPhrase).expect("解析词库");
    // 缺少编码列 → 序列化必然失败
    let bad_options = WriteOptions::new(vec![ColumnKind::Text], Newline::Lf);

    let err = save_parsed_dict(
        &path,
        &expected,
        &dict,
        &bad_options,
        &WriteSafetyOptions::default(),
    )
    .expect_err("非法列配置必须拒绝");
    assert_eq!(err.code, ErrorCode::FileFormatInvalid);
    assert_bytes(&path, ORIGINAL.as_bytes());
    assert_no_backups(&path);
}
