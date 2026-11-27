//! 集成测试：`DictStore` 在真实凇鹤夹具上的打开 / 查询 / 排序 / 分页 /
//! 暂存 / 校验 / 序列化行为（task-5 验收场景）。
//!
//! 夹具是用户实机配置的脱敏副本，测试**只读**，绝不写入用户 Rime 目录。

use rime_core::types::{
    ColumnKind, Entry, EntryKey, EntryOrigin, EntryPage, EntryQuery, EntryUpdate, ErrorCode,
    FileFormat, FileRole, Fingerprint, NewEntry, SaveRequest, SortField, SortSpec, ValidationIssue,
};
use rime_core::{fingerprint_bytes, fingerprint_file, DictStore, ParsedDict, UNGROUPED};
use std::path::{Path, PathBuf};

fn fixture(rel: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("定位仓库根失败")
        .to_path_buf();
    root.join("tests/fixtures").join(rel)
}

fn flypy_path() -> PathBuf {
    fixture("rime-crane-flypy/custom_phrase_double.txt")
}

fn xhup_path() -> PathBuf {
    fixture("rime-crane-xhup/xhup.user.dict.yaml")
}

fn open_flypy() -> DictStore {
    DictStore::open(&flypy_path(), FileFormat::CustomPhrase, FileRole::CustomPhrase)
        .expect("打开小鹤双拼短语表失败")
}

fn open_xhup() -> DictStore {
    DictStore::open(&xhup_path(), FileFormat::RimeDictYaml, FileRole::UserDict)
        .expect("打开 xhup 用户码表失败")
}

/// 所有词条（测试用，忽略分页）。
fn all(store: &DictStore, sort: SortSpec) -> EntryPage {
    store.query(&EntryQuery {
        file_id: Some(store.file_id().clone()),
        keyword: None,
        group: None,
        sort,
        offset: 0,
        limit: 10_000,
    })
}

fn request(store: &DictStore) -> SaveRequest {
    SaveRequest {
        file_id: store.file_id().clone(),
        expected_fingerprint: store.fingerprint().clone(),
        add: Vec::new(),
        update: Vec::new(),
        delete: Vec::new(),
        reorder: None,
        deploy_after: false,
    }
}

fn key(store: &DictStore, text: &str, code: &str) -> EntryKey {
    EntryKey {
        file_id: store.file_id().clone(),
        text: text.to_owned(),
        code: code.to_owned(),
    }
}

fn find<'a>(items: &'a [Entry], text: &str, code: &str) -> &'a Entry {
    items
        .iter()
        .find(|e| e.key.text == text && e.key.code == code)
        .unwrap_or_else(|| panic!("未找到词条 {text}/{code}"))
}

fn codes(issues: &[ValidationIssue]) -> Vec<ErrorCode> {
    issues.iter().map(|i| i.code).collect()
}

// ───────────────────────── 打开 ─────────────────────────

#[test]
fn opens_flypy_with_correct_total_columns_and_fingerprint() {
    let path = flypy_path();
    let store = open_flypy();

    // 与解析器对同一文件的词条数一致（夹具为 36 条）
    let src = std::fs::read_to_string(&path).unwrap();
    let parsed = ParsedDict::parse(&src, FileFormat::CustomPhrase).unwrap();
    assert_eq!(store.total(), parsed.entry_count());
    assert_eq!(store.total(), 36);

    assert_eq!(
        store.columns(),
        &[ColumnKind::Text, ColumnKind::Code, ColumnKind::Weight]
    );
    assert_eq!(store.file_path(), path.as_path());
    assert_eq!(store.file_id(), &path.display().to_string());
    assert_eq!(store.format(), FileFormat::CustomPhrase);
    assert_eq!(store.fingerprint(), &fingerprint_file(&path).unwrap());
    assert!(!store.has_staged());
}

