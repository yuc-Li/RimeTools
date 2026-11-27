//! 方案配置写入的集成测试：在**副本**上跑完整链路（补丁 + 备份 + 原子写 + 冲突拒绝）。
//!
//! 全程使用临时目录，不触碰真实 Rime 配置。

use std::path::Path;

use rime_core::fingerprint_file;
use rime_platform::scheme_config;
use rime_safety::{save_file, WriteSafetyOptions};

fn fixtures() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures")
}

fn copy_fixture(src: &str, dst: &Path, name: &str) -> std::path::PathBuf {
    let to = dst.join(name);
    std::fs::copy(fixtures().join(src), &to).expect("复制夹具失败");
    to
}

fn safety(dir: &Path) -> WriteSafetyOptions {
    WriteSafetyOptions {
        backup_dir: Some(dir.join("backups")),
        ..Default::default()
    }
}

const TEMPLATE: &str = "\
##  一些常用的设置项
# patch:
  # speller/auto_select: true # 自动上屏
  # style/inline_preedit: true
";

#[test]
fn patch_custom_creates_active_block_and_keeps_comments() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("xhup.custom.yaml");
    std::fs::write(&path, TEMPLATE).unwrap();

    let before = std::fs::read_to_string(&path).unwrap();
    let next = scheme_config::patch_custom_yaml(Some(&before), Some(false), Some(4));
    let expected = fingerprint_file(&path).unwrap();
    let outcome = save_file(&path, &expected, next.as_bytes(), &safety(dir.path())).unwrap();

    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.lines().any(|l| l.trim() == "patch:"),
        "应激活 patch 块"
    );
    assert!(after.contains("  speller/auto_select: false"));
    assert!(after.contains("  speller/max_code_length: 4"));
    assert!(
        after.contains("# style/inline_preedit: true"),
        "注释必须保留"
    );

    // 写前备份存在，且内容就是原始文件
    let backup = std::fs::read_to_string(&outcome.backup.path).unwrap();
    assert_eq!(backup, before, "备份内容必须是写前原文");
}

#[test]
fn patched_values_are_readable_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("xhup.custom.yaml");
    std::fs::write(&path, TEMPLATE).unwrap();

    let next = scheme_config::patch_custom_yaml(Some(TEMPLATE), Some(false), Some(6));
    std::fs::write(&path, &next).unwrap();

    let cfg = scheme_config::read_speller_from_custom(&next);
    assert_eq!(cfg.auto_select, Some(false));
    assert_eq!(cfg.max_code_length, Some(6));
}

#[test]
fn toggling_mount_writes_and_roundtrips() {
    let dir = tempfile::tempdir().unwrap();
    let dict = copy_fixture(
        "rime-crane-xhup/xhup.dict.yaml",
        dir.path(),
        "xhup.dict.yaml",
    );
    let original = std::fs::read_to_string(&dict).unwrap();
    let mounts = scheme_config::parse_mounts(&original);
    assert!(mounts.len() >= 3, "夹具里应有多个挂载项");

    let target = mounts[0].0.clone();
    let off = scheme_config::toggle_mounts(&original, &[(target.clone(), false)]).unwrap();

    let expected = fingerprint_file(&dict).unwrap();
    let outcome = save_file(&dict, &expected, off.as_bytes(), &safety(dir.path())).unwrap();
    let after = std::fs::read_to_string(&dict).unwrap();

    assert!(
        after.contains(&format!("# - \"{target}\"")),
        "应停用目标词表"
    );
    assert!(
        after.lines().count() == original.lines().count(),
        "停用不应增删行"
    );
    assert_eq!(
        std::fs::read_to_string(&outcome.backup.path).unwrap(),
        original,
        "备份必须是写前原文"
    );

    // 再打开，应逐字还原
    let on = scheme_config::toggle_mounts(&after, &[(target, true)]).unwrap();
    assert_eq!(on, original, "关→开 应逐字还原");
}

#[test]
fn stale_fingerprint_is_rejected_without_touching_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("xhup.custom.yaml");
    std::fs::write(&path, TEMPLATE).unwrap();

    // 模拟「打开之后文件被外部改过」
    let stale = fingerprint_file(&path).unwrap();
    std::fs::write(&path, format!("{TEMPLATE}\n# 外部追加的一行\n")).unwrap();
    let current = std::fs::read_to_string(&path).unwrap();

    let next = scheme_config::patch_custom_yaml(Some(&current), Some(true), None);
    let err = save_file(&path, &stale, next.as_bytes(), &safety(dir.path()))
        .expect_err("指纹不一致必须拒绝写入");
    assert_eq!(err.code, rime_core::types::ErrorCode::ExternalModified);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        current,
        "拒绝写入时文件不得被改动"
    );
}

#[test]
fn mount_parse_ignores_other_sections() {
    let dir = tempfile::tempdir().unwrap();
    let dict = copy_fixture(
        "rime-crane-xhup/xhup.dict.yaml",
        dir.path(),
        "xhup.dict.yaml",
    );
    let text = std::fs::read_to_string(&dict).unwrap();
    let mounts = scheme_config::parse_mounts(&text);

    // 不应把注释分节标题、或 import_tables 之后的内容当成挂载项
    assert!(
        mounts.iter().all(|(name, _, _)| name.contains('/')),
        "挂载项都应带目录，实际：{mounts:?}"
    );
}
