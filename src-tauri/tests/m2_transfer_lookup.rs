//! M2 功能端到端测试：导入导出 + 只读反查。
//!
//! 全部在临时目录里进行，不触碰真实 Rime 配置。

use std::path::Path;

use rime_core::lookup::{query, LookupMode, LookupQuery, SourceOrigin};
use rime_core::transfer::{
    export, parse_import, plan_import, to_save_request, ImportPolicy, TransferFormat,
};
use rime_core::types::{FileFormat, FileRole};
use rime_core::DictStore;
use rime_platform::sources;
use rime_platform::symbols;

fn fixtures() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures")
}

fn copy_fixture(src: &str, dst_dir: &Path, name: &str) -> std::path::PathBuf {
    let from = fixtures().join(src);
    let to = dst_dir.join(name);
    std::fs::copy(&from, &to).expect("复制夹具失败");
    to
}

fn open(path: &Path, format: FileFormat, role: FileRole) -> DictStore {
    DictStore::open(path, format, role).expect("打开词库失败")
}

// ───────────────────────── 导出 ─────────────────────────

#[test]
fn export_roundtrips_back_into_the_same_entries() {
    let dir = tempfile::tempdir().unwrap();
    let path = copy_fixture(
        "rime-crane-xhup/xhup.user.dict.yaml",
        dir.path(),
        "xhup.user.dict.yaml",
    );
    let store = open(&path, FileFormat::RimeDictYaml, FileRole::UserDict);
    let original = store.total();

    for format in [
        TransferFormat::Txt,
        TransferFormat::Csv,
        TransferFormat::DictYaml,
    ] {
        let out = export(&store, format);
        assert_eq!(
            out.entry_count as usize, original,
            "{format:?} 应导出全部词条"
        );
        assert!(!out.content.is_empty());

        // 导出的内容应能被自己解析回来，且条数一致
        let parsed = parse_import(&out.content, format).expect("导出内容应可解析");
        assert_eq!(
            parsed.entries.len(),
            original,
            "{format:?} 导出后重新解析条数不一致"
        );
    }
}

#[test]
fn export_csv_escapes_commas() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.dict.yaml");
    std::fs::write(&path, "---\nname: t\n...\n\n带,逗号\tdh\t1\n简单\tjd\n").unwrap();
    let store = open(&path, FileFormat::RimeDictYaml, FileRole::UserDict);

    let csv = export(&store, TransferFormat::Csv);
    assert!(csv.content.contains("\"带,逗号\""), "含逗号的字段应加引号");

    let parsed = parse_import(&csv.content, TransferFormat::Csv).unwrap();
    assert_eq!(parsed.entries.len(), 2);
    assert_eq!(parsed.entries[0].text, "带,逗号");
}

// ───────────────────────── 导入 ─────────────────────────

#[test]
fn import_plan_detects_conflicts_and_policies_change_the_outcome() {
    let dir = tempfile::tempdir().unwrap();
    let path = copy_fixture(
        "rime-crane-xhup/xhup.user.dict.yaml",
        dir.path(),
        "xhup.user.dict.yaml",
    );
    let store = open(&path, FileFormat::RimeDictYaml, FileRole::UserDict);

    // 两列文件：导入权重会被忽略，同（文本,编码）视为已存在
    let content = "你好\tnihc\t9\n新词甲\txinc\n新词乙\txiny\n";
    let parsed = parse_import(content, TransferFormat::Txt).unwrap();
    let plan = plan_import(
        &store,
        &parsed,
        ImportPolicy::SkipExisting,
        "abcdefghijklmnopqrstuvwxyz;'",
        Some(4),
    );

    assert_eq!(plan.total, 3);
    assert_eq!(plan.added, 2, "两个新词应新增");
    assert_eq!(plan.unchanged, 1, "两列文件里已存在即视为不变");
    assert!(plan.weight_ignored, "两列文件应提示权重被忽略");
    assert_eq!(plan.conflicts.len(), 1);
}

#[test]
fn import_apply_writes_new_entries_and_keeps_file_format() {
    let dir = tempfile::tempdir().unwrap();
    let path = copy_fixture(
        "rime-crane-xhup/xhup.user.dict.yaml",
        dir.path(),
        "xhup.user.dict.yaml",
    );
    let before = std::fs::read_to_string(&path).unwrap();
    let mut store = open(&path, FileFormat::RimeDictYaml, FileRole::UserDict);

    let parsed = parse_import("导入词\tdrci\n", TransferFormat::Txt).unwrap();
    let plan = plan_import(
        &store,
        &parsed,
        ImportPolicy::SkipExisting,
        "abcdefghijklmnopqrstuvwxyz;'",
        Some(4),
    );
    assert_eq!(plan.added, 1);

    let request = to_save_request(&store, &plan, false);
    store.stage(&request).unwrap();
    let (content, _opts) = store.serialize().unwrap();

    assert!(content.contains("导入词\tdrci"), "应写入新词条");
    assert!(content.contains("#---config@"), "头部注释必须保留");
    assert!(content.contains("\r\n"), "CRLF 换行必须保留");
    assert_eq!(
        content.lines().count(),
        before.lines().count() + 1,
        "只应多出一行"
    );
}

