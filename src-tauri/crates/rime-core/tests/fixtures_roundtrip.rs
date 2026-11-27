//! 集成测试：用真实凇鹤配置的脱敏副本验证往返保真与解析正确性。
//!
//! 夹具来源：用户实机 `~/.local/share/fcitx5/rime`（已脱敏，见 `tests/fixtures/`）。
//! 这些用例是「不写坏用户文件」这条护栏的直接证据。

use rime_core::types::FileFormat;
use rime_core::ParsedDict;
use std::path::{Path, PathBuf};

fn fixture(rel: &str) -> PathBuf {
    // CARGO_MANIFEST_DIR = src-tauri/crates/rime-core
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("定位仓库根失败")
        .to_path_buf();
    root.join("tests/fixtures").join(rel)
}

fn read_fixture(rel: &str) -> String {
    let p = fixture(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("读取夹具失败 {}: {e}", p.display()))
}

/// 所有夹具都必须「解析 → 序列化」逐字节相同。
#[test]
fn all_fixtures_roundtrip_byte_identical() {
    let cases = [
        ("rime-crane-flypy/custom_phrase_double.txt", FileFormat::CustomPhrase),
        ("rime-crane-flypy/default.custom.yaml", FileFormat::PlainTxt),
        ("rime-crane-flypy/rime_ice.dict.yaml", FileFormat::RimeDictYaml),
        ("rime-crane-xhup/xhup.dict.yaml", FileFormat::RimeDictYaml),
        ("rime-crane-xhup/xhup.user.dict.yaml", FileFormat::RimeDictYaml),
        ("rime-crane-xhup/xhup.user.top.dict.yaml", FileFormat::RimeDictYaml),
        ("rime-crane-xhup/xhup.primary.sample.dict.yaml", FileFormat::RimeDictYaml),
    ];

    for (rel, format) in cases {
        let src = read_fixture(rel);
        let parsed = ParsedDict::parse(&src, format)
            .unwrap_or_else(|e| panic!("{rel} 解析失败: {e}"));
        let (out, _) = parsed
            .serialize_default()
            .unwrap_or_else(|e| panic!("{rel} 序列化失败: {e}"));
        assert_eq!(out, src, "{rel} 往返后内容不一致（会写坏用户文件）");
    }
}

/// 小鹤双拼短语表：应识别出全部词条，且注释行保持为 Raw。
#[test]
fn custom_phrase_double_structure() {
    let src = read_fixture("rime-crane-flypy/custom_phrase_double.txt");
    let parsed = ParsedDict::parse(&src, FileFormat::CustomPhrase).unwrap();

    assert!(parsed.entry_count() > 20, "应解析出全部短语，实际 {}", parsed.entry_count());
    assert!(!parsed.bom);
    assert!(!parsed.has_dirty());

    // 已知词条（用户实机配置）
    let idx = parsed.find_entry("一个", "ig").expect("应存在 一个/ig");
    let (_, e) = parsed.entries().find(|(i, _)| *i == idx).unwrap();
    assert_eq!(e.weight, None, "该行无权重列");

    let idx = parsed.find_entry("有", "u").expect("应存在 有/u");
    let (_, e) = parsed.entries().find(|(i, _)| *i == idx).unwrap();
    assert_eq!(e.weight, Some(3), "有/u 权重应为 3");

    // 英文条目应保留原样大小写
    assert!(parsed.find_entry("Amazon", "amazon").is_some());
    assert!(parsed.find_entry("SOHO", "soho").is_some());

    // 「此行之后不能写注释」约束：数据区不应出现注释行被写入
    let has_comment_after_data = parsed
        .lines
        .iter()
        .skip_while(|l| matches!(l, rime_core::Line::Raw(s) if !s.contains("此行之后不能写注释")))
        .skip(1)
        .any(|l| matches!(l, rime_core::Line::Raw(s) if s.starts_with('#')));
    assert!(!has_comment_after_data, "数据区出现注释行");
}

/// 音形用户码表：两列格式（无权重列），且 `#---config@` 头注释必须保留。
#[test]
fn xhup_user_dict_is_two_column_and_keeps_config_headers() {
    let src = read_fixture("rime-crane-xhup/xhup.user.dict.yaml");
    let parsed = ParsedDict::parse(&src, FileFormat::RimeDictYaml).unwrap();

    assert!(parsed.entry_count() >= 10, "应解析出用户词条");
    // 全部词条都无权重
    for (_, e) in parsed.entries() {
        assert_eq!(e.weight, None, "xhup 用户码表不应有权重列：{}", e.text);
    }
    // 列风格推断为两列
    let cols = rime_core::detect_columns(parsed.format, &parsed.lines);
    assert_eq!(cols.len(), 2, "两列文件不应被推断出权重列");

    // 头注释保留
    let raw_text: String = parsed
        .lines
        .iter()
        .filter_map(|l| l.raw_text())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(raw_text.contains("#---config@码表分类=主码-用户码表"));
    assert!(raw_text.contains("sort: by_weight"));

    // 已知词条：二字词 = 各字双拼全码拼接（夹具为合成数据）
    assert!(parsed.find_entry("你好", "nihc").is_some());
    assert!(parsed.find_entry("中文", "vswf").is_some());
}

/// 置顶码表与普通用户码表可区分（角色不同，写入目标不同）。
#[test]
fn user_top_dict_parses() {
    let src = read_fixture("rime-crane-xhup/xhup.user.top.dict.yaml");
    let parsed = ParsedDict::parse(&src, FileFormat::RimeDictYaml).unwrap();
    assert!(parsed.entry_count() >= 5, "置顶码表应有词条");
    assert!(parsed.find_entry("长短", "ihdr").is_some());
    assert!(parsed.find_entry("示例词", "pxm").is_some(), "自定义码条目应保留");
}

/// 编辑后仅目标行变化，其余字节不变（模拟「批量调权重」的最小场景）。
#[test]
fn editing_one_entry_keeps_rest_intact() {
    let src = read_fixture("rime-crane-flypy/custom_phrase_double.txt");
    let mut parsed = ParsedDict::parse(&src, FileFormat::CustomPhrase).unwrap();

    let idx = parsed.find_entry("有", "u").expect("应存在 有/u");
    if let rime_core::Line::Entry(e) = &mut parsed.lines[idx] {
        e.set_weight(Some(9));
    }
    let (out, _) = parsed.serialize_default().unwrap();

    let expected = src.replace("有\tu\t3\n", "有\tu\t9\n");
    assert_eq!(out, expected, "只应改动目标行");
}

/// 方案依赖的 `import_tables` 头部必须原样保留（改动会破坏词库装载顺序）。
#[test]
fn import_tables_header_preserved() {
    let src = read_fixture("rime-crane-xhup/xhup.dict.yaml");
    let parsed = ParsedDict::parse(&src, FileFormat::RimeDictYaml).unwrap();
    let (out, _) = parsed.serialize_default().unwrap();
    assert_eq!(out, src);
    assert!(out.contains("xhup.user.top"), "置顶表 import 必须保留");
    assert!(out.contains("xhup.user"), "用户表 import 必须保留");
}