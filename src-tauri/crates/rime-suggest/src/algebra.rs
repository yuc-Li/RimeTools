//! Rime `speller/algebra` 求值器。
//!
//! 设计依据：对真实方案 algebra 求值并与官方键位表逐一对照（见下方语义说明）。
//!
//! # 语义（与 librime 对齐的关键点）
//!
//! 1. **规则逐条顺序作用**，且每条规则对所有当前拼写生效。`xform` 用
//!    `regex::replace_all` 语义（自左向右、不重叠、**替换后从原串继续**），
//!    因此同一条规则可以「重复命中」——`ang →（^([aoe])(ng)?$）→ aang →
//!    （(.)ang$）→ aⓗ →（xlit）→ ah` 依赖这一顺序语义，**不能并行匹配**。
//! 2. `derive`/`abbrev` 只**追加**可选拼写（原拼写保留），追加的拼写会继续
//!    经过后续规则（与 librime 的 Script 集合语义一致）。主码只取非派生拼写。
//! 3. `erase` 删除匹配的拼写（按 `is_match`，不要求整串匹配，通常写 `^xx$`）。
//! 4. `xlit` 是逐字符平移（圈字母 `ⓆⓌ…` → `qwrty…`），不涉及正则。
//!
//! # 替换串展开
//!
//! Rime/boost 用 `$1` 表示分组，而 `regex` crate 会把 `$1v` 解析成「名为 `1v`
//! 的分组」（-> 空串）。flypy 里恰好有 `derive/^([jqxy])u$/$1v/`，所以这里
//! **不用** `regex` 自带的展开，而是自己解析 `$N` / `${N}` / `\N` / `$$`。

use regex::Regex;
use std::collections::HashSet;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AlgebraError {
    #[error("algebra 规则格式不合法（应为 `类型/模式/替换/`）：{0}")]
    Malformed(String),
    #[error("algebra 规则类型不支持：{0}")]
    UnsupportedRule(String),
    #[error("algebra 规则 `{rule}` 的正则非法：{message}")]
    InvalidRegex { rule: String, message: String },
    #[error("xlit 源字符串为空，或两侧长度不一致：{from:?} → {to:?}")]
    InvalidXlit { from: String, to: String },
}

/// 规则类型（Rime 支持的子集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind {
    /// 变换拼写（顺序、可重复命中）
    Xform,
    /// 派生额外拼写（保留原拼写）
    Derive,
    /// 删除拼写
    Erase,
    /// 逐字符平移
    Xlit,
    /// 简拼（追加一条缩写拼写）
    Abbrev,
    /// 模糊音（`fuzz`，行为等同 derive；flypy 未使用，为兼容其他方案保留）
    Fuzz,
}

impl RuleKind {
    fn parse(name: &str) -> Option<RuleKind> {
        Some(match name {
            "xform" => RuleKind::Xform,
            "derive" => RuleKind::Derive,
            "erase" => RuleKind::Erase,
            "xlit" => RuleKind::Xlit,
            "abbrev" => RuleKind::Abbrev,
            "fuzz" => RuleKind::Fuzz,
            _ => return None,
        })
    }

    /// 是否为「追加拼写」类规则（不影响主码）。
    pub fn is_additive(self) -> bool {
        matches!(self, RuleKind::Derive | RuleKind::Abbrev | RuleKind::Fuzz)
    }
}

/// 单条规则。
#[derive(Debug, Clone)]
pub struct Rule {
    kind: RuleKind,
    pattern: String,
    replacement: String,
    /// `Erase`/`Xlit` 不需要正则。
    regex: Option<Regex>,
    /// `Xlit` 的字符映射。
    xlit: Vec<(char, char)>,
    /// 原始规则串，便于回显与排错。
    raw: String,
}