#[test]
fn import_rejects_codes_outside_alphabet() {
    let dir = tempfile::tempdir().unwrap();
    let path = copy_fixture(
        "rime-crane-xhup/xhup.user.dict.yaml",
        dir.path(),
        "xhup.user.dict.yaml",
    );
    let store = open(&path, FileFormat::RimeDictYaml, FileRole::UserDict);

    let parsed = parse_import("坏码\tA1\n", TransferFormat::Txt).unwrap();
    let plan = plan_import(
        &store,
        &parsed,
        ImportPolicy::SkipExisting,
        "abcdefghijklmnopqrstuvwxyz",
        Some(4),
    );
    assert_eq!(plan.invalid_codes.len(), 1, "大写字母不在字母表内应被标记");
}

// ───────────────────────── 反查 ─────────────────────────

#[test]
fn lookup_finds_words_across_user_files() {
    let dir = tempfile::tempdir().unwrap();
    copy_fixture(
        "rime-crane-xhup/xhup.user.dict.yaml",
        dir.path(),
        "xhup.user.dict.yaml",
    );
    copy_fixture(
        "rime-crane-flypy/custom_phrase_double.txt",
        dir.path(),
        "custom_phrase_double.txt",
    );

    let files = sources::scan(dir.path(), dir.path(), &["xhup".to_owned()]);
    assert_eq!(files.len(), 2, "应扫到两个文件");
    let loaded = sources::load_all(&files);
    assert!(
        loaded.iter().all(|s| s.error.is_none()),
        "夹具应全部解析成功"
    );
    assert!(
        loaded
            .iter()
            .any(|s| s.schema_id.as_deref() == Some("xhup")),
        "应按文件名把词库归到方案"
    );

    let result = query(
        &loaded,
        &LookupQuery {
            mode: LookupMode::Text,
            keyword: "你好".into(),
            exact: true,
            ..Default::default()
        },
    );
    assert!(result.total >= 1, "应能按词查到编码");
    assert!(result.hits.iter().all(|h| h.text.contains("你好")));

    let by_code = query(
        &loaded,
        &LookupQuery {
            mode: LookupMode::Code,
            keyword: "nihc".into(),
            exact: true,
            ..Default::default()
        },
    );
    assert!(
        by_code.hits.iter().any(|h| h.code == "nihc"),
        "应能按编码查到词"
    );
}

#[test]
fn lookup_ignores_build_and_backup_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("build")).unwrap();
    std::fs::write(
        dir.path().join("build").join("x.dict.yaml"),
        "---\nname: x\n...\n\n坏\tbad\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("y.dict.yaml.bak"), "备份不是源文件").unwrap();
    std::fs::write(
        dir.path().join("ok.dict.yaml"),
        "---\nname: ok\n...\n\n好\thc\n",
    )
    .unwrap();

    let files = sources::scan(dir.path(), dir.path(), &[]);
    let names: Vec<String> = files
        .iter()
        .map(|f| f.path.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, vec!["ok.dict.yaml"]);
}

// ───────────────────────── 符号 ─────────────────────────

#[test]
fn symbols_resolve_how_to_type_a_punctuation() {
    let dir = tempfile::tempdir().unwrap();
    let shared = dir.path().join("shared");
    let user = dir.path().join("user");
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::create_dir_all(&user).unwrap();

    std::fs::write(
        shared.join("symbols.yaml"),
        "punctuator:\n  half_shape:\n    '<' : [ <, ≤ ]\n  full_shape:\n    '<' : [ 《, 〈 ]\n",
    )
    .unwrap();
    // 用户覆盖半角
    std::fs::write(
        user.join("symbols.yaml"),
        "punctuator:\n  half_shape:\n    '<' : [ ＜ ]\n",
    )
    .unwrap();

    let table = symbols::load(&shared, &user, &[]);
    let hits = symbols::search(&table, "《", 10);
    assert_eq!(hits.len(), 1, "按符号应能查到唯一触发键");
    assert_eq!(hits[0].key, "<");
    assert_eq!(
        hits[0].shape,
        rime_platform::symbols::SymbolShape::Full,
        "《 属于全角表"
    );
}

#[test]
fn symbol_search_by_key_lists_all_shapes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("symbols.yaml"),
        "punctuator:\n  half_shape:\n    ';' : [ ；, ： ]\n  full_shape:\n    ';' : { commit: ； }\n",
    )
    .unwrap();

    let table = symbols::load(dir.path(), dir.path(), &[]);
    let hits = symbols::search(&table, ";", 10);
    assert_eq!(hits.len(), 2, "半角与全角各一条");
    let commit = hits.iter().find(|h| h.mode == "commit").unwrap();
    assert_eq!(commit.symbols, vec!["；"]);
}

#[test]
fn sources_are_labelled_with_origin() {
    let dir = tempfile::tempdir().unwrap();
    let user = dir.path().join("user");
    let system = dir.path().join("system");
    std::fs::create_dir_all(&user).unwrap();
    std::fs::create_dir_all(&system).unwrap();
    std::fs::write(user.join("a.dict.yaml"), "---\nname: a\n...\n\n甲\tj\n").unwrap();
    std::fs::write(system.join("b.dict.yaml"), "---\nname: b\n...\n\n乙\ty\n").unwrap();

    let mut files = sources::scan(&user, &system, &[]);
    files.sort_by(|a, b| a.id.cmp(&b.id));
    let loaded = sources::load_all(&files);
    assert!(loaded.iter().any(|s| s.origin == SourceOrigin::User));
    assert!(loaded.iter().any(|s| s.origin == SourceOrigin::System));
}
