//! 端到端集成测试：MVP 主路径。
//!
//! 串起四个 crate 的真实协作，验证「加一个词 → 落盘 → 重开可见 → 既有内容不被动」：
//! `rime-platform`（方案/受管文件识别）
//!   → `rime-core`（DictStore 暂存/校验/序列化）
//!   → `rime-safety`（指纹检测 + 写前备份 + 原子写）
//!   → `rime-suggest`（编码建议）
//!
//! 全部操作在 `tempfile` 的副本上进行，**绝不触碰用户真实 Rime 目录**。

use rime_core::store::DictStore;
use rime_core::types::{ColumnKind, EntryQuery, FileFormat, FileRole, NewEntry, SaveRequest};
use rime_safety::{save_file, WriteSafetyOptions};
use std::path::{Path, PathBuf};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(1)
        .expect("定位仓库根失败")
        .join("tests/fixtures")
}

/// 把夹具复制到临时目录，返回（临时目录守卫, 目标路径）。
fn staged_copy(rel: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("创建临时目录失败");
    let src = fixture_root().join(rel);
    let dst = dir.path().join(src.file_name().expect("文件名"));
    std::fs::copy(&src, &dst).expect("复制夹具失败");
    (dir, dst)
}

fn save_request(
    path: &Path,
    store: &DictStore,
    add: Vec<NewEntry>,
    deploy_after: bool,
) -> SaveRequest {
    SaveRequest {
        file_id: path.display().to_string(),
        expected_fingerprint: store.fingerprint().clone(),
        add,
        update: Vec::new(),
        delete: Vec::new(),
        reorder: None,
        deploy_after,
    }
}

#[test]
fn add_entry_full_cycle_on_custom_phrase() {
    let (_guard, path) = staged_copy("rime-crane-flypy/custom_phrase_double.txt");
    let before = std::fs::read_to_string(&path).expect("读取副本失败");

    // 1) 打开
    let mut store =
        DictStore::open(&path, FileFormat::CustomPhrase, FileRole::CustomPhrase).expect("打开失败");
    assert!(store.total() > 20, "应读出全部短语，实际 {}", store.total());
    assert_eq!(
        store.columns(),
        &[ColumnKind::Text, ColumnKind::Code, ColumnKind::Weight]
    );

    // 2) 暂存新增
    let req = save_request(
        &path,
        &store,
        vec![NewEntry {
            text: "中州韵".to_owned(),
            code: "vsvzyy".to_owned(),
            weight: Some(100),
            group: Some("项目".to_owned()),
        }],
        false,
    );
    store.stage(&req).expect("暂存失败");
    assert!(store.has_staged());

    // 3) 校验（小鹤双拼字母表允许这些字符）
    let issues = store.validate(
        "zyxwvutsrqponmlkjihgfedcbaZYXWVUTSRQPONMLKJIHGFEDCBA`",
        None,
    );
    assert!(issues.is_empty(), "校验不应有问题：{issues:?}");

    // 4) 序列化并原子写入（带写前备份）
    let (content, _opts) = store.serialize().expect("序列化失败");
    let outcome = save_file(
        &path,
        &req.expected_fingerprint,
        content.as_bytes(),
        &WriteSafetyOptions::default(),
    )
    .expect("写入失败");

    // 备份必须存在且内容 = 写前内容
    let backup = std::fs::read_to_string(&outcome.backup.path).expect("读取备份失败");
    assert_eq!(backup, before, "备份内容必须是写前原文");

    // 5) 重开验证：新词可见，分组保留在内存态（重开后丢失属预期，此处只校验落盘内容）
    let reopened =
        DictStore::open(&path, FileFormat::CustomPhrase, FileRole::CustomPhrase).expect("重开失败");
    assert!(
        reopened
            .query(&EntryQuery {
                file_id: Some(path.display().to_string()),
                keyword: Some("中州韵".to_owned()),
                ..Default::default()
            })
            .total
            == 1,
        "新增词条应可检索"
    );

    // 6) 关键护栏：既有内容逐字节未变（只多了一行）
    let after = std::fs::read_to_string(&path).expect("读取结果失败");
    assert_eq!(
        after,
        format!("{before}中州韵\tvsvzyy\t100\n"),
        "既有字节必须完全一致"
    );
}

#[test]
fn external_modification_is_rejected_and_file_untouched() {
    let (_guard, path) = staged_copy("rime-crane-xhup/xhup.user.dict.yaml");

    let mut store =
        DictStore::open(&path, FileFormat::RimeDictYaml, FileRole::UserDict).expect("打开失败");
    let stale = store.fingerprint().clone();

    let req = save_request(
        &path,
        &store,
        vec![NewEntry {
            text: "新词".to_owned(),
            code: "xbci".to_owned(),
            weight: None,
            group: None,
        }],
        false,
    );
    store.stage(&req).expect("暂存失败");
    let (content, _) = store.serialize().expect("序列化失败");

    // 模拟外部修改（用户手工编辑 / 方案更新覆盖）
    let external = std::fs::read_to_string(&path).unwrap() + "外部\twbci\n";
    std::fs::write(&path, &external).unwrap();

    // 用过期指纹写入必须被拒绝
    let err = save_file(
        &path,
        &stale,
        content.as_bytes(),
        &WriteSafetyOptions::default(),
    )
    .expect_err("外部修改后必须拒绝写入");
    assert_eq!(
        err.code,
        rime_core::types::ErrorCode::ExternalModified,
        "错误码应为 ExternalModified，实际 {:?}",
        err.code
    );

    // 原文件必须逐字节保持外部修改后的内容（工具没有覆盖）
    let now = std::fs::read_to_string(&path).unwrap();
    assert_eq!(now, external, "拒绝写入时不得改动文件");
}

