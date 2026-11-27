//! 编码建议引擎（PRD FR-3.23 / FR-2.6，设计定稿见 T0.4 报告 §6）。
//!
//! 流程：
//! 1. 文本分类：纯中文 / 纯英文 / 数字 / 混合；
//! 2. 纯英文：小写 / 大写 / 原样，短语去空格与连字符；
//! 3. 纯中文：字→拼音（多音字多读音）→ 经双拼映射得音节码 → 按方案类型合成词码；
//! 4. 混合：按段落切分后拼接；
//! 5. 全部候选标注来源（`kind`/`label`）与置信度，并附警告（阻断学习、超长、多音字…）。
//!
//! `CodeSuggestion::reading` 的约定：中文为带调读音（多音字/多候选据此区分），
//! 英文为「小写/大写/原样」说明，简拼候选追加「仅自定义短语可用」标注
//! （flypy 主翻译器 `abbrev` 已禁用，简码只能用于 custom_phrase）。

use crate::double_pinyin::DoublePinyin;
use crate::english::{self, EnglishCase};
use crate::pinyin::{is_han, BuiltinPinyin, PinyinReading, PinyinSource};
use crate::shape::{canonical_shape, ShapeSource};
use rime_core::types::{
    AlgebraSpec, CodeSuggestion, Confidence, FileRole, SchemeInfo, SuggestionKind,
    SuggestionWarning, TranslatorKind,
};
use std::collections::HashSet;

// 契约里约定的标注文字（docs/ipc.md §2：全拼/双拼/简拼/音形词码/英文/历史）。
const LABEL_FULL_PINYIN: &str = "全拼";
const LABEL_DOUBLE_PINYIN: &str = "双拼";
const LABEL_ABBREV: &str = "简拼";
const LABEL_SHAPE_WORD: &str = "音形词码";
const LABEL_ENGLISH: &str = "英文";
const LABEL_HISTORY: &str = "历史";
/// 中英混合（契约未列举，作为扩展标注）。
const LABEL_MIXED: &str = "混合";

/// 默认候选上限（避免多音字组合爆炸，UI 一屏可读）。
pub const DEFAULT_LIMIT: usize = 12;

/// 建议策略：由方案类型决定（FR-1.8 识别 → 本枚举）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestMode {
    /// 全拼方案：全拼 + 严格简拼（各音节首字母）
    FullPinyin,
    /// 双拼方案：双拼全码 + 首键简拼
    DoublePinyin,
    /// 音形方案（table）：C2 词码 + 单字「双拼+鹤形」
    ShapeWord,
}

impl SuggestMode {
    /// 由方案结构信息推断策略。
    pub fn from_scheme(kind: TranslatorKind, is_double_pinyin: bool) -> Self {
        match kind {
            TranslatorKind::Table | TranslatorKind::Mixed => SuggestMode::ShapeWord,
            TranslatorKind::Script if is_double_pinyin => SuggestMode::DoublePinyin,
            TranslatorKind::Script => SuggestMode::FullPinyin,
            TranslatorKind::Unknown if is_double_pinyin => SuggestMode::DoublePinyin,
            TranslatorKind::Unknown => SuggestMode::FullPinyin,
        }
    }
}

/// 生成建议的上下文（来自当前方案与受管文件）。
#[derive(Debug, Clone)]
pub struct SuggestOptions {
    pub mode: SuggestMode,
    /// 目标文件角色，仅用于「完整码会阻断自学习」判定（FR-3.22）
    pub role: Option<FileRole>,
    /// `speller/max_code_length`
    pub max_code_length: Option<u32>,
    /// 历史/已有码：优先展示（C5：用户自定义码要能被记住）
    pub known_codes: Vec<String>,
    /// 需要排除的既有编码（对应 IPC `suggest_codes.exclude_existing`）：
    /// 生成结果中剔除这些码，避免与词库已有词条重码。历史码不受影响。
    pub exclude: Vec<String>,
    /// 候选数量上限
    pub limit: usize,
}

impl Default for SuggestOptions {
    fn default() -> Self {
        SuggestOptions {
            mode: SuggestMode::DoublePinyin,
            role: None,
            max_code_length: None,
            known_codes: Vec::new(),
            exclude: Vec::new(),
            limit: DEFAULT_LIMIT,
        }
    }
}

impl SuggestOptions {
    pub fn new(mode: SuggestMode) -> Self {
        SuggestOptions {
            mode,
            ..Default::default()
        }
    }

    pub fn with_role(mut self, role: FileRole) -> Self {
        self.role = Some(role);
        self
    }