#[test]
fn opens_xhup_user_dict_as_two_columns_without_weight() {
    let store = open_xhup();
    assert_eq!(store.columns(), &[ColumnKind::Text, ColumnKind::Code]);
    assert_eq!(store.total(), 23);

    let page = all(&store, SortSpec::default());
    assert_eq!(page.total, 23);
    // 两列文件：weight 一律为 None
    for entry in &page.items {
        assert_eq!(entry.weight, None, "xhup 用户码表不应有权重：{}", entry.key.text);
        assert_eq!(entry.origin, EntryOrigin::File);
        assert!(entry.line_no.is_some());
    }
    // 已知词条
    assert_eq!(find(&page.items, "你好", "nihc").weight, None);
    assert_eq!(find(&page.items, "中国", "vsgo").weight, None);

    // 原样序列化逐字节一致（CRLF 保留）
    let src = std::fs::read_to_string(xhup_path()).unwrap();
    let (out, opts) = store.serialize().unwrap();
    assert_eq!(out, src);
    assert_eq!(opts.columns, vec![ColumnKind::Text, ColumnKind::Code]);
}

#[test]
fn open_bad_utf8_maps_to_corrupt_encoding() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.txt");
    std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
    let err = DictStore::open(&path, FileFormat::CustomPhrase, FileRole::CustomPhrase).unwrap_err();
    assert_eq!(err.code, ErrorCode::CorruptEncoding);
}

// ───────────────────────── 查询 / 搜索 ─────────────────────────

#[test]
fn keyword_matches_text_and_code_case_insensitively() {
    let store = open_flypy();

    for keyword in ["go", "GO", "Go"] {
        let page = store.query(&EntryQuery {
            keyword: Some(keyword.to_owned()),
            ..EntryQuery::default()
        });
        assert_eq!(page.total, 6, "关键字 {keyword} 应命中 6 条");
        for entry in &page.items {
            let text = entry.key.text.to_ascii_lowercase();
            let code = entry.key.code.to_ascii_lowercase();
            assert!(text.contains("go") || code.contains("go"));
        }
        // 文本命中：Golang / Hugo / Go程；编码命中：go / goto / gone
        assert!(page.items.iter().any(|e| e.key.text == "Golang"));
        assert!(page.items.iter().any(|e| e.key.text == "Go程"));
        assert!(page.items.iter().any(|e| e.key.code == "goto"));
    }

    // 空白关键字 = 不过滤
    let page = store.query(&EntryQuery {
        keyword: Some("   ".to_owned()),
        ..EntryQuery::default()
    });
    assert_eq!(page.total, 36);

    // 中文按文本命中
    let page = store.query(&EntryQuery {
        keyword: Some("有".to_owned()),
        ..EntryQuery::default()
    });
    assert!(page.total >= 5);
    assert!(page.items.iter().all(|e| e.key.text.contains('有')));

    // file_id 不匹配 → 空页（多文件索引由上层路由）
    let page = store.query(&EntryQuery {
        file_id: Some("别的文件".to_owned()),
        ..EntryQuery::default()
    });
    assert_eq!(page.total, 0);
    assert!(page.items.is_empty());
}

// ───────────────────────── 排序 ─────────────────────────

#[test]
fn sorts_by_text_code_and_weight() {
    let store = open_flypy();

    let by_text = all(
        &store,
        SortSpec { field: SortField::Text, desc: false },
    );
    assert_eq!(by_text.items[0].key.text, "Amazon");
    assert!(by_text
        .items
        .windows(2)
        .all(|w| w[0].key.text <= w[1].key.text));

    let by_text_desc = all(
        &store,
        SortSpec { field: SortField::Text, desc: true },
    );
    assert_eq!(by_text_desc.items[0].key.text, "由");
    assert!(by_text_desc
        .items
        .windows(2)
        .all(|w| w[0].key.text >= w[1].key.text));

    let by_code = all(
        &store,
        SortSpec { field: SortField::Code, desc: false },
    );
    assert_eq!(by_code.items[0].key.code, "amazon");
    assert!(by_code
        .items
        .windows(2)
        .all(|w| w[0].key.code <= w[1].key.code));

    let by_weight = all(
        &store,
        SortSpec { field: SortField::Weight, desc: false },
    );
    assert!(by_weight
        .items
        .windows(2)
        .all(|w| w[0].weight.unwrap_or(0) <= w[1].weight.unwrap_or(0)));
    assert_eq!(by_weight.items[0].weight, None, "无权重按 0 排在最前");

    let by_weight_desc = all(
        &store,
        SortSpec { field: SortField::Weight, desc: true },
    );
    assert_eq!(by_weight_desc.items[0].key.text, "一");
    assert_eq!(by_weight_desc.items[0].weight, Some(4));

    // 行号模式：严格等于文件行号顺序
    let by_line = all(
        &store,
        SortSpec { field: SortField::LineNo, desc: false },
    );
    assert!(by_line
        .items
        .windows(2)
        .all(|w| w[0].line_no.unwrap() < w[1].line_no.unwrap()));
}

