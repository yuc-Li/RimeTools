//! 夹具回归：用真实凇鹤方案（用户实机配置的脱敏副本）验证引擎。
//!
//! 夹具来源：`tests/fixtures/rime-crane-flypy` / `tests/fixtures/rime-crane-xhup`
//! （用户实机 `~/.local/share/fcitx5/rime` 的脱敏副本，见 PRD §2.4）。
//!
//! 这些用例是 T0.4 报告结论 C1/C2/C3/C5 的直接回归：
//! - C1：双拼映射可由 `speller/algebra` 运行时求值，且与内置键位表逐音节一致；
//! - C2：音形词码规则用真实词条（美团 mwtr、报关单 bgd、一会儿 yhe…）回归；
//! - C3：多音字要出多候选，能命中码表里「非首读音」的既有码（重置 isvi）；
//! - C5：用户自定义码（张三 pxm）不可推导，引擎不会给出该码。

use rime_core::types::AlgebraSpec;
use rime_suggest::{
    BuiltinPinyin, DoublePinyin, PinyinSource, ShapeMap, SuggestEngine, SuggestMode,
    SuggestOptions, SYLLABLES,
};
use std::path::{Path, PathBuf};

fn fixture(rel: &str) -> PathBuf {
    // CARGO_MANIFEST_DIR = src-tauri/crates/rime-suggest
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("定位仓库根失败")
        .to_path_buf();
    root.join("tests/fixtures").join(rel)
}

