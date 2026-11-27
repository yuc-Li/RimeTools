//! 英文编码规则（FR-2.6 ⑤）：小写 / 大写 / 原样；英文短语去空格与连字符。
//!
//! 依据用户实机码表（`tests/fixtures/rime-crane-flypy/custom_phrase_double.txt`）：
//! `Amazon → amazon`、`SOHO → soho`、`Golang → golang`，即**小写是默认值**，
//! 原样/大写作为备选。

/// 英文（含英文短语）候选的三种大小写形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnglishCase {
    /// 全部小写（默认建议，与用户实机码表一致）
    Lower,
    /// 全部大写
    Upper,
    /// 保留原文大小写
    AsIs,
}

impl EnglishCase {
    /// 界面标注文字。
    pub fn label(self) -> &'static str {
        match self {
            EnglishCase::Lower => "小写",
            EnglishCase::Upper => "大写",
            EnglishCase::AsIs => "原样",
        }
    }
}

/// 是否为纯英文文本（字母，可含空格/连字符/撇号/下划线等英文短语分隔符）。
///
/// 数字不算英文（数字文本只能由用户自定义码，见 C5 结论）。
pub fn is_english_text(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && text.chars().any(|c| c.is_ascii_alphabetic())
        && text
            .chars()
            .all(|c| c.is_ascii_alphabetic() || matches!(c, ' ' | '\t' | '-' | '_' | '\'' | '.'))
}

/// 去掉空格与连字符（英文短语 → 连续编码）。
pub fn collapse(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, ' ' | '\t' | '-'))
        .collect()
}

/// 生成英文候选编码，**小写在前**，并按需附带原样/大写（去重）。
pub fn english_codes(text: &str) -> Vec<(String, EnglishCase)> {
    let collapsed = collapse(text.trim());
    if collapsed.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<(String, EnglishCase)> = Vec::new();
    let mut push = |code: String, case: EnglishCase| {
        if !out.iter().any(|(existing, _)| *existing == code) {
            out.push((code, case));
        }
    };
    push(collapsed.to_lowercase(), EnglishCase::Lower);
    push(collapsed.clone(), EnglishCase::AsIs);
    push(collapsed.to_uppercase(), EnglishCase::Upper);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_english_text() {
        assert!(is_english_text("Amazon"));
        assert!(is_english_text("hello world"));
        assert!(is_english_text("hello-world"));
        assert!(is_english_text("don't"));
        assert!(!is_english_text(""));
        assert!(!is_english_text("123"));
        assert!(!is_english_text("Go程"));
        assert!(!is_english_text("中文"));
        assert!(!is_english_text("a，b"));
    }

    #[test]
    fn collapses_spaces_and_hyphens() {
        assert_eq!(collapse("hello world"), "helloworld");
        assert_eq!(collapse("hello-world"), "helloworld");
        assert_eq!(collapse("  a b-c "), "abc");
        assert_eq!(collapse("don't"), "don't");
    }

    /// 用户实机码表的既有英文条目（Amazon→amazon、SOHO→soho）。
    #[test]
    fn lowercase_is_the_default_code() {
        assert_eq!(english_codes("Amazon")[0].0, "amazon");
        assert_eq!(english_codes("SOHO")[0].0, "soho");
        assert_eq!(english_codes("Golang")[0].0, "golang");
        assert_eq!(english_codes("Go")[0].0, "go");
    }

    #[test]
    fn offers_case_variants_without_duplicates() {
        let amazon = english_codes("Amazon");
        assert_eq!(
            amazon,
            vec![
                ("amazon".to_string(), EnglishCase::Lower),
                ("Amazon".to_string(), EnglishCase::AsIs),
                ("AMAZON".to_string(), EnglishCase::Upper),
            ]
        );
        // SOHO：大写与原样相同，只出两个
        let soho = english_codes("SOHO");
        assert_eq!(soho.len(), 2);
        assert_eq!(soho[0].0, "soho");
        assert_eq!(soho[1].0, "SOHO");

        let phrase = english_codes("hello world");
        assert_eq!(phrase[0].0, "helloworld");
        assert_eq!(phrase[1].0, "HELLOWORLD");
        assert_eq!(phrase.len(), 2);
    }

    #[test]
    fn empty_text_yields_no_codes() {
        assert!(english_codes("   ").is_empty());
        assert!(english_codes("-").is_empty());
    }
}