// ───────────────────────── 分页 ─────────────────────────

#[test]
fn pagination_offset_limit_and_total_semantics() {
    let store = open_flypy();
    let page1 = store.query(&EntryQuery {
        offset: 0,
        limit: 10,
        ..EntryQuery::default()
    });
    let page2 = store.query(&EntryQuery {
        offset: 10,
        limit: 10,
        ..EntryQuery::default()
    });
    let page4 = store.query(&EntryQuery {
        offset: 30,
        limit: 10,
        ..EntryQuery::default()
    });
    let beyond = store.query(&EntryQuery {
        offset: 100,
        limit: 10,
        ..EntryQuery::default()
    });
    let zero_limit = store.query(&EntryQuery {
        offset: 0,
        limit: 0,
        ..EntryQuery::default()
    });

    assert_eq!(page1.total, 36, "total 是过滤后总数，与分页无关");
    assert_eq!(page2.total, 36);
    assert_eq!(page4.total, 36);
    assert_eq!(beyond.total, 36);
    assert_eq!(page1.items.len(), 10);
    assert_eq!(page2.items.len(), 10);
    assert_eq!(page4.items.len(), 6);
    assert!(beyond.items.is_empty());
    assert!(zero_limit.items.is_empty(), "limit=0 返回空页");
    assert_eq!(zero_limit.total, 36);

    // 相邻页无重复、无遗漏（默认行号排序）
    let mut seen = std::collections::HashSet::new();
    for entry in page1.items.iter().chain(page2.items.iter()) {
        assert!(seen.insert(entry.key.clone()), "分页出现重复词条");
    }
    assert_eq!(seen.len(), 20);
}

// ───────────────────────── 暂存与保真 ─────────────────────────

#[test]
fn stage_add_update_delete_keeps_untouched_bytes() {
    let store = open_flypy();
    let baseline = std::fs::read_to_string(flypy_path()).unwrap();

    let mut store = store;
    let mut req = request(&store);
    req.update.push(EntryUpdate {
        key: key(&store, "有", "u"),
        new_text: None,
        new_code: None,
        weight: Some(9),
        group: None,
    });
    req.delete.push(key(&store, "NASA", "nasa"));
    req.add.push(NewEntry {
        text: "测试词".to_owned(),
        code: "ceshi".to_owned(),
        weight: Some(5),
        group: None,
    });

    store.stage(&req).unwrap();
    assert!(store.has_staged());
    assert_eq!(store.total(), 36, "删 1 增 1，总数不变");

    let (out, opts) = store.serialize().unwrap();
    assert_eq!(opts.columns.len(), 3);

    // 期望：仅目标行变化，其余字节原样
    let expected = baseline
        .replace("有\tu\t3\n", "有\tu\t9\n")
        .replace("NASA\tnasa\n", "");
    let expected = format!("{expected}测试词\tceshi\t5\n");
    assert_eq!(out, expected, "未编辑行必须逐字节保留");

    // 暂存态查询：来源与权重正确
    let page = all(&store, SortSpec::default());
    assert_eq!(page.total, 36);
    assert!(page.items.iter().all(|e| e.key.text != "NASA"));
    assert_eq!(find(&page.items, "有", "u").weight, Some(9));
    assert_eq!(find(&page.items, "有", "u").origin, EntryOrigin::UserEdited);
    let added = find(&page.items, "测试词", "ceshi");
    assert_eq!(added.origin, EntryOrigin::UserAdded);
    assert_eq!(added.weight, Some(5));
    // 新增行的行号 = 序列化后的实际行号
    assert_eq!(added.line_no, Some(out.lines().count() as u32));
}