impl Rule {
    pub fn parse(formula: &str) -> Result<Rule, AlgebraError> {
        let formula = formula.trim();
        let (kind_name, rest) = formula
            .split_once('/')
            .ok_or_else(|| AlgebraError::Malformed(formula.to_owned()))?;
        let kind = RuleKind::parse(kind_name)
            .ok_or_else(|| AlgebraError::UnsupportedRule(kind_name.to_owned()))?;
        let fields = split_fields(rest)?;
        // Rime 写法固定为 `kind/模式/替换/`（末尾斜杠）；容忍省略末尾斜杠。
        let (pattern, replacement) = match fields.as_slice() {
            [p, r] => (p.clone(), r.clone()),
            [p, r, tail] if tail.is_empty() => (p.clone(), r.clone()),
            _ => return Err(AlgebraError::Malformed(formula.to_owned())),
        };

        let mut rule = Rule {
            kind,
            pattern,
            replacement,
            regex: None,
            xlit: Vec::new(),
            raw: formula.to_owned(),
        };
        match kind {
            RuleKind::Xlit => {
                let from: Vec<char> = rule.pattern.chars().collect();
                let to: Vec<char> = rule.replacement.chars().collect();
                if from.is_empty() {
                    return Err(AlgebraError::InvalidXlit {
                        from: rule.pattern.clone(),
                        to: rule.replacement.clone(),
                    });
                }
                // librime 要求两侧等长；这里保持严格，避免静默丢字符。
                if from.len() != to.len() {
                    return Err(AlgebraError::InvalidXlit {
                        from: rule.pattern.clone(),
                        to: rule.replacement.clone(),
                    });
                }
                rule.xlit = from.into_iter().zip(to).collect();
            }
            RuleKind::Erase => {
                rule.regex = Some(compile(&rule.pattern, formula)?);
            }
            _ => {
                rule.regex = Some(compile(&rule.pattern, formula)?);
            }
        }
        Ok(rule)
    }

