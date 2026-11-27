//! 备份（FR-6.1/6.3/6.4）：命名兼容、列表过滤、按份数清理。

use rime_core::types::{BackupReason, ErrorCode};
use rime_safety::backup::BackupOptions;
use rime_safety::{backup_file, create_backup, list_backups, prune_backups, resolve_backup_dir};
use std::fs;
use tempfile::TempDir;

/// 校验 `原文件名_yyyyMMdd_HHmmss[_N].bak`（参考工具兼容格式）。
fn is_compat_name(name: &str, source_name: &str) -> bool {
    let Some(rest) = name.strip_prefix(&format!("{source_name}_")) else {
        return false;
    };
    let Some(rest) = rest.strip_suffix(".bak") else {
        return false;
    };
    let bytes = rest.as_bytes();
    if bytes.len() < 15 {
        return false;
    }
    if !bytes[..8].iter().all(u8::is_ascii_digit) || bytes[8] != b'_' {
        return false;
    }
    if !bytes[9..15].iter().all(u8::is_ascii_digit) {
        return false;
    }
    if bytes.len() == 15 {
        return true;
    }
    match rest[15..].strip_prefix('_') {
        Some(seq) => !seq.is_empty() && seq.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

fn fixture() -> (TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("custom_phrase_double.txt");
    fs::write(&source, b"# header\n\xe4\xb8\x80\xe4\xb8\xaa\tig\n").unwrap();
    (dir, source)
}

#[test]
fn backup_name_matches_reference_tool_format() {
    let (dir, source) = fixture();
    let backup = create_backup(&source, b"old contents", &BackupOptions::default()).unwrap();

    assert_eq!(backup.path.parent().unwrap(), dir.path());
    assert!(
        is_compat_name(&backup.id, "custom_phrase_double.txt"),
        "备份名不合规：{}",
        backup.id
    );
    assert_eq!(
        backup.id,
        backup.path.file_name().unwrap().to_string_lossy()
    );
    assert_eq!(backup.source_path, source);
    assert_eq!(backup.size, b"old contents".len() as u64);
    assert_eq!(backup.reason, BackupReason::BeforeWrite);
    assert!(backup.created_at > 0);
    assert_eq!(fs::read(&backup.path).unwrap(), b"old contents");
}

#[test]
fn backup_file_reads_from_disk_and_marks_manual_reason() {
    let (_dir, source) = fixture();
    let original = fs::read(&source).unwrap();

    let backup = backup_file(
        &source,
        &BackupOptions::default().for_reason(BackupReason::Manual),
    )
    .unwrap();

    assert_eq!(backup.reason, BackupReason::Manual);
    assert_eq!(fs::read(&backup.path).unwrap(), original);
}

#[test]
fn configured_backup_dir_is_created_and_used() {
    let (dir, source) = fixture();
    let backup_dir = dir.path().join("backups").join("nested");
    let backup = create_backup(&source, b"x", &BackupOptions::in_dir(&backup_dir)).unwrap();

    assert_eq!(backup.path.parent().unwrap(), backup_dir);
    assert!(backup_dir.is_dir(), "配置的备份目录应自动创建");
    assert_eq!(list_backups(&source, Some(&backup_dir)).unwrap().len(), 1);
    // 默认目录（源目录）里不应该有
    assert!(list_backups(&source, None).unwrap().is_empty());
}

#[test]
fn list_ignores_unrelated_files_and_other_sources() {
    let (dir, source) = fixture();
    let other = dir.path().join("another.txt");
    fs::write(&other, b"x").unwrap();

    create_backup(&source, b"mine", &BackupOptions::default()).unwrap();
    create_backup(&other, b"not mine", &BackupOptions::default()).unwrap();
    // 干扰项：非备份名 / 旧式名
    fs::write(dir.path().join("custom_phrase_double.txt.bak"), b"legacy").unwrap();
    fs::write(dir.path().join("notes.md"), b"hello").unwrap();

    let list = list_backups(&source, None).unwrap();
    assert_eq!(list.len(), 1, "只应返回本文件的备份：{list:?}");
    assert_eq!(fs::read(&list[0].path).unwrap(), b"mine");

    let all_other = list_backups(&other, None).unwrap();
    assert_eq!(all_other.len(), 1);
    assert_eq!(fs::read(&all_other[0].path).unwrap(), b"not mine");
}

#[test]
fn prune_keeps_newest_files_and_reports_count() {
    let (_dir, source) = fixture();
    let opts = BackupOptions::default();
    for i in 1..=5u32 {
        fs::write(&source, format!("v{i}")).unwrap();
        create_backup(&source, format!("v{i}").as_bytes(), &opts).unwrap();
    }

    let outcome = prune_backups(&source, None, 3).unwrap();
    assert_eq!(outcome.deleted.len(), 2);
    assert_eq!(outcome.kept, 3);

    let list = list_backups(&source, None).unwrap();
    assert_eq!(list.len(), 3);
    let contents: Vec<Vec<u8>> = list.iter().map(|b| fs::read(&b.path).unwrap()).collect();
    assert_eq!(
        contents,
        vec![b"v5".to_vec(), b"v4".to_vec(), b"v3".to_vec()]
    );
}

#[test]
fn prune_with_zero_keep_count_deletes_everything() {
    let (_dir, source) = fixture();
    create_backup(&source, b"a", &BackupOptions::default()).unwrap();
    create_backup(&source, b"b", &BackupOptions::default()).unwrap();

    let outcome = prune_backups(&source, None, 0).unwrap();
    assert_eq!(outcome.deleted.len(), 2);
    assert_eq!(outcome.kept, 0);
    assert!(list_backups(&source, None).unwrap().is_empty());
}

#[test]
fn prune_is_noop_when_under_keep_count() {
    let (_dir, source) = fixture();
    create_backup(&source, b"only", &BackupOptions::default()).unwrap();

    let outcome = prune_backups(&source, None, 10).unwrap();
    assert!(outcome.deleted.is_empty());
    assert_eq!(outcome.kept, 1);
}

#[test]
fn resolve_backup_dir_defaults_to_source_directory() {
    let (dir, source) = fixture();
    assert_eq!(resolve_backup_dir(&source, None), dir.path());

    let custom = dir.path().join("b");
    assert_eq!(resolve_backup_dir(&source, Some(&custom)), custom);
}

#[test]
fn backup_of_missing_source_reports_file_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.txt");
    let err = backup_file(&missing, &BackupOptions::default()).unwrap_err();
    assert_eq!(err.code, ErrorCode::FileNotFound);
}