#[test]
fn stage_is_idempotent_for_the_same_request() {
    let mut store = open_flypy();
    let baseline = std::fs::read_to_string(flypy_path()).unwrap();

    let mut req = request(&store);
    req.update.push(EntryUpdate {
        key: key(&store, "有", "u"),
        new_text: None,
        new_code: None,
        weight: Some(9),
        group: None,
    });
    req.add.push(NewEntry {
        text: "测试词".to_owned(),
        code: "ceshi".to_owned(),
        weight: None,
        group: None,
    });

    store.stage(&req).unwrap();
    let first = store.serialize().unwrap().0;
    let first_total = store.total();

    store.stage(&req).unwrap();
    store.stage(&req).unwrap();
    assert_eq!(store.total(), first_total, "重复 stage 不得叠加条目");
    assert_eq!(store.serialize().unwrap().0, first);
    // 未编辑的行保持原样，且目标行按列风格重排
    assert!(first.contains("一个\tig\n"));
    assert!(first.contains("有\tu\t9\n"));
    assert_eq!(first, baseline.replace("有\tu\t3\n", "有\tu\t9\n") + "测试词\tceshi\t0\n");
}

#[test]
fn group_only_edit_changes_metadata_but_not_bytes() {
    let mut store = open_flypy();
    let baseline = std::fs::read_to_string(flypy_path()).unwrap();

    let mut req = request(&store);
    req.update.push(EntryUpdate {
        key: key(&store, "有", "u"),
        new_text: None,
        new_code: None,
        weight: None,
        group: Some("收藏".to_owned()),
    });
    req.update.push(EntryUpdate {
        key: key(&store, "一", "i"),
        new_text: None,
        new_code: None,
        weight: None,
        group: Some("常用".to_owned()),
    });
    store.stage(&req).unwrap();

    let (out, _) = store.serialize().unwrap();
    assert_eq!(out, baseline, "分组是工具元数据，绝不能写入 Rime 文件");

    // 分组筛选
    let hit = store.query(&EntryQuery {
        group: Some("收藏".to_owned()),
        ..EntryQuery::default()
    });
    assert_eq!(hit.total, 1);
    assert_eq!(hit.items[0].key.text, "有");
    assert_eq!(hit.items[0].group.as_deref(), Some("收藏"));

    let ungrouped = store.query(&EntryQuery {
        group: Some(UNGROUPED.to_owned()),
        ..EntryQuery::default()
    });
    assert_eq!(ungrouped.total, 34);
    // 未知分组 → 空
    let none = store.query(&EntryQuery {
        group: Some("无此分组".to_owned()),
        ..EntryQuery::default()
    });
    assert_eq!(none.total, 0);

    // 分组计数：「未分组」在前
    let counts = store.group_counts();
    assert_eq!(counts[0].name, UNGROUPED);
    assert_eq!(counts[0].count, 34);
    assert_eq!(counts[1].name, "常用", "其余分组按名称升序");
    assert_eq!(counts[1].count, 1);
    assert_eq!(counts[2].name, "收藏");
    assert_eq!(counts[2].count, 1);
    // Query 的 groups 也带计数（关键词过滤后、分组过滤前）
    assert_eq!(hit.groups.iter().map(|g| g.count).sum::<u32>(), 36);
}