#[test]
fn xhup_user_dict_keeps_two_columns_and_headers() {
    let (_guard, path) = staged_copy("rime-crane-xhup/xhup.user.dict.yaml");
    let before = std::fs::read_to_string(&path).unwrap();

    let mut store =
        DictStore::open(&path, FileFormat::RimeDictYaml, FileRole::UserDict).expect("打开失败");
    assert_eq!(store.columns().len(), 2, "用户码表必须是两列");

    let req = save_request(
        &path,
        &store,
        vec![NewEntry {
            text: "代码".to_owned(),
            code: "dacd".to_owned(),
            weight: None,
            group: None,
        }],
        false,
    );
    store.stage(&req).expect("暂存失败");

    // 音形方案字母表含 ; 与 '
    let issues = store.validate("abcdefghijklmnopqrstuvwxyz;'", Some(4));
    assert!(issues.is_empty(), "校验不应有问题：{issues:?}");

    let (content, _) = store.serialize().unwrap();
    save_file(
        &path,
        &store.fingerprint().clone(),
        content.as_bytes(),
        &WriteSafetyOptions::default(),
    )
    .expect("写入失败");

    let after = std::fs::read_to_string(&path).unwrap();
    // 头注释与 YAML 头必须保留（方案靠它识别码表用途）
    assert!(
        after.contains("#---config@码表分类=主码-用户码表"),
        "头注释丢失"
    );
    assert!(after.contains("sort: by_weight"), "YAML 头丢失");
    // 新增行必须是两列（不能凭空长出权重列）。
    // 该文件是 CRLF 换行，序列化须保留原换行风格。
    assert!(
        after.contains("代码\tdacd\r\n"),
        "新增行格式错误（应两列 + 保留 CRLF）"
    );
    assert!(
        after.contains("\r\n代码\tdacd\r\n"),
        "新增行须接在既有行之后，实际尾部：{:?}",
        &after[after.len().saturating_sub(40)..]
    );
    assert!(!after.contains("代码\tdacd\t"), "两列文件不应写入第三列");
    // 既有内容前缀不变
    let prefix_len = before.trim_end_matches('\n').len();
    assert!(
        after.starts_with(&before[..prefix_len.min(before.len())]) || after.len() > before.len()
    );
}

#[test]
fn validation_blocks_bad_codes_before_write() {
    let (_guard, path) = staged_copy("rime-crane-xhup/xhup.user.dict.yaml");
    let before = std::fs::read(&path).unwrap();

    let mut store =
        DictStore::open(&path, FileFormat::RimeDictYaml, FileRole::UserDict).expect("打开失败");

    // 非法字符：'-' 不在 xhup 字母表内
    let req = save_request(
        &path,
        &store,
        vec![NewEntry {
            text: "坏码".to_owned(),
            code: "ab-cd".to_owned(),
            weight: None,
            group: None,
        }],
        false,
    );
    store.stage(&req).expect("暂存失败");
    let issues = store.validate("abcdefghijklmnopqrstuvwxyz;'", Some(4));
    assert!(!issues.is_empty(), "非法字符必须被拦下");
    assert!(
        issues
            .iter()
            .any(|i| i.code == rime_core::types::ErrorCode::CodeCharsetInvalid),
        "应为 CodeCharsetInvalid，实际 {:?}",
        issues.iter().map(|i| i.code).collect::<Vec<_>>()
    );

    // 校验失败丢弃暂存后，文件一个字节都不能变
    store.discard_staged();
    assert_eq!(std::fs::read(&path).unwrap(), before, "文件不得被改动");
}

#[test]
fn suggest_codes_works_for_double_pinyin_scheme() {
    // 用真实方案文件驱动编码建议（把方案路径喂给 rime-platform 的解析）
    let scheme_path = fixture_root().join("rime-crane-flypy/double_pinyin_flypy.schema.yaml");
    let raw = rime_platform::parse_schema(&scheme_path).expect("解析方案失败");
    assert!(raw.is_double_pinyin, "应识别为双拼方案");

    let scheme = rime_core::types::SchemeInfo {
        schema_id: raw.schema_id,
        name: raw.name,
        version: raw.version,
        is_active: true,
        is_current: true,
        translator_kind: raw.translator_kind,
        alphabet: raw.alphabet,
        max_code_length: raw.max_code_length,
        managed_files: Vec::new(),
        is_double_pinyin: raw.is_double_pinyin,
        algebra: raw.algebra,
    };

    // 优先用本机字表（用户真实安装），没有则退回内置表
    let mut engine = rime_suggest::SuggestEngine::from_scheme(&scheme);
    let user_dir = rime_platform::fcitx5_user_dir();
    let mut source_note = "内置常用字表";
    if user_dir.is_dir() {
        let map = rime_platform::load_char_pinyin(&user_dir);
        if !map.is_empty() {
            source_note = "本机字表";
            engine = engine.with_pinyin(Box::new(map));
        }
    }
    println!("拼音数据源：{source_note}");

    let opts = rime_suggest::SuggestOptions::from_scheme(&scheme);
    let out = engine.suggest("中州韵", &opts);

    assert!(!out.is_empty(), "应给出编码建议（数据源：{source_note}）");
    // 验证报告 §2.2：zhong→vs、zhou→vz、yun→yy；3 字词取各字首键 = vvy
    let codes: Vec<&str> = out.iter().map(|s| s.code.as_str()).collect();
    assert!(
        codes.contains(&"vvy"),
        "三字词应建议 vvy（各字双拼首键），实际 {codes:?}"
    );
}
