//! 词条校验（PRD FR-3.11）：列数 / 字符集 / 码长 / 重复 / 权重 / 非法字符。
//!
//! 设计约定：
//! - **只读**：不修改 `ParsedDict`，校验结果供 UI 逐条定位。
//! - 每个问题都带 `line_no`（当前工作副本按序输出时的 1-based 行号）、`text`、
//!   `entry_code`，字段为 `Option` 但本模块总是填 `Some`（空串也填，便于 UI 显示）。
//! - `alphabet` 为空串表示「不限制字符集」（命令层找不到方案时的兜底）；
//!   此时仍拒绝编码中的空白字符（Rime 编码不允许空白）。
//! - 文件里被 `EntryLine::parse` 降级成 `Line::Raw` 的「坏数据行」（列数不对、
//!   文本/编码为空、权重不是非负整数）在这里重新识别并逐条报出，
//!   否则这些行会悄悄绕过校验。

use crate::types::{ColumnKind, ErrorCode, FileFormat, ValidationIssue};
use crate::{Line, ParsedDict};
use std::collections::hash_map::Entry as MapEntry;
use std::collections::{BTreeSet, HashMap, HashSet};

/// 校验一个（可能含暂存编辑的）词典。
///
/// 规则顺序与 FR-3.11 对齐：
/// 1. 文本 / 编码为空；
/// 2. 编码含 Tab / 换行（会破坏列结构，`FileFormatInvalid`）；
/// 3. 列数超出文件列风格（xhup 用户码表两列合法，第三列不允许）；
/// 4. 编码含 `speller/alphabet` 以外的字符（`CodeCharsetInvalid`）；
/// 5. 编码长度超过 `max_code_length`（`CodeTooLong`）；
/// 6. 文本+编码完全相同（`DuplicateEntry`，只在后出现的行上报）；
/// 7. 权重列存在但权重不是非负整数（`ValidationFailed`，来自坏数据行）。
pub fn validate_dict(
    dict: &ParsedDict,
    columns: &[ColumnKind],
    alphabet: &str,
    max_code_length: Option<u32>,
) -> Vec<ValidationIssue> {
    let allowed: Option<HashSet<char>> = if alphabet.is_empty() {
        None
    } else {
        Some(alphabet.chars().collect())
    };
    let allow_weight = columns.contains(&ColumnKind::Weight);
    let max_columns = columns.len();

    let mut issues: Vec<ValidationIssue> = Vec::new();
    let mut seen: HashMap<(String, String), u32> = HashMap::new();
    // 与 parse.rs 保持一致：`*.dict.yaml` 的 `...` 之后才是数据区
    let mut in_data = dict.format != FileFormat::RimeDictYaml;

    for (idx, line) in dict.lines.iter().enumerate() {
        let line_no = (idx + 1) as u32;
        match line {
            Line::Raw(raw) => {
                if !in_data {
                    if raw.trim() == "..." {
                        in_data = true;
                    }
                    continue;
                }
                // 空行、注释、单列行（如 rime_ice 里的 `3G`）都不是词条，跳过
                if raw.starts_with('#') || raw.trim().is_empty() || !raw.contains('\t') {
                    continue;
                }
                check_malformed_row(raw, line_no, max_columns, allow_weight, &mut issues);
            }
            Line::Entry(e) => {
                // 数据区之外不可能有 Entry（parse 的保证），防御一下
                if !in_data {
                    continue;
                }

                let text_empty = e.text.trim().is_empty();
                let code_empty = e.code.trim().is_empty();
                if text_empty {
                    issues.push(issue(
                        ErrorCode::ValidationFailed,
                        "文本不能为空",
                        line_no,
                        &e.text,
                        &e.code,
                    ));
                }
                if code_empty {
                    issues.push(issue(
                        ErrorCode::ValidationFailed,
                        "编码不能为空",
                        line_no,
                        &e.text,
                        &e.code,
                    ));
                }

                // Tab / 换行会破坏列结构：无论是否编辑过都必须拦下
                if contains_layout_breakers(&e.text) || contains_layout_breakers(&e.code) {
                    issues.push(issue(
                        ErrorCode::FileFormatInvalid,
                        "文本或编码含 Tab/换行，会破坏 Rime 文件的列结构",
                        line_no,
                        &e.text,
                        &e.code,
                    ));
                }

                // 列数检查只对「原样保留」的行有意义：已编辑行会按列风格重新格式化
                if !e.is_dirty() {
                    if let Some(raw) = line.raw_text() {
                        let fields = raw.split('\t').count();
                        if fields > max_columns {
                            issues.push(issue(
                                ErrorCode::ValidationFailed,
                                format!(
                                    "列数为 {fields}，超出该文件的列风格（最多 {max_columns} 列）"
                                ),
                                line_no,
                                &e.text,
                                &e.code,
                            ));
                        }
                    }
                }

                if !code_empty {
                    let bad = bad_code_chars(&e.code, allowed.as_ref());
                    if !bad.is_empty() {
                        issues.push(issue(
                            ErrorCode::CodeCharsetInvalid,
                            format!(
                                "编码含方案字母表以外的字符：{}",
                                bad.iter()
                                    .map(|c| format!("`{c}`"))
                                    .collect::<Vec<_>>()
                                    .join("、")
                            ),
                            line_no,
                            &e.text,
                            &e.code,
                        ));
                    }
                    if let Some(max) = max_code_length {
                        let len = e.code.chars().count() as u32;
                        if len > max {
                            issues.push(issue(
                                ErrorCode::CodeTooLong,
                                format!("编码长度为 {len}，超过方案上限 {max}"),
                                line_no,
                                &e.text,
                                &e.code,
                            ));
                        }
                    }
                }

                // 重复条目：同文本+同编码。首次出现记为基准，后续重复逐条上报。
                if !text_empty && !code_empty {
                    match seen.entry((e.text.clone(), e.code.clone())) {
                        MapEntry::Occupied(first) => issues.push(issue(
                            ErrorCode::DuplicateEntry,
                            format!("重复条目：与第 {} 行的文本+编码相同", first.get()),
                            line_no,
                            &e.text,
                            &e.code,
                        )),
                        MapEntry::Vacant(slot) => {
                            slot.insert(line_no);
                        }
                    }
                }
            }
        }
    }

    issues
}