#[test]
fn stage_reorder_moves_entries_between_existing_slots() {
    let mut store = open_flypy();
    let baseline = std::fs::read_to_string(flypy_path()).unwrap();

    let mut req = request(&store);
    req.reorder = Some(vec![key(&store, "有", "u"), key(&store, "一", "i")]);
    store.stage(&req).unwrap();

    let (out, _) = store.serialize().unwrap();
    let entries: Vec<&str> = out
        .lines()
        .filter(|line| line.contains('\t') && !line.starts_with('#'))
        .collect();
    assert_eq!(entries.len(), 36);
    assert_eq!(entries[0], "有\tu\t3", "重排只改变词条行顺序，行内容原样保留");
    assert_eq!(entries[1], "一\ti\t4");
    assert_eq!(entries[2], "噷\thm", "未列出的词条按原顺序补在后面");
    assert_eq!(out.lines().count(), baseline.lines().count(), "重排不增删行");

    // 行号随新顺序更新，查询按行号排序与序列化一致
    let page = all(&store, SortSpec::default());
    assert_eq!(page.items[0].key.text, "有");
    assert_eq!(page.items[1].key.text, "一");
    assert_eq!(page.items[2].key.text, "噷");

    // 幂等
    store.stage(&req).unwrap();
    assert_eq!(store.serialize().unwrap().0, out);

    // 未知键 → NotFound，且不改变当前暂存状态
    let mut bad = request(&store);
    bad.reorder = Some(vec![key(&store, "不存在的词", "xyz")]);
    assert_eq!(store.stage(&bad).unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(store.serialize().unwrap().0, out);
}

#[test]
fn stage_add_to_xhup_stays_two_columns_and_keeps_crlf() {
    let mut store = open_xhup();
    let baseline = std::fs::read_to_string(xhup_path()).unwrap();

    let mut req = request(&store);
    req.add.push(NewEntry {
        text: "新词".to_owned(),
        // 两列文件：即使传了权重也必须被忽略
        weight: Some(99),
        code: "xinc".to_owned(),
        group: None,
    });
    store.stage(&req).unwrap();

    let (out, opts) = store.serialize().unwrap();
    assert_eq!(opts.columns, vec![ColumnKind::Text, ColumnKind::Code]);
    assert_eq!(
        out,
        format!("{baseline}新词\txinc\r\n"),
        "两列文件不得出现权重列，且必须保留 CRLF"
    );

    let page = all(&store, SortSpec::default());
    let added = find(&page.items, "新词", "xinc");
    assert_eq!(added.weight, None);
    assert_eq!(added.origin, EntryOrigin::UserAdded);
}

#[test]
fn cloned_probe_does_not_pollute_the_real_store() {
    // 对应命令层 `dict_validate`：在 clone 上 stage + validate，不能影响已打开的真实状态
    let mut store = open_flypy();
    let mut req = request(&store);
    req.add.push(NewEntry {
        text: "探针".to_owned(),
        code: "tanzhen".to_owned(),
        weight: None,
        group: None,
    });

    let mut probe = store.clone();
    probe.stage(&req).unwrap();
    assert_eq!(probe.total(), 37);
    let flypy_alphabet = "zyxwvutsrqponmlkjihgfedcbaZYXWVUTSRQPONMLKJIHGFEDCBA`";
    assert!(probe.validate(flypy_alphabet, None).is_empty());

    assert_eq!(store.total(), 36, "probe 的暂存不得渗入真实 store");
    assert!(!store.has_staged());

    // 真实保存路径：stage → validate → serialize
    store.stage(&req).unwrap();
    assert!(store.validate(flypy_alphabet, None).is_empty());
    let (content, _) = store.serialize().unwrap();
    assert!(content.ends_with("探针\ttanzhen\t0\n"));
    // probe 与真实 store 相互独立：丢弃 probe 的暂存不影响真实 store
    assert_eq!(probe.total(), 37);
    probe.discard_staged();
    assert_eq!(probe.total(), 36);
    assert_ne!(probe.serialize().unwrap().0, content);
    assert_eq!(store.total(), 37);
}

#[test]
fn discard_staged_returns_to_disk_baseline() {
    let mut store = open_flypy();
    let baseline = std::fs::read_to_string(flypy_path()).unwrap();

    let mut req = request(&store);
    req.delete.push(key(&store, "NASA", "nasa"));
    store.stage(&req).unwrap();
    assert!(store.has_staged());
    assert_eq!(store.total(), 35);

    store.discard_staged();
    assert!(!store.has_staged());
    assert_eq!(store.total(), 36);
    assert_eq!(store.serialize().unwrap().0, baseline);
}

#[test]
fn mark_saved_promotes_working_copy_to_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("custom_phrase_double.txt");
    std::fs::copy(flypy_path(), &path).unwrap();
    let mut store =
        DictStore::open(&path, FileFormat::CustomPhrase, FileRole::CustomPhrase).unwrap();

    let mut req = request(&store);
    req.update.push(EntryUpdate {
        key: key(&store, "有", "u"),
        new_text: None,
        new_code: None,
        weight: Some(9),
        group: None,
    });
    req.add.push(NewEntry {
        text: "测试词".to_owned(),
        code: "ceshi".to_owned(),
        weight: None,
        group: None,
    });
    store.stage(&req).unwrap();
    let (content, _) = store.serialize().unwrap();

    // 模拟 rime-safety 写盘成功：内容落盘 + mark_saved 刷新指纹
    std::fs::write(&path, &content).unwrap();
    let new_fingerprint = fingerprint_file(&path).unwrap();
    store.mark_saved(new_fingerprint.clone());

    assert!(!store.has_staged());
    assert_eq!(store.fingerprint(), &new_fingerprint);
    assert_eq!(store.total(), 37, "基线 36 + 新增 1");
    // 固化后序列化仍逐字节一致（不会被旧 raw 覆盖）
    assert_eq!(store.serialize().unwrap().0, content);
    // 全部来源于文件
    let page = all(&store, SortSpec::default());
    assert!(page.items.iter().all(|e| e.origin == EntryOrigin::File));
    assert_eq!(find(&page.items, "有", "u").weight, Some(9));

    // 保存后再次暂存：从新基线出发
    let mut req = request(&store);
    req.delete.push(key(&store, "测试词", "ceshi"));
    store.stage(&req).unwrap();
    let (after_delete, _) = store.serialize().unwrap();
    assert_eq!(after_delete, content.replace("测试词\tceshi\t0\n", ""));
}