fn read_fixture(rel: &str) -> String {
    let path = fixture(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读取夹具失败 {}: {e}", path.display()))
}

/// 从真实方案 YAML 中抽出 `speller/algebra` 与 `speller/alphabet`
/// （只做行级抽取，不引入 YAML 依赖）。
fn flypy_algebra_spec() -> AlgebraSpec {
    let text = read_fixture("rime-crane-flypy/double_pinyin_flypy.schema.yaml");
    let mut rules = Vec::new();
    let mut alphabet = String::new();
    let mut in_algebra = false;
    let mut algebra_indent = 0usize;

    for line in text.lines() {
        let line = line.trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();

        if let Some(rest) = trimmed.strip_prefix("algebra:") {
            in_algebra = true;
            algebra_indent = indent;
            // 支持行内写法（`algebra: [...]`），flypy 用块写法，这里只保证不误判
            if rest.trim().starts_with('[') {
                in_algebra = false;
            }
            continue;
        }
        if trimmed.starts_with("alphabet:") {
            let value = trimmed.split_once(':').map(|(_, v)| v).unwrap_or("").trim();
            alphabet = value.trim_matches(|c| c == '"' || c == '\'').to_owned();
            continue;
        }
        if !in_algebra {
            continue;
        }
        if indent <= algebra_indent {
            in_algebra = false;
            continue;
        }
        if let Some(rule) = trimmed.strip_prefix("- ") {
            rules.push(rule.trim().to_owned());
        }
    }

    assert!(!rules.is_empty(), "未从方案中抽取到 speller/algebra");
    assert!(!alphabet.is_empty(), "未从方案中抽取到 speller/alphabet");
    AlgebraSpec { rules, alphabet }
}

fn engine() -> SuggestEngine {
    SuggestEngine::for_double_pinyin(None)
}

fn codes(suggestions: &[rime_core::types::CodeSuggestion]) -> Vec<String> {
    suggestions.iter().map(|s| s.code.clone()).collect()
}

/// C1：真实 flypy algebra 求值出的关键音节必须与报告 §2.2/§2.3 一致。
#[test]
fn real_flypy_algebra_matches_verified_key_syllables() {
    let spec = flypy_algebra_spec();
    assert!(spec.rules.len() >= 20, "规则条数异常：{}", spec.rules.len());
    assert!(
        spec.rules.iter().any(|r| r.starts_with("xlit/ⓆⓌ")),
        "flypy 应含圈字母 xlit 规则"
    );

    let dp = DoublePinyin::from_spec(&spec).expect("algebra 求值失败");
    assert_eq!(dp.syllable_count(), SYLLABLES.len(), "音节表应全部求值成功");

    let cases = [
        ("zhong", "vs"),
        ("mei", "mw"),
        ("tuan", "tr"),
        ("xing", "xk"),
        ("shuang", "ul"),
        ("nv", "nv"),
        ("lve", "lt"),
        ("ang", "ah"),
        ("an", "an"),
        ("a", "aa"),
        ("chong", "is"),
        ("zhou", "vz"),
        ("yun", "yy"),
        // 报告 §2.3 零声母
        ("ai", "ai"),
        ("ao", "ao"),
        ("er", "er"),
        ("eng", "eg"),
        ("o", "oo"),
    ];
    for (syllable, expected) in cases {
        assert_eq!(
            dp.code_for(syllable).as_deref(),
            Some(expected),
            "全拼 {syllable} 的双拼码应为 {expected}"
        );
    }
}

/// 规则顺序敏感的证据：若并行（一次性）匹配，`ang` 不会得到 `ah`。
#[test]
fn ang_is_order_sensitive_proof() {
    let spec = flypy_algebra_spec();
    let dp = DoublePinyin::from_spec(&spec).unwrap();
    assert_eq!(dp.code_for("ang").as_deref(), Some("ah"));
    // 中间态：`^([aoe])(ng)?$` 先把 ang 变成 aang，再由 `(.)ang$` 变成 aⓗ
    let algebra = rime_suggest::Algebra::parse_lenient(&spec.rules);
    let evaluation = algebra.evaluate("ang");
    assert_eq!(evaluation.primary.as_deref(), Some("ah"));
    // `an` 没有匹配首条规则，保持原样
    assert_eq!(algebra.evaluate("an").primary.as_deref(), Some("an"));
}

/// 运行时 algebra 求值与内置小鹤键位表必须逐音节一致（R6「对照官方键位图」）。
#[test]
fn algebra_evaluation_agrees_with_builtin_flypy_keymap() {
    let spec = flypy_algebra_spec();
    let from_algebra = DoublePinyin::from_spec(&spec).unwrap();
    let builtin = DoublePinyin::flypy();

    let mut mismatches = Vec::new();
    for syllable in SYLLABLES {
        let a = from_algebra.code_for(syllable);
        let b = builtin.code_for(syllable);
        if a != b {
            mismatches.push(format!("{syllable}: algebra={a:?} builtin={b:?}"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "algebra 与内置键位表不一致：{mismatches:#?}"
    );
}

/// 从码表夹具抽取「纯汉字 文本 + 字母 编码」的两列词条。
fn two_char_entries(rel: &str) -> Vec<(String, String)> {
    let text = read_fixture(rel);
    let mut entries = Vec::new();
    let mut in_header = false;
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        if line.trim() == "---" || line.trim() == "..." {
            in_header = !in_header;
            continue;
        }
        if in_header || !line.contains('\t') {
            continue;
        }
        let mut columns = line.split('\t');
        let (Some(word), Some(code)) = (columns.next(), columns.next()) else {
            continue;
        };
        let chars: Vec<char> = word.chars().collect();
        // 只回归「两字中文词 + 纯字母码」：3 字以上混有方案内固定全码变体，见下方说明。
        if chars.len() != 2
            || !chars.iter().all(|c| rime_suggest::is_han(*c))
            || code.is_empty()
            || !code.chars().all(|c| c.is_ascii_alphabetic())
        {
            continue;
        }
        entries.push((word.to_owned(), code.to_owned()));
    }
    assert!(!entries.is_empty(), "{rel} 未解析出两字词条");
    entries
}

/// C2 + C3：真实用户码表的两字词条必须能命中（含多音字非首读音的变体）。
///
/// 说明：三字及以上词条里混有方案内「全码词」固定码（如 `进口商 jkuh` = 前两字
/// 首键 + 末字全码），与本工具实现的 C2 规则（三字取各字首键）不同；那类码属于
/// 方案内不可推导的固定码，按 C5 由用户手动填写，不在回归范围内。
#[test]
fn real_user_dict_two_char_entries_are_reproduced() {
    let engine = engine();
    let opts = SuggestOptions::new(SuggestMode::ShapeWord);

    // C5：用户自定义任意码，不可推导（不参与「应能命中」回归，另见 custom_codes_are_not_derived）
    const CUSTOM_CODE_WORDS: &[&str] = &["张三"];

    let mut checked = 0;
    let mut failures = Vec::new();
    for rel in [
        "rime-crane-xhup/xhup.user.dict.yaml",
        "rime-crane-xhup/xhup.user.top.dict.yaml",
    ] {
        for (word, expected) in two_char_entries(rel) {
            if CUSTOM_CODE_WORDS.contains(&word.as_str()) {
                continue;
            }
            checked += 1;
            let suggestions = engine.suggest(&word, &opts);
            if !codes(&suggestions).contains(&expected) {
                failures.push(format!(
                    "{word} 期望 {expected}，实际 {:?}",
                    codes(&suggestions)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "词码回归失败：{failures:#?}");
    assert!(checked >= 30, "回归词条过少：{checked}");
}

/// 报告中点名的词条（含三字词/非首读音），逐条对照。
#[test]
fn named_entries_from_report() {
    let engine = engine();
    let opts = SuggestOptions::new(SuggestMode::ShapeWord);

    // 主读音即可命中的
    for (word, expected) in [
        ("美团", "mwtr"),
        ("报关单", "bgd"),
        ("一会儿", "yhe"),
        ("摸鱼", "moyu"),
        ("标识", "bnui"),
        ("精力", "jkli"),
        ("骑行", "qixk"),
    ] {
        assert_eq!(
            codes(&engine.suggest(word, &opts))
                .first()
                .map(String::as_str),
            Some(expected),
            "{word} 的首选码应为 {expected}"
        );
    }

    // 重置 = chóng zhì → isvi 属于多音字变体（C3）
    let reset = engine.suggest("重置", &opts);
    let variant = reset
        .iter()
        .find(|suggestion| suggestion.code == "isvi")
        .unwrap_or_else(|| panic!("应给出 重置/isvi 变体，实际 {:?}", codes(&reset)));
    assert!(variant
        .warnings
        .contains(&rime_core::types::SuggestionWarning::Polyphone));
}

/// C5：用户自定义码不可推导 —— 引擎不会「猜出」这些码。
#[test]
fn custom_codes_are_not_derived() {
    let engine = engine();
    let opts = SuggestOptions::new(SuggestMode::ShapeWord);

    // 张三 → pxm 是用户自定义码
    let zhang = engine.suggest("张三", &opts);
    assert!(
        !codes(&zhang).iter().any(|code| code == "pxm"),
        "{:?}",
        codes(&zhang)
    );
    assert_eq!(zhang[0].code, "vhsj", "张三 的可推导码应为 vhsj");

    // 数字串只能原样给出，不能推导出 puj/pufv
    for text in ["00000000000", "000000000000000000"] {
        let suggestions = engine.suggest(text, &opts);
        assert!(
            suggestions
                .iter()
                .all(|s| s.code != "puj" && s.code != "pufv"),
            "{text} 不应推导出自定义码"
        );
        assert_eq!(suggestions.first().map(|s| s.code.as_str()), Some(text));
    }
}

/// 形码反查：用真实主码表样本（xhup.primary.sample）验证单字建议，
/// 并确认 `an` 这类纯双拼条目也能给出建议。
#[test]
fn shape_lookup_uses_real_primary_dict_sample() {
    let text = read_fixture("rime-crane-xhup/xhup.primary.sample.dict.yaml");
    let shapes = ShapeMap::parse_dict(&text);
    assert!(shapes.len() > 30, "主码表样本条目过少：{}", shapes.len());

    let engine = engine().with_shapes(Box::new(shapes));
    let opts = SuggestOptions::new(SuggestMode::ShapeWord);

    for (word, expected) in [
        ("按", "anf"),
        ("安", "an"),
        ("阿", "aa"),
        ("岸", "ane"),
        ("癌", "aib"),
    ] {
        let suggestions = engine.suggest(word, &opts);
        assert_eq!(
            suggestions.first().map(|s| s.code.as_str()),
            Some(expected),
            "{word} 的单字音形码应为 {expected}，实际 {:?}",
            codes(&suggestions)
        );
    }
}

/// 拼音数据源可注入：trait 对象替换后引擎行为随之变化。
#[test]
fn pinyin_source_is_injectable() {
    let mut map: std::collections::HashMap<char, Vec<String>> = Default::default();
    map.insert('行', vec!["hang2".into(), "xing2".into()]);
    let engine = SuggestEngine::new(DoublePinyin::flypy(), Box::new(map));
    let opts = SuggestOptions::new(SuggestMode::ShapeWord);
    assert_eq!(
        codes(&engine.suggest("行", &opts))
            .first()
            .map(String::as_str),
        Some("hh"),
        "注入源的主读音是 hang（hang → h+ang=h），应得到 hh"
    );

    // 内置源的主读音是 xing（与用户码表一致）
    let builtin = BuiltinPinyin::new();
    assert_eq!(builtin.readings('行')[0].syllable, "xing");
    assert!(PinyinSource::contains(&builtin, '行'));
}