    pub fn with_max_code_length(mut self, max: u32) -> Self {
        self.max_code_length = Some(max);
        self
    }

    pub fn with_known_codes(mut self, codes: impl IntoIterator<Item = String>) -> Self {
        self.known_codes = codes.into_iter().collect();
        self
    }

    pub fn with_excluded_codes(mut self, codes: impl IntoIterator<Item = String>) -> Self {
        self.exclude = codes.into_iter().collect();
        self
    }

    /// 从方案结构信息初始化策略与最大码长（FR-1.8 识别结果 → 建议策略）。
    pub fn from_scheme(info: &SchemeInfo) -> Self {
        SuggestOptions {
            mode: SuggestMode::from_scheme(info.translator_kind, info.is_double_pinyin),
            max_code_length: info.max_code_length,
            ..SuggestOptions::default()
        }
    }
}

/// 完整码写入 `custom_phrase` 类文件会阻断自学习造词（FR-3.22 / 文件头注释明示）。
///
/// 判定辅助：**是否采纳由上层按文件角色决定**（本 crate 只负责算出这个结论）。
/// `code_len` 为候选码长度，`full_code_len` 为该词「完整码」长度
/// （各音节/各字完整双拼码之和）。只有达到完整码长度才算「完整编码」。
pub fn blocks_learning(role: Option<FileRole>, code_len: usize, full_code_len: usize) -> bool {
    matches!(role, Some(FileRole::CustomPhrase)) && full_code_len > 0 && code_len >= full_code_len
}

/// 编码建议引擎。
pub struct SuggestEngine {
    double_pinyin: DoublePinyin,
    pinyin: Box<dyn PinyinSource>,
    shapes: Option<Box<dyn ShapeSource>>,
}

impl std::fmt::Debug for SuggestEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SuggestEngine")
            .field("double_pinyin", &self.double_pinyin)
            .field("has_shapes", &self.shapes.is_some())
            .finish_non_exhaustive()
    }
}

impl SuggestEngine {
    pub fn new(double_pinyin: DoublePinyin, pinyin: Box<dyn PinyinSource>) -> Self {
        SuggestEngine {
            double_pinyin,
            pinyin,
            shapes: None,
        }
    }

    /// 双拼方案 + 内置常用字表（上层通常这样构造，再按需替换数据源）。
    pub fn for_double_pinyin(spec: Option<&AlgebraSpec>) -> Self {
        SuggestEngine::new(
            DoublePinyin::for_scheme(spec),
            Box::new(BuiltinPinyin::new()),
        )
    }

    /// 从方案结构信息构造（双拼映射来自 `speller/algebra`；拼音/形码数据源由上层注入）。
    pub fn from_scheme(info: &SchemeInfo) -> Self {
        SuggestEngine::for_double_pinyin(info.algebra.as_ref())
    }

    /// 注入形码反查数据源（音形方案单字建议用）。
    pub fn with_shapes(mut self, shapes: Box<dyn ShapeSource>) -> Self {
        self.shapes = Some(shapes);
        self
    }

    /// 替换拼音数据源（如注入 Unihan 或本地码表反查结果）。
    pub fn with_pinyin(mut self, pinyin: Box<dyn PinyinSource>) -> Self {
        self.pinyin = pinyin;
        self
    }

    pub fn double_pinyin(&self) -> &DoublePinyin {
        &self.double_pinyin
    }

    /// 生成编码建议。返回按优先级排序、已去重的候选列表。
    pub fn suggest(&self, text: &str, opts: &SuggestOptions) -> Vec<CodeSuggestion> {
        let text = text.trim();
        if text.is_empty() {
            return Vec::new();
        }
        let limit = opts.limit.max(1);

        let mut out: Vec<CodeSuggestion> = Vec::new();
        // 历史码优先（C5：自定义码不可推导，必须能被记住并优先展示）
        for code in &opts.known_codes {
            let code = code.trim();
            if code.is_empty() {
                continue;
            }
            out.push(CodeSuggestion {
                code: code.to_owned(),
                kind: SuggestionKind::History,
                label: LABEL_HISTORY.to_owned(),
                reading: None,
                confidence: Confidence::High,
                warnings: self.warnings_for(code, None, opts, true),
            });
        }

        out.extend(self.generate(text, opts));
        let mut seen = HashSet::new();
        out.retain(|suggestion| seen.insert(suggestion.code.clone()));
        if !opts.exclude.is_empty() {
            out.retain(|suggestion| {
                suggestion.kind == SuggestionKind::History
                    || !opts
                        .exclude
                        .iter()
                        .any(|code| code.trim() == suggestion.code)
            });
        }
        out.truncate(limit);
        out
    }