// ───────────────────────── 校验 ─────────────────────────

#[test]
fn validates_clean_fixtures_without_issues() {
    let flypy = open_flypy();
    let xhup = open_xhup();
    let flypy_alphabet = "zyxwvutsrqponmlkjihgfedcbaZYXWVUTSRQPONMLKJIHGFEDCBA`";
    let xhup_alphabet = "abcdefghijklmnopqrstuvwxyz;'";

    assert!(flypy.validate(flypy_alphabet, None).is_empty());
    assert!(xhup.validate(xhup_alphabet, Some(4)).is_empty());
}

#[test]
fn validation_reports_charset_length_and_duplicate_with_line_numbers() {
    let mut store = open_xhup();
    let alphabet = "abcdefghijklmnopqrstuvwxyz;'";

    let mut req = request(&store);
    // 第 36 行：编码含非法字符 `-`
    req.add.push(NewEntry {
        text: "测试甲".to_owned(),
        code: "ab-c".to_owned(),
        weight: None,
        group: None,
    });
    // 第 37 行：5 键，超过 max_code_length=4
    req.add.push(NewEntry {
        text: "测试乙".to_owned(),
        code: "abcde".to_owned(),
        weight: None,
        group: None,
    });
    // 第 38 行：与夹具第 15 行的 你好/nihc 重复
    req.add.push(NewEntry {
        text: "你好".to_owned(),
        code: "nihc".to_owned(),
        weight: None,
        group: None,
    });
    // 第 39 行：`;` `'` 属于 xhup 字母表，合法
    req.add.push(NewEntry {
        text: "合法符号".to_owned(),
        code: "a;'".to_owned(),
        weight: None,
        group: None,
    });
    store.stage(&req).unwrap();

    let issues = store.validate(alphabet, Some(4));

    let charset = issues
        .iter()
        .find(|i| i.code == ErrorCode::CodeCharsetInvalid)
        .expect("应报 CodeCharsetInvalid");
    assert_eq!(charset.line_no, Some(36));
    assert_eq!(charset.text.as_deref(), Some("测试甲"));
    assert_eq!(charset.entry_code.as_deref(), Some("ab-c"));

    let too_long = issues
        .iter()
        .find(|i| i.code == ErrorCode::CodeTooLong)
        .expect("应报 CodeTooLong");
    assert_eq!(too_long.line_no, Some(37));
    assert_eq!(too_long.entry_code.as_deref(), Some("abcde"));

    let dup = issues
        .iter()
        .find(|i| i.code == ErrorCode::DuplicateEntry)
        .expect("应报 DuplicateEntry");
    assert_eq!(dup.line_no, Some(38));
    assert_eq!(dup.text.as_deref(), Some("你好"));
    assert!(dup.message.contains("第 15 行"), "{}", dup.message);

    // 合法符号条目不应产生任何问题
    assert!(!issues.iter().any(|i| i.text.as_deref() == Some("合法符号")));
    assert_eq!(codes(&issues).len(), 3);
}