fn issue(
    code: ErrorCode,
    message: impl Into<String>,
    line_no: u32,
    text: &str,
    entry_code: &str,
) -> ValidationIssue {
    ValidationIssue {
        code,
        message: message.into(),
        line_no: Some(line_no),
        text: Some(text.to_owned()),
        entry_code: Some(entry_code.to_owned()),
    }
}

fn contains_layout_breakers(s: &str) -> bool {
    s.contains('\t') || s.contains('\n') || s.contains('\r')
}

/// 编码中不属于字母表（或 alphabet 为空时的空白字符）的字符，去重且稳定排序。
fn bad_code_chars(code: &str, allowed: Option<&HashSet<char>>) -> BTreeSet<char> {
    let mut bad = BTreeSet::new();
    for c in code.chars() {
        match allowed {
            // 编码里不允许空白，即使字母表配置漏写了空白
            Some(_) if c.is_whitespace() => {
                bad.insert(c);
            }
            Some(a) if !a.contains(&c) => {
                bad.insert(c);
            }
            // alphabet 为空 = 不限制字符集（命令层兜底），仅拦空白
            None if c.is_whitespace() => {
                bad.insert(c);
            }
            _ => {}
        }
    }
    bad
}

/// 检查被降级成 Raw 的坏数据行。
///
/// `EntryLine::parse` 会把这些行当 Raw 原样保留，若不在这里报出来，用户会在
/// 「保存成功」后得到一个有问题的文件。这里只报告能被确定的字段问题。
fn check_malformed_row(
    raw: &str,
    line_no: u32,
    max_columns: usize,
    allow_weight: bool,
    issues: &mut Vec<ValidationIssue>,
) {
    let fields: Vec<&str> = raw.split('\t').collect();
    let text = fields.first().copied().unwrap_or("");
    let code = fields.get(1).copied().unwrap_or("");

    if fields.len() > max_columns {
        issues.push(issue(
            ErrorCode::ValidationFailed,
            format!(
                "列数为 {}，超出该文件的列风格（最多 {max_columns} 列）",
                fields.len()
            ),
            line_no,
            text,
            code,
        ));
    }
    if text.trim().is_empty() {
        issues.push(issue(
            ErrorCode::ValidationFailed,
            "文本不能为空",
            line_no,
            text,
            code,
        ));
    }
    if fields.len() >= 2 && code.trim().is_empty() {
        issues.push(issue(
            ErrorCode::ValidationFailed,
            "编码不能为空",
            line_no,
            text,
            code,
        ));
    }
    if allow_weight && fields.len() >= 3 {
        let weight = fields[2].trim();
        if !weight.is_empty() && weight.parse::<u32>().is_err() {
            issues.push(issue(
                ErrorCode::ValidationFailed,
                format!("权重必须为非负整数，当前为 `{weight}`"),
                line_no,
                text,
                code,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FileFormat;

    const COLS3: [ColumnKind; 3] = [ColumnKind::Text, ColumnKind::Code, ColumnKind::Weight];
    const COLS2: [ColumnKind; 2] = [ColumnKind::Text, ColumnKind::Code];

    fn parse(src: &str) -> ParsedDict {
        ParsedDict::parse(src, FileFormat::CustomPhrase).unwrap()
    }

    #[test]
    fn clean_two_column_dict_has_no_issues() {
        let d = parse("一个\tig\n美团\tmwtr\n");
        let issues = validate_dict(&d, &COLS2, "abcdefghijklmnopqrstuvwxyz;'", Some(4));
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn flags_bad_weight_that_parse_downgraded_to_raw() {
        let d = parse("一个\tig\n美团\tmwtr\tabc\n");
        let issues = validate_dict(&d, &COLS3, "abcdefghijklmnopqrstuvwxyz", None);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, ErrorCode::ValidationFailed);
        assert_eq!(issues[0].line_no, Some(2));
        assert!(issues[0].message.contains("权重"));
    }

    #[test]
    fn flags_third_column_in_two_column_file() {
        let d = parse("一个\tig\n美团\tmwtr\t100\n");
        let issues = validate_dict(&d, &COLS2, "abcdefghijklmnopqrstuvwxyz", None);
        assert!(issues.iter().any(|i| i.code == ErrorCode::ValidationFailed
            && i.message.contains("列")));
    }

    #[test]
    fn empty_alphabet_still_rejects_whitespace() {
        let d = parse("一个\tig x\n");
        let issues = validate_dict(&d, &COLS2, "", None);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, ErrorCode::CodeCharsetInvalid);
    }

    #[test]
    fn duplicate_is_reported_on_later_line_only() {
        let d = parse("一个\tig\n一个\tig\n一个\tig\n");
        let issues = validate_dict(&d, &COLS2, "abcdefghijklmnopqrstuvwxyz", None);
        let dups: Vec<_> = issues
            .iter()
            .filter(|i| i.code == ErrorCode::DuplicateEntry)
            .collect();
        assert_eq!(dups.len(), 2);
        assert_eq!(dups[0].line_no, Some(2));
        assert_eq!(dups[1].line_no, Some(3));
        assert!(dups[0].message.contains("第 1 行"));
    }

    #[test]
    fn semicolon_and_quote_are_legal_in_xhup_alphabet() {
        let d = parse("测试\ta;'\n");
        let issues = validate_dict(&d, &COLS2, "abcdefghijklmnopqrstuvwxyz;'", Some(4));
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn code_too_long_and_charset_are_separate_issues() {
        let d = parse("测试\tabcd-\n");
        let issues = validate_dict(&d, &COLS2, "abcdefghijklmnopqrstuvwxyz", Some(4));
        assert!(issues.iter().any(|i| i.code == ErrorCode::CodeTooLong));
        assert!(issues.iter().any(|i| i.code == ErrorCode::CodeCharsetInvalid));
    }
}