    fn generate(&self, text: &str, opts: &SuggestOptions) -> Vec<CodeSuggestion> {
        // 纯英文（含短语）先行处理：短语需要整串去空格/连字符
        if english::is_english_text(text) {
            return self.english_candidates(text, opts);
        }
        match segment(text) {
            Some(segments) if segments.is_empty() => Vec::new(),
            Some(mut segments) if segments.len() == 1 => match segments.remove(0) {
                Segment::Han(han) => self.chinese_candidates(&han, opts),
                Segment::Ascii(ascii) => self.ascii_candidates(&ascii, opts),
            },
            Some(segments) => self.mixed_candidates(&segments, opts),
            None => {
                tracing::debug!(text, "文本含不支持的字符（标点/emoji），不生成算法编码");
                Vec::new()
            }
        }
    }

    // ───────────────────────── 英文 / 数字 ─────────────────────────

    fn english_candidates(&self, text: &str, opts: &SuggestOptions) -> Vec<CodeSuggestion> {
        english::english_codes(text)
            .into_iter()
            .map(|(code, case)| {
                let confidence = match case {
                    EnglishCase::Lower => Confidence::High,
                    EnglishCase::AsIs => Confidence::Medium,
                    EnglishCase::Upper => Confidence::Low,
                };
                CodeSuggestion {
                    warnings: self.warnings_for(&code, None, opts, false),
                    code,
                    kind: SuggestionKind::English,
                    label: LABEL_ENGLISH.to_owned(),
                    reading: Some(case.label().to_owned()),
                    confidence,
                }
            })
            .collect()
    }

    fn ascii_candidates(&self, text: &str, opts: &SuggestOptions) -> Vec<CodeSuggestion> {
        if english::is_english_text(text) {
            return self.english_candidates(text, opts);
        }
        // 纯数字/字母数字混合：只能「原样」当自定义码（C5：手机号等的码不可推导）
        if text.chars().all(|c| c.is_ascii_alphanumeric()) {
            let mut warnings = self.warnings_for(text, None, opts, false);
            warnings.push(SuggestionWarning::UnsupportedCharset);
            return vec![CodeSuggestion {
                code: text.to_owned(),
                kind: SuggestionKind::English,
                label: LABEL_ENGLISH.to_owned(),
                reading: Some("原样".to_owned()),
                confidence: Confidence::Low,
                warnings,
            }];
        }
        Vec::new()
    }

    // ───────────────────────── 中文 ─────────────────────────

    fn chinese_candidates(&self, text: &str, opts: &SuggestOptions) -> Vec<CodeSuggestion> {
        let chars: Vec<char> = text.chars().collect();
        let mut readings: Vec<Vec<PinyinReading>> = Vec::with_capacity(chars.len());
        for ch in &chars {
            let list = self.pinyin.readings(*ch);
            if list.is_empty() {
                tracing::debug!(ch = %ch, "拼音数据源不认识该字，跳过建议");
                return Vec::new();
            }
            readings.push(list);
        }

        let limit = opts.limit.max(1);
        let mut out: Vec<CodeSuggestion> = Vec::new();
        for selection in selections(&readings, limit) {
            let syllables: Vec<&str> = selection
                .indices
                .iter()
                .enumerate()
                .map(|(index, choice)| readings[index][*choice].syllable.as_str())
                .collect();
            let reading_display = selection
                .indices
                .iter()
                .enumerate()
                .map(|(index, choice)| readings[index][*choice].display())
                .collect::<Vec<_>>()
                .join(" ");

            let confidence = match selection.divergence {
                0 => Confidence::High,
                1 => Confidence::Medium,
                _ => Confidence::Low,
            };
            for composed in self.build(chars[0], &chars, &syllables, opts) {
                let mut warnings =
                    self.warnings_for(&composed.code, Some(composed.full_len), opts, true);
                if selection.divergence > 0 {
                    warnings.push(SuggestionWarning::Polyphone);
                }
                let reading = match composed.note {
                    Some(note) => format!("{reading_display}（{note}）"),
                    None => reading_display.clone(),
                };
                out.push(CodeSuggestion {
                    code: composed.code,
                    kind: composed.kind,
                    label: composed.label.to_owned(),
                    reading: Some(reading),
                    confidence,
                    warnings,
                });
            }
        }
        dedupe_and_truncate(out, limit)
    }