#[test]
fn validation_flags_empty_fields_and_layout_breakers() {
    let mut store = open_xhup();

    let mut req = request(&store);
    req.add.push(NewEntry {
        text: String::new(),
        code: "abc".to_owned(),
        weight: None,
        group: None,
    });
    req.add.push(NewEntry {
        text: "坏\t词".to_owned(),
        code: "abc".to_owned(),
        weight: None,
        group: None,
    });
    store.stage(&req).unwrap();

    let issues = store.validate("abcdefghijklmnopqrstuvwxyz;'", Some(4));
    assert!(issues
        .iter()
        .any(|i| i.code == ErrorCode::ValidationFailed && i.text.as_deref() == Some("")));
    assert!(issues.iter().any(|i| i.code == ErrorCode::FileFormatInvalid));

    // 防御：带坏字段的工作副本拒绝序列化（正常流程会先被 validate 拦下）
    let err = store.serialize().unwrap_err();
    assert_eq!(err.code, ErrorCode::ValidationFailed);
}

#[test]
fn validation_with_empty_alphabet_skips_charset_check() {
    let mut store = open_xhup();
    let mut req = request(&store);
    req.add.push(NewEntry {
        text: "测试".to_owned(),
        code: "ab-c".to_owned(),
        weight: None,
        group: None,
    });
    store.stage(&req).unwrap();

    // 命令层找不到方案时传空串：不限制字符集
    assert!(store.validate("", None).is_empty());
}

#[test]
fn validation_flags_weight_that_parser_downgraded_to_raw() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("custom_phrase.txt");
    std::fs::write(&path, "一个\tig\n你好\tnihc\tabc\n").unwrap();
    let store =
        DictStore::open(&path, FileFormat::CustomPhrase, FileRole::CustomPhrase).unwrap();

    let issues = store.validate("abcdefghijklmnopqrstuvwxyz", None);
    let weight_issue = issues
        .iter()
        .find(|i| i.code == ErrorCode::ValidationFailed && i.message.contains("权重"))
        .expect("坏权重行应被校验报出（parse 会把它降级为 Raw）");
    assert_eq!(weight_issue.line_no, Some(2));
}

// ───────────────────────── 杂项 ─────────────────────────

#[test]
fn serialize_rejects_tab_in_entry_fields() {
    let mut store = open_xhup();
    let mut req = request(&store);
    req.add.push(NewEntry {
        text: "正常".to_owned(),
        code: "ab\tc".to_owned(),
        weight: None,
        group: None,
    });
    store.stage(&req).unwrap();
    let err = store.serialize().unwrap_err();
    assert_eq!(err.code, ErrorCode::FileFormatInvalid);
}

#[test]
fn staged_new_entry_without_weight_serializes_zero_in_three_column_file() {
    let mut store = open_flypy();
    let mut req = request(&store);
    req.add.push(NewEntry {
        text: "无权重".to_owned(),
        code: "wuquan".to_owned(),
        weight: None,
        group: None,
    });
    store.stage(&req).unwrap();
    let (out, _) = store.serialize().unwrap();
    // custom_phrase 为三列格式：权重缺省按 Rime 语义写 0
    assert!(out.ends_with("无权重\twuquan\t0\n"), "{out}");
}

#[test]
fn mark_saved_with_explicit_fingerprint_accepts_bytes_hash() {
    // `fingerprint_bytes` 与 `fingerprint_file` 对同一内容一致（供上层在内存写入后用）
    let store = open_flypy();
    let bytes = std::fs::read(flypy_path()).unwrap();
    let fp = Fingerprint {
        mtime_ms: 0,
        size: bytes.len() as u64,
        sha256: fingerprint_bytes(&bytes),
    };
    assert_eq!(fp.sha256, store.fingerprint().sha256);
}
/// 性能护栏（PRD AC：10 万条搜索 ≤ 200ms）。
///
/// 机器差异较大，默认 ignore；本地/发布前手动运行：
/// `cargo test -p rime-core --test store perf -- --ignored --nocapture`
#[test]
#[ignore = "性能护栏：手动运行，避免 CI 抖动"]
fn perf_query_100k_entries_stays_within_budget() {
    use std::time::Instant;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.lst");
    let mut content = String::from("# Rime table\n");
    for i in 0..100_000u32 {
        content.push_str(&format!("词{i}\tma{i}\t{i}\n"));
    }
    std::fs::write(&path, &content).unwrap();

    let open_start = Instant::now();
    let mut store =
        DictStore::open(&path, FileFormat::CustomPhrase, FileRole::CustomPhrase).unwrap();
    println!("10 万条打开耗时：{:?}", open_start.elapsed());
    assert_eq!(store.total(), 100_000);

    let search_start = Instant::now();
    let page = store.query(&EntryQuery {
        keyword: Some("词99999".to_owned()),
        ..EntryQuery::default()
    });
    let search = search_start.elapsed();
    println!("10 万条关键词查询：{search:?}，命中 {}", page.total);
    assert!(page.total >= 1);
    assert!(search.as_millis() < 200, "关键词查询超过 200ms 预算：{search:?}");

    let sort_start = Instant::now();
    let page = store.query(&EntryQuery {
        sort: SortSpec { field: SortField::Weight, desc: true },
        offset: 50,
        limit: 100,
        ..EntryQuery::default()
    });
    let sort = sort_start.elapsed();
    println!("10 万条排序 + 分页：{sort:?}，total {}", page.total);
    assert_eq!(page.items.len(), 100);
    assert!(sort.as_millis() < 200, "排序分页超过 200ms 预算：{sort:?}");

    let add_start = Instant::now();
    let mut req = request(&store);
    for _ in 0..100 {
        req.add.push(NewEntry {
            text: "新增".to_owned(),
            code: "xinzeng".to_owned(),
            weight: None,
            group: None,
        });
    }
    store.stage(&req).unwrap();
    println!("10 万条暂存 100 条新增：{:?}", add_start.elapsed());
    assert_eq!(store.total(), 100_100);

    // 批量删除 1000 条（PRD AC 场景）：单趟扫描，O(n)
    let delete_start = Instant::now();
    let mut req = request(&store);
    for i in 0..1000u32 {
        req.delete.push(EntryKey {
            file_id: store.file_id().clone(),
            text: format!("词{i}"),
            code: format!("ma{i}"),
        });
    }
    store.stage(&req).unwrap();
    let delete = delete_start.elapsed();
    println!("10 万条批量删除 1000 条：{delete:?}");
    // stage 是「相对基线的完整 diff」：上一请求的 100 条新增没有再次提交，因此被丢弃
    assert_eq!(store.total(), 99_000);
    assert!(delete.as_millis() < 500, "批量删除超过 500ms：{delete:?}");
}