    pub fn kind(&self) -> RuleKind {
        self.kind
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    pub fn replacement(&self) -> &str {
        &self.replacement
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// `xform`/`derive`/`abbrev`/`fuzz` 的替换（全部命中、自左向右）。
    fn replace_all(&self, input: &str) -> String {
        let Some(regex) = &self.regex else {
            return input.to_owned();
        };
        if !regex.is_match(input) {
            return input.to_owned();
        }
        regex
            .replace_all(input, |caps: &regex::Captures<'_>| {
                expand_replacement(&self.replacement, caps)
            })
            .into_owned()
    }

    /// `xlit` 的逐字符平移。
    fn translate(&self, input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        for ch in input.chars() {
            match self.xlit.iter().find(|(from, _)| *from == ch) {
                Some((_, to)) => out.push(*to),
                None => out.push(ch),
            }
        }
        out
    }

    fn is_match(&self, input: &str) -> bool {
        self.regex.as_ref().is_some_and(|re| re.is_match(input))
    }
}

fn compile(pattern: &str, raw: &str) -> Result<Regex, AlgebraError> {
    Regex::new(pattern).map_err(|e| AlgebraError::InvalidRegex {
        rule: raw.to_owned(),
        message: e.to_string(),
    })
}

/// 按未转义的 `/` 切分（`\/` 表示字面斜杠）。Rime 规则里斜杠是分隔符。
fn split_fields(rest: &str) -> Result<Vec<String>, AlgebraError> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for ch in rest.chars() {
        if escaped {
            // 只把 `\/` 还原成 `/`；其余反斜杠原样保留（正则里要用）。
            if ch == '/' {
                current.push('/');
            } else {
                current.push('\\');
                current.push(ch);
            }
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '/' => {
                fields.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    if escaped {
        current.push('\\');
    }
    // 末尾字段即使为空也要保留：`erase/^xx$/` 的替换字段就是空串。
    fields.push(current);
    if fields.len() < 2 {
        return Err(AlgebraError::Malformed(rest.to_owned()));
    }
    Ok(fields)
}

/// 展开替换串中的分组引用：`$1`、`${1}`、`\1`，`$$` → 字面 `$`。
/// 与 boost（librime 所用）一致：未参与匹配的分组展开为空串。
fn expand_replacement(template: &str, caps: &regex::Captures<'_>) -> String {
    let chars: Vec<char> = template.chars().collect();
    let mut out = String::with_capacity(template.len());
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        match ch {
            '$' if i + 1 < chars.len() && chars[i + 1] == '$' => {
                out.push('$');
                i += 2;
            }
            '$' if i + 1 < chars.len() && chars[i + 1] == '{' => {
                // ${N} / ${name}
                let mut j = i + 2;
                let mut name = String::new();
                while j < chars.len() && chars[j] != '}' {
                    name.push(chars[j]);
                    j += 1;
                }
                if j < chars.len() {
                    push_group(&mut out, caps, &name);
                    i = j + 1;
                } else {
                    out.push('$');
                    i += 1;
                }
            }
            '$' if i + 1 < chars.len() && chars[i + 1].is_ascii_digit() => {
                let (name, end) = read_group_ref(&chars, i + 1);
                push_group(&mut out, caps, &name);
                i = end;
            }
            '\\' if i + 1 < chars.len() && chars[i + 1].is_ascii_digit() => {
                let (name, end) = read_group_ref(&chars, i + 1);
                push_group(&mut out, caps, &name);
                i = end;
            }
            _ => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

/// 读取 `$` 后的分组号，返回（分组名, 分组号之后的绝对下标）。
fn read_group_ref(chars: &[char], start: usize) -> (String, usize) {
    let mut j = start;
    while j < chars.len() && chars[j].is_ascii_digit() {
        j += 1;
    }
    let name: String = chars[start..j].iter().collect();
    (name, j)
}

fn push_group(out: &mut String, caps: &regex::Captures<'_>, name: &str) {
    if let Ok(index) = name.parse::<usize>() {
        if let Some(m) = caps.get(index) {
            out.push_str(m.as_str());
        }
        return;
    }
    if let Some(m) = caps.name(name) {
        out.push_str(m.as_str());
    }
}

/// 拼写来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpellingOrigin {
    /// 主拼写（`xform`/`xlit` 链的结果）
    Primary,
    /// `derive`/`fuzz` 追加的可选拼写
    Derived,
    /// `abbrev` 追加的简拼
    Abbrev,
}

/// 求值过程中的一个拼写。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spelling {
    pub code: String,
    pub origin: SpellingOrigin,
}

impl Spelling {
    pub fn new(code: impl Into<String>, origin: SpellingOrigin) -> Self {
        Spelling {
            code: code.into(),
            origin,
        }
    }
}

/// 单个全拼拼写的求值结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Evaluation {
    /// 主码（被 `erase` 排除时为 `None`）
    pub primary: Option<String>,
    /// 派生出的可选拼写（不含主码，去重、保序）
    pub derived: Vec<String>,
    /// `abbrev` 产出的简拼（不含主码/派生）
    pub abbreviations: Vec<String>,
}

/// 一组 algebra 规则。
#[derive(Debug, Clone, Default)]
pub struct Algebra {
    rules: Vec<Rule>,
}

impl Algebra {
    /// 空规则集（无 algebra 的 table 方案）。
    pub fn empty() -> Self {
        Algebra { rules: Vec::new() }
    }

    /// 解析规则串列表（顺序即执行顺序）。
    pub fn parse<S: AsRef<str>>(rules: &[S]) -> Result<Self, AlgebraError> {
        let mut parsed = Vec::with_capacity(rules.len());
        for raw in rules {
            let raw = raw.as_ref();
            let formula = raw.trim();
            if formula.is_empty() {
                continue;
            }
            parsed.push(Rule::parse(formula)?);
        }
        Ok(Algebra { rules: parsed })
    }

    /// 解析并跳过无法识别的规则（记录 `tracing::warn`），用于容忍上游方案里的
    /// 未知扩展（如未来 librime 新增的计算类型），避免整条建议链失效。
    pub fn parse_lenient<S: AsRef<str>>(rules: &[S]) -> Self {
        let mut parsed = Vec::new();
        for raw in rules {
            match Rule::parse(raw.as_ref().trim()) {
                Ok(rule) => parsed.push(rule),
                Err(e) => {
                    tracing::warn!(rule = raw.as_ref(), error = %e, "忽略无法解析的 algebra 规则")
                }
            }
        }
        Algebra { rules: parsed }
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// 对单个全拼拼写求值（顺序执行全部规则）。
    pub fn evaluate(&self, input: &str) -> Evaluation {
        let mut set = vec![Spelling::new(input, SpellingOrigin::Primary)];
        self.apply(&mut set);
        let primary = set
            .iter()
            .find(|s| s.origin == SpellingOrigin::Primary)
            .map(|s| s.code.clone())
            .filter(|c| !c.is_empty());
        let derived = set
            .iter()
            .filter(|s| s.origin == SpellingOrigin::Derived)
            .map(|s| s.code.clone())
            .collect();
        let abbreviations = set
            .iter()
            .filter(|s| s.origin == SpellingOrigin::Abbrev)
            .map(|s| s.code.clone())
            .collect();
        Evaluation {
            primary,
            derived,
            abbreviations,
        }
    }

    /// 对拼写集合就地求值（供需要多拼写联动的场景使用）。
    pub fn apply(&self, set: &mut Vec<Spelling>) {
        for rule in &self.rules {
            match rule.kind {
                RuleKind::Xform => {
                    for s in set.iter_mut() {
                        s.code = rule.replace_all(&s.code);
                    }
                }
                RuleKind::Xlit => {
                    for s in set.iter_mut() {
                        s.code = rule.translate(&s.code);
                    }
                }
                RuleKind::Erase => {
                    set.retain(|s| !rule.is_match(&s.code));
                }
                RuleKind::Derive | RuleKind::Fuzz | RuleKind::Abbrev => {
                    let origin = if rule.kind == RuleKind::Abbrev {
                        SpellingOrigin::Abbrev
                    } else {
                        SpellingOrigin::Derived
                    };
                    let mut additions = Vec::new();
                    for s in set.iter() {
                        let candidate = rule.replace_all(&s.code);
                        if candidate != s.code && !candidate.is_empty() {
                            additions.push(Spelling::new(candidate, origin));
                        }
                    }
                    for addition in additions {
                        if !set.iter().any(|s| s.code == addition.code) {
                            set.push(addition);
                        }
                    }
                }
            }
            dedupe(set);
        }
    }
}

fn dedupe(set: &mut Vec<Spelling>) {
    let mut seen = HashSet::new();
    set.retain(|s| seen.insert(s.code.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rule_shapes() {
        let rule = Rule::parse("xform/(.)ei$/$1Ⓦ/").unwrap();
        assert_eq!(rule.kind(), RuleKind::Xform);
        assert_eq!(rule.pattern(), "(.)ei$");
        assert_eq!(rule.replacement(), "$1Ⓦ");

        let erase = Rule::parse("erase/^xx$/").unwrap();
        assert_eq!(erase.kind(), RuleKind::Erase);

        let xlit = Rule::parse("xlit/ⓆⓌ/qw/").unwrap();
        assert_eq!(xlit.kind(), RuleKind::Xlit);

        // 省略末尾斜杠也接受
        assert_eq!(
            Rule::parse("derive/^([jqxy])u$/$1v").unwrap().replacement(),
            "$1v"
        );
    }

    #[test]
    fn rejects_unsupported_rule_kind() {
        assert!(matches!(
            Rule::parse("bogus/a/b/"),
            Err(AlgebraError::UnsupportedRule(_))
        ));
        assert!(matches!(
            Rule::parse("no-slash"),
            Err(AlgebraError::Malformed(_))
        ));
    }

    /// flypy 里的 `$1v` 若交给 regex crate 自带展开会变成空串——必须自己展开。
    #[test]
    fn expands_numeric_group_refs_like_boost() {
        let algebra = Algebra::parse(&["derive/^([jqxy])u$/$1v/"]).unwrap();
        let ev = algebra.evaluate("ju");
        assert_eq!(ev.primary.as_deref(), Some("ju"));
        assert_eq!(ev.derived, vec!["jv".to_string()]);
    }

    #[test]
    fn expansion_handles_literal_dollar_and_missing_group() {
        let re = Regex::new("(a)(b)?").unwrap();
        let caps = re.captures("a").unwrap();
        assert_eq!(expand_replacement("$1$2", &caps), "a");
        assert_eq!(expand_replacement("$$1", &caps), "$1");
        assert_eq!(expand_replacement("${1}x", &caps), "ax");
        assert_eq!(expand_replacement("\\1x", &caps), "ax");
    }

    #[test]
    fn erase_removes_primary() {
        let algebra = Algebra::parse(&["erase/^xx$/"]).unwrap();
        assert_eq!(algebra.evaluate("xx").primary, None);
        assert_eq!(algebra.evaluate("xing").primary.as_deref(), Some("xing"));
    }

    #[test]
    fn xlit_translates_chars() {
        let algebra = Algebra::parse(&["xlit/ⓆⓌ/qw/"]).unwrap();
        assert_eq!(algebra.evaluate("ⓆⓌx").primary.as_deref(), Some("qwx"));
        // 未映射字符原样保留
        assert_eq!(algebra.evaluate("中文Ⓠ").primary.as_deref(), Some("中文q"));
    }

    /// 规则必须顺序、可重复命中：同一串被两条 xform 先后改写。
    #[test]
    fn xform_rules_chain_sequentially() {
        let algebra = Algebra::parse(&[
            "xform/^([aoe])(ng)?$/$1$1$2/",
            "xform/(.)ang$/$1ⓗ/",
            "xlit/ⓗ/h/",
        ])
        .unwrap();
        let ev = algebra.evaluate("ang");
        assert_eq!(ev.primary.as_deref(), Some("ah"));
        // 若并行匹配（一次只取一条规则）则不会是 ah
        assert_eq!(algebra.evaluate("an").primary.as_deref(), Some("an"));
    }

    /// `xform` 的 replace_all 语义：同一规则可命中多处。
    #[test]
    fn xform_replaces_all_occurrences() {
        let algebra = Algebra::parse(&["xform/a/A/"]).unwrap();
        assert_eq!(algebra.evaluate("aaab").primary.as_deref(), Some("AAAb"));
    }

    #[test]
    fn lenient_parse_skips_unknown_rules() {
        let algebra = Algebra::parse_lenient(&["xform/a/b/", "wat/a/b/"]);
        assert_eq!(algebra.len(), 1);
    }
}