    /// 按方案策略把一个读音序列合成为候选码。
    ///
    /// 音形词码规则（T0.4 报告 §3.1 / C2）：
    /// - 2 字：各字双拼**全码**拼接（`美团 meit uan → mwtr`）；
    /// - 3 字：各字双拼**首键**（`报关单 → bgd`）；
    /// - ≥4 字：前 3 字首键 + 末字首键（`狗咬吕洞宾 → gylb`）。
    ///
    /// 已知边界：小鹤音形码表另有「全码词」固定码（如 `进口商 jkuh` = 前两字首键
    /// + 末字全码），与上述规则不同且无法由读音推导（C5），只能由用户手动填写。
    fn build(
        &self,
        first_char: char,
        chars: &[char],
        syllables: &[&str],
        opts: &SuggestOptions,
    ) -> Vec<Composed> {
        let mut out = Vec::new();
        match opts.mode {
            SuggestMode::FullPinyin => {
                let full: String = syllables.concat();
                let full_len = full.chars().count();
                let initials: String = syllables.iter().filter_map(|s| s.chars().next()).collect();
                out.push(Composed::new(
                    full,
                    SuggestionKind::FullPinyin,
                    LABEL_FULL_PINYIN,
                    full_len,
                ));
                if initials.chars().count() > 0 {
                    out.push(Composed::with_note(
                        initials,
                        SuggestionKind::Abbrev,
                        LABEL_ABBREV,
                        full_len,
                    ));
                }
            }
            SuggestMode::DoublePinyin => {
                let Some(codes) = self.syllable_codes(syllables) else {
                    return out;
                };
                let full: String = codes.concat();
                let full_len = full.chars().count();
                let abbrev: String = codes.iter().filter_map(|c| c.chars().next()).collect();
                out.push(Composed::new(
                    full,
                    SuggestionKind::DoublePinyin,
                    LABEL_DOUBLE_PINYIN,
                    full_len,
                ));
                if !abbrev.is_empty() {
                    out.push(Composed::with_note(
                        abbrev,
                        SuggestionKind::Abbrev,
                        LABEL_ABBREV,
                        full_len,
                    ));
                }
            }
            SuggestMode::ShapeWord => {
                let Some(codes) = self.syllable_codes(syllables) else {
                    return out;
                };
                let full_len: usize = codes.iter().map(|c| c.chars().count()).sum();
                match chars.len() {
                    1 => {
                        let dp = codes[0].clone();
                        if let Some(shape) = self.shape_for(first_char, &dp) {
                            if !shape.is_empty() {
                                out.push(Composed::new(
                                    format!("{dp}{shape}"),
                                    SuggestionKind::ShapeChar,
                                    LABEL_SHAPE_WORD,
                                    full_len,
                                ));
                            }
                        }
                        // 无可用形码时也给出纯双拼码（用户可自行补形）
                        out.push(Composed::new(
                            dp,
                            SuggestionKind::DoublePinyin,
                            LABEL_DOUBLE_PINYIN,
                            full_len,
                        ));
                    }
                    2 => {
                        out.push(Composed::new(
                            codes.concat(),
                            SuggestionKind::ShapeWord,
                            LABEL_SHAPE_WORD,
                            full_len,
                        ));
                    }
                    3 => {
                        let code: String = codes.iter().filter_map(|c| c.chars().next()).collect();
                        out.push(Composed::new(
                            code,
                            SuggestionKind::ShapeWord,
                            LABEL_SHAPE_WORD,
                            full_len,
                        ));
                    }
                    n => {
                        let mut code: String = codes
                            .iter()
                            .take(3)
                            .filter_map(|c| c.chars().next())
                            .collect();
                        if let Some(last) = codes.get(n - 1).and_then(|c| c.chars().next()) {
                            code.push(last);
                        }
                        out.push(Composed::new(
                            code,
                            SuggestionKind::ShapeWord,
                            LABEL_SHAPE_WORD,
                            full_len,
                        ));
                    }
                }
            }
        }
        out
    }

    fn syllable_codes(&self, syllables: &[&str]) -> Option<Vec<String>> {
        syllables
            .iter()
            .map(|syllable| self.double_pinyin.code_for(syllable))
            .collect()
    }

    fn shape_for(&self, ch: char, dp_code: &str) -> Option<String> {
        let shapes = self.shapes.as_ref()?;
        canonical_shape(&shapes.entries(ch), dp_code)
    }

    // ───────────────────────── 混合 ─────────────────────────

    fn mixed_candidates(&self, segments: &[Segment], opts: &SuggestOptions) -> Vec<CodeSuggestion> {
        let mut parts: Vec<Vec<String>> = Vec::new();
        for segment in segments {
            match segment {
                Segment::Han(han) => {
                    let codes: Vec<String> = self
                        .chinese_candidates(han, opts)
                        .into_iter()
                        .take(2)
                        .map(|suggestion| suggestion.code)
                        .collect();
                    if codes.is_empty() {
                        return Vec::new();
                    }
                    parts.push(codes);
                }
                Segment::Ascii(ascii) => {
                    if english::is_english_text(ascii) {
                        let code = english::english_codes(ascii)
                            .into_iter()
                            .next()
                            .map(|(code, _)| code);
                        match code {
                            Some(code) => parts.push(vec![code]),
                            None => return Vec::new(),
                        }
                    } else {
                        parts.push(vec![ascii.clone()]);
                    }
                }
            }
        }

        let limit = opts.limit.max(1);
        let mut combos = vec![String::new()];
        for part in &parts {
            let mut next = Vec::new();
            'outer: for prefix in &combos {
                for code in part {
                    next.push(format!("{prefix}{code}"));
                    if next.len() >= limit {
                        break 'outer;
                    }
                }
            }
            combos = next;
        }

        let kind = match opts.mode {
            SuggestMode::FullPinyin => SuggestionKind::FullPinyin,
            SuggestMode::DoublePinyin => SuggestionKind::DoublePinyin,
            SuggestMode::ShapeWord => SuggestionKind::ShapeWord,
        };
        combos
            .into_iter()
            .map(|code| CodeSuggestion {
                warnings: self.warnings_for(&code, None, opts, true),
                code,
                kind,
                label: LABEL_MIXED.to_owned(),
                reading: None,
                confidence: Confidence::Medium,
            })
            .collect()
    }

    // ───────────────────────── 警告 ─────────────────────────

    /// `check_alphabet`：英文候选有意保留大写/原样，不能按双拼字母表判定字符集。
    fn warnings_for(
        &self,
        code: &str,
        full_len: Option<usize>,
        opts: &SuggestOptions,
        check_alphabet: bool,
    ) -> Vec<SuggestionWarning> {
        let mut warnings = Vec::new();
        let code_len = code.chars().count();
        if let Some(max) = opts.max_code_length {
            if code_len > max as usize {
                warnings.push(SuggestionWarning::ExceedsMaxCodeLength);
            }
        }
        if check_alphabet && !self.double_pinyin.invalid_chars(code).is_empty() {
            warnings.push(SuggestionWarning::UnsupportedCharset);
        }
        if let Some(full_len) = full_len {
            if blocks_learning(opts.role, code_len, full_len) {
                warnings.push(SuggestionWarning::BlocksLearning);
            }
        }
        warnings
    }
}

/// 合成出来的一个候选（尚未附加读音/警告）。
struct Composed {
    code: String,
    kind: SuggestionKind,
    label: &'static str,
    full_len: usize,
    note: Option<&'static str>,
}

impl Composed {
    fn new(code: String, kind: SuggestionKind, label: &'static str, full_len: usize) -> Self {
        Composed {
            code,
            kind,
            label,
            full_len,
            note: None,
        }
    }

    fn with_note(code: String, kind: SuggestionKind, label: &'static str, full_len: usize) -> Self {
        Composed {
            code,
            kind,
            label,
            full_len,
            note: Some("仅自定义短语可用"),
        }
    }
}

/// 文本分段（中文 / ASCII 连续段）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Han(String),
    Ascii(String),
}

/// 分段；遇到标点/emoji 等无法算法编码的字符返回 `None`。
fn segment(text: &str) -> Option<Vec<Segment>> {
    let mut segments: Vec<Segment> = Vec::new();
    for ch in text.chars() {
        if is_han(ch) {
            match segments.last_mut() {
                Some(Segment::Han(buffer)) => buffer.push(ch),
                _ => segments.push(Segment::Han(ch.to_string())),
            }
        } else if ch.is_ascii_alphanumeric() {
            match segments.last_mut() {
                Some(Segment::Ascii(buffer)) => buffer.push(ch),
                _ => segments.push(Segment::Ascii(ch.to_string())),
            }
        } else if ch.is_whitespace() {
            continue;
        } else {
            return None;
        }
    }
    Some(segments)
}

/// 读音组合选择：先全主读音，再一次只改一个字（多音字 → 多候选，C3）。
struct Selection {
    indices: Vec<usize>,
    divergence: usize,
}

fn selections(readings: &[Vec<PinyinReading>], limit: usize) -> Vec<Selection> {
    let mut out = vec![Selection {
        indices: vec![0; readings.len()],
        divergence: 0,
    }];
    if limit <= 1 {
        return out;
    }
    'outer: for (char_index, list) in readings.iter().enumerate() {
        for alternative in 1..list.len() {
            if out.len() >= limit {
                break 'outer;
            }
            let mut indices = vec![0usize; readings.len()];
            indices[char_index] = alternative;
            out.push(Selection {
                indices,
                divergence: 1,
            });
        }
    }
    out
}

fn dedupe_and_truncate(candidates: Vec<CodeSuggestion>, limit: usize) -> Vec<CodeSuggestion> {
    let mut seen = HashSet::new();
    let mut out: Vec<CodeSuggestion> = candidates
        .into_iter()
        .filter(|candidate| !candidate.code.is_empty() && seen.insert(candidate.code.clone()))
        .collect();
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::ShapeMap;

    fn engine() -> SuggestEngine {
        SuggestEngine::for_double_pinyin(None)
    }

    fn codes(suggestions: &[CodeSuggestion]) -> Vec<&str> {
        suggestions.iter().map(|s| s.code.as_str()).collect()
    }

    fn first(suggestions: &[CodeSuggestion]) -> &str {
        suggestions
            .first()
            .map(|s| s.code.as_str())
            .unwrap_or("<none>")
    }

    #[test]
    fn empty_text_yields_nothing() {
        assert!(engine().suggest("", &SuggestOptions::default()).is_empty());
        assert!(engine()
            .suggest("   ", &SuggestOptions::default())
            .is_empty());
    }

    /// 音形词码规则（C2）：2 字 = 各字双拼全码；3 字 = 各字首键；
    /// ≥4 字 = 前 3 字首键 + 末字首键。
    #[test]
    fn shape_word_codes_follow_c2() {
        let engine = engine();
        let opts = SuggestOptions::new(SuggestMode::ShapeWord);

        // 首选码（多音字会在其后追加变体，见 polyphonic_* 用例）
        assert_eq!(first(&engine.suggest("美团", &opts)), "mwtr");
        assert_eq!(first(&engine.suggest("报关单", &opts)), "bgd");
        assert_eq!(first(&engine.suggest("一会儿", &opts)), "yhe");
        assert_eq!(first(&engine.suggest("狗咬吕洞宾", &opts)), "gylb");
        assert_eq!(first(&engine.suggest("临时抱佛脚", &opts)), "lubj");
        assert_eq!(first(&engine.suggest("十年生死两茫茫", &opts)), "unum");
        assert_eq!(first(&engine.suggest("一个天上一个地下", &opts)), "ygtx");
        // 多音字「单」应追加 shan/chan 的变体，但首选仍是 bgd
        assert!(codes(&engine.suggest("报关单", &opts)).contains(&"bgu"));
    }

    /// 双拼方案：逐音节全码拼接 + 首键简拼。
    #[test]
    fn double_pinyin_codes() {
        let engine = engine();
        let opts = SuggestOptions::new(SuggestMode::DoublePinyin);
        assert_eq!(&codes(&engine.suggest("美团", &opts))[..2], ["mwtr", "mt"]);
        assert_eq!(
            &codes(&engine.suggest("报关单", &opts))[..2],
            ["bcgrdj", "bgd"]
        );
        let one = engine.suggest("中", &opts);
        assert_eq!(one[0].code, "vs");
        assert_eq!(one[0].kind, SuggestionKind::DoublePinyin);
        assert_eq!(one[0].reading.as_deref(), Some("zhōng"));
    }

    /// 多音字要出多候选并标注 Polyphone（C3）。
    #[test]
    fn polyphonic_words_produce_multiple_candidates() {
        let engine = engine();
        let opts = SuggestOptions::new(SuggestMode::ShapeWord);
        let suggestions = engine.suggest("骑行", &opts);
        // 主读音 xing → qixk，与用户实机码表一致
        assert_eq!(suggestions[0].code, "qixk");
        assert_eq!(suggestions[0].confidence, Confidence::High);
        assert!(!suggestions[0]
            .warnings
            .contains(&SuggestionWarning::Polyphone));
        // hang 变体
        let variant = suggestions
            .iter()
            .find(|s| s.code == "qihh")
            .expect("应有 hang 读音的变体 qihh");
        assert_eq!(variant.confidence, Confidence::Medium);
        assert!(variant.warnings.contains(&SuggestionWarning::Polyphone));
        assert_eq!(variant.reading.as_deref(), Some("qí háng"));

        // 地痞 → dipi（非 de）
        let suggestions = engine.suggest("地痞", &opts);
        assert_eq!(suggestions[0].code, "dipi");
        assert!(suggestions.iter().any(|s| s.code == "depi"));
    }

    #[test]
    fn full_pinyin_mode_outputs_full_pinyin_and_initials() {
        let engine = engine();
        let opts = SuggestOptions::new(SuggestMode::FullPinyin);
        let suggestions = engine.suggest("美团", &opts);
        assert_eq!(codes(&suggestions), vec!["meituan", "mt"]);
        assert_eq!(suggestions[0].kind, SuggestionKind::FullPinyin);
    }

    #[test]
    fn english_and_phrase() {
        let engine = engine();
        let opts = SuggestOptions::default();
        let suggestions = engine.suggest("Amazon", &opts);
        assert_eq!(codes(&suggestions), vec!["amazon", "Amazon", "AMAZON"]);
        assert_eq!(suggestions[0].kind, SuggestionKind::English);
        assert_eq!(suggestions[0].confidence, Confidence::High);

        let phrase = engine.suggest("hello world", &opts);
        assert_eq!(codes(&phrase), vec!["helloworld", "HELLOWORLD"]);
    }

    /// 混合文本按段落拼接（英文段 + 中文段）。
    #[test]
    fn mixed_text_is_composed_by_segments() {
        let engine = engine();
        let opts = SuggestOptions::new(SuggestMode::ShapeWord);
        let suggestions = engine.suggest("Go程", &opts);
        assert_eq!(suggestions[0].code, "goig", "go + 程(cheng→ig)");
    }

    #[test]
    fn unsupported_characters_yield_no_suggestion() {
        let engine = engine();
        assert!(engine
            .suggest("中，文", &SuggestOptions::default())
            .is_empty());
    }

    #[test]
    fn history_codes_come_first_and_are_deduped() {
        let engine = engine();
        let opts = SuggestOptions::new(SuggestMode::ShapeWord)
            .with_known_codes(vec!["pxm".to_string(), "bgd".to_string()]);
        let suggestions = engine.suggest("报关单", &opts);
        assert_eq!(&codes(&suggestions)[..2], ["pxm", "bgd"]);
        assert_eq!(suggestions[0].kind, SuggestionKind::History);
        assert_eq!(suggestions[0].label, "历史");
        // bgd 由历史码提供，生成结果中的同名码被去重
        assert_eq!(suggestions[1].kind, SuggestionKind::History);
    }

    #[test]
    fn warnings_exceed_max_code_length() {
        let engine = engine();
        let opts = SuggestOptions::new(SuggestMode::FullPinyin).with_max_code_length(4);
        let suggestions = engine.suggest("美团", &opts);
        assert!(suggestions[0]
            .warnings
            .contains(&SuggestionWarning::ExceedsMaxCodeLength));
    }

    /// 完整码写入 custom_phrase 会阻断自学习（FR-3.22）。
    #[test]
    fn blocks_learning_only_for_complete_codes_in_custom_phrase() {
        let engine = engine();

        // 双拼方案 + custom_phrase：完整双拼码 → 阻断
        let opts = SuggestOptions::new(SuggestMode::DoublePinyin).with_role(FileRole::CustomPhrase);
        let suggestions = engine.suggest("中", &opts);
        assert!(suggestions[0]
            .warnings
            .contains(&SuggestionWarning::BlocksLearning));

        // 简拼不阻断
        let abbrev = suggestions
            .iter()
            .find(|s| s.kind == SuggestionKind::Abbrev)
            .expect("应有简拼候选");
        assert!(!abbrev.warnings.contains(&SuggestionWarning::BlocksLearning));

        // 其他文件角色不阻断
        let opts = SuggestOptions::new(SuggestMode::DoublePinyin).with_role(FileRole::UserDict);
        let suggestions = engine.suggest("中", &opts);
        assert!(!suggestions[0]
            .warnings
            .contains(&SuggestionWarning::BlocksLearning));
    }

    #[test]
    fn blocks_learning_helper() {
        assert!(blocks_learning(Some(FileRole::CustomPhrase), 4, 4));
        assert!(blocks_learning(Some(FileRole::CustomPhrase), 5, 4));
        assert!(!blocks_learning(Some(FileRole::CustomPhrase), 3, 4));
        assert!(!blocks_learning(Some(FileRole::UserDict), 4, 4));
        assert!(!blocks_learning(None, 4, 4));
    }

    /// 音形单字：双拼 + 鹤形（形码从注入的码表反查，过滤万能码变体）。
    #[test]
    fn single_char_shape_uses_reverse_lookup() {
        let mut shapes = ShapeMap::new();
        shapes.insert('按', "anf");
        shapes.insert('按', "an`n"); // 万能码变体应被过滤
        shapes.insert('安', "an");
        let engine = engine().with_shapes(Box::new(shapes));
        let opts = SuggestOptions::new(SuggestMode::ShapeWord);

        let suggestions = engine.suggest("按", &opts);
        assert_eq!(suggestions[0].code, "anf");
        assert_eq!(suggestions[0].kind, SuggestionKind::ShapeChar);
        // 纯双拼码作为备选
        assert!(suggestions.iter().any(|s| s.code == "an"));

        let suggestions = engine.suggest("安", &opts);
        assert_eq!(suggestions[0].code, "an");
    }

    #[test]
    fn excluded_codes_are_filtered_but_history_is_kept() {
        let engine = engine();
        let opts = SuggestOptions::new(SuggestMode::ShapeWord)
            .with_known_codes(vec!["bgd".to_string()])
            .with_excluded_codes(vec!["bgd".to_string(), "bgu".to_string()]);
        let suggestions = engine.suggest("报关单", &opts);
        // 历史码是用户既有编码，仍展示；生成的 bgd/bgu 已被排除，其余多音字变体保留
        assert_eq!(codes(&suggestions), vec!["bgd", "bgi"]);
        assert_eq!(suggestions[0].kind, SuggestionKind::History);

        let opts = SuggestOptions::new(SuggestMode::ShapeWord)
            .with_excluded_codes(vec!["bgd".to_string()]);
        let suggestions = engine.suggest("报关单", &opts);
        assert!(!codes(&suggestions).contains(&"bgd"));
        assert_eq!(first(&suggestions), "bgu");
    }

    #[test]
    fn options_from_scheme_info() {
        let info = SchemeInfo {
            schema_id: "double_pinyin_flypy".to_owned(),
            name: "小鹤双拼".to_owned(),
            version: None,
            is_active: true,
            is_current: true,
            translator_kind: TranslatorKind::Script,
            alphabet: "abcdefghijklmnopqrstuvwxyz".to_owned(),
            max_code_length: Some(4),
            managed_files: Vec::new(),
            is_double_pinyin: true,
            algebra: None,
        };
        let opts = SuggestOptions::from_scheme(&info);
        assert_eq!(opts.mode, SuggestMode::DoublePinyin);
        assert_eq!(opts.max_code_length, Some(4));

        let engine = SuggestEngine::from_scheme(&info);
        assert_eq!(
            engine.double_pinyin().syllable_count(),
            crate::syllable::SYLLABLES.len()
        );
    }

    #[test]
    fn limit_is_respected() {
        let engine = engine();
        let mut opts = SuggestOptions::new(SuggestMode::ShapeWord);
        opts.limit = 1;
        assert_eq!(engine.suggest("骑行", &opts).len(), 1);
    }

    #[test]
    fn suggest_mode_from_scheme() {
        assert_eq!(
            SuggestMode::from_scheme(TranslatorKind::Script, true),
            SuggestMode::DoublePinyin
        );
        assert_eq!(
            SuggestMode::from_scheme(TranslatorKind::Script, false),
            SuggestMode::FullPinyin
        );
        assert_eq!(
            SuggestMode::from_scheme(TranslatorKind::Table, false),
            SuggestMode::ShapeWord
        );
    }

    #[test]
    fn segments_split_han_and_ascii() {
        assert_eq!(
            segment("Go程"),
            Some(vec![
                Segment::Ascii("Go".to_string()),
                Segment::Han("程".to_string())
            ])
        );
        assert_eq!(segment("中，文"), None);
        assert_eq!(
            segment("第3章"),
            Some(vec![
                Segment::Han("第".to_string()),
                Segment::Ascii("3".to_string()),
                Segment::Han("章".to_string())
            ])
        );
    }

    #[test]
    fn custom_pinyin_source_is_used() {
        // 注入一个只有「行 hang」的数据源，验证 trait 可替换
        let mut map: std::collections::HashMap<char, Vec<String>> = Default::default();
        map.insert('行', vec!["hang2".into()]);
        let engine = SuggestEngine::new(DoublePinyin::flypy(), Box::new(map));
        let opts = SuggestOptions::new(SuggestMode::ShapeWord);
        assert_eq!(codes(&engine.suggest("行", &opts)), vec!["hh"]);
    }
}
