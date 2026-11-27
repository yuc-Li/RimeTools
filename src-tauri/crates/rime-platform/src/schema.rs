//! Rime 方案（`*.schema.yaml`）解析。
//!
//! 只读取本工具需要的结构信息，不修改、不重写方案文件。

use rime_core::types::{AlgebraSpec, TranslatorKind};
use serde_yaml::Value;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum SchemaParseError {
    #[error("读取方案文件失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("YAML 解析失败：{0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("方案文件缺少 schema/schema_id")]
    MissingSchemaId,
}

/// 从方案文件抽取的结构信息。
#[derive(Debug, Clone)]
pub struct SchemaRaw {
    pub schema_id: String,
    pub name: String,
    pub version: Option<String>,
    pub translator_kind: TranslatorKind,
    /// `speller/alphabet`
    pub alphabet: String,
    /// `speller/max_code_length`
    pub max_code_length: Option<u32>,
    /// 主翻译器挂载的词典名（`translator/dictionary`）
    pub primary_dictionary: Option<String>,
    /// `table_translator@custom_phrase` 所挂载的 user_dict（不含扩展名）
    pub custom_phrase_user_dict: Option<String>,
    /// `speller/algebra`
    pub algebra: Option<AlgebraSpec>,
    /// 是否为双拼方案
    pub is_double_pinyin: bool,
}

pub fn parse_schema(path: &Path) -> Result<SchemaRaw, SchemaParseError> {
    let text = std::fs::read_to_string(path)?;
    parse_schema_str(&text)
}

pub fn parse_schema_str(text: &str) -> Result<SchemaRaw, SchemaParseError> {
    let value: Value = serde_yaml::from_str(text)?;

    let schema_id = value
        .get("schema")
        .and_then(|s| s.get("schema_id"))
        .and_then(|v| v.as_str())
        .ok_or(SchemaParseError::MissingSchemaId)?
        .to_owned();

    let name = value
        .get("schema")
        .and_then(|s| s.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or(&schema_id)
        .to_owned();

    let version = value
        .get("schema")
        .and_then(|s| s.get("version"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_owned());

    // ── 翻译器类型 ──
    //
    // 注意：方案里常同时出现 `script_translator` 与多个 `table_translator@xxx`
    // （custom_phrase / 英文 / 反查等辅助翻译器）。只有**不带 `@` 后缀**的那个才是
    // 主翻译器，方案性质由它决定。按「有 table 就算 Mixed」会误判小鹤双拼为 Mixed。
    let translators = value
        .get("engine")
        .and_then(|e| e.get("translators"))
        .and_then(|t| t.as_sequence())
        .cloned()
        .unwrap_or_default();
    let names: Vec<&str> = translators
        .iter()
        .filter_map(|t| t.as_str().map(|s| s.trim()))
        .collect();

    let primary = names.iter().find(|s| {
        !s.contains('@')
            && (s.starts_with("script_translator") || s.starts_with("table_translator"))
    });

    let translator_kind = match primary {
        Some(s) if s.starts_with("script_translator") => TranslatorKind::Script,
        Some(_) => TranslatorKind::Table,
        None => TranslatorKind::Unknown,
    };

    // ── custom_phrase 挂载 ──
    let has_custom_phrase_translator = names.iter().any(|s| *s == "table_translator@custom_phrase");
    let custom_phrase_user_dict = if has_custom_phrase_translator {
        value
            .get("custom_phrase")
            .and_then(|c| c.get("user_dict"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned())
    } else {
        None
    };

    // ── 主词典 ──
    let primary_dictionary = value
        .get("translator")
        .and_then(|t| t.get("dictionary"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_owned());

    // ── speller ──
    let speller = value.get("speller");
    let alphabet = speller
        .and_then(|s| s.get("alphabet"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let max_code_length = speller
        .and_then(|s| s.get("max_code_length"))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    let algebra_rules = speller
        .and_then(|s| s.get("algebra"))
        .and_then(|a| a.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let algebra = if algebra_rules.is_empty() {
        None
    } else {
        Some(AlgebraSpec {
            rules: algebra_rules.clone(),
            alphabet: alphabet.clone(),
        })
    };

    // 双拼判定：algebra 中出现圈字母标记（flypy 用 ⓆⓌ… 做中间态）
    // 或方案带 preedit_format 的双拼展开规则。
    let circled = "ⓆⓌⓇⓉⓎⓊⒾⓄⓅⓈⒹⒻⒼⒽⒿⓀⓁⓏⓍⒸⓋⒷⓃⓂ";
    let has_circled = algebra_rules
        .iter()
        .any(|rule| rule.chars().any(|c| circled.contains(c)));
    let has_preedit = value
        .get("translator")
        .and_then(|t| t.get("preedit_format"))
        .is_some();
    let is_double_pinyin = has_circled && has_preedit;

    Ok(SchemaRaw {
        schema_id,
        name,
        version,
        translator_kind,
        alphabet,
        max_code_length,
        primary_dictionary,
        custom_phrase_user_dict,
        algebra,
        is_double_pinyin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("定位仓库根失败")
            .join("tests/fixtures")
            .join(rel)
    }

    #[test]
    fn flypy_schema_is_double_pinyin_with_custom_phrase() {
        let raw = parse_schema(&fixture("rime-crane-flypy/double_pinyin_flypy.schema.yaml"))
            .expect("解析失败");
        assert_eq!(raw.schema_id, "double_pinyin_flypy");
        assert_eq!(raw.name, "小鹤双拼");
        assert!(raw.is_double_pinyin);
        assert_eq!(raw.translator_kind, TranslatorKind::Script);
        assert_eq!(
            raw.custom_phrase_user_dict.as_deref(),
            Some("custom_phrase_double")
        );
        assert_eq!(raw.primary_dictionary.as_deref(), Some("rime_ice"));
        let algebra = raw.algebra.expect("应有 algebra");
        assert!(
            algebra.rules.iter().any(|r| r.contains("Ⓠ")),
            "应含圈字母规则"
        );
    }

    #[test]
    fn xhup_schema_is_table_with_max_code_length() {
        let raw = parse_schema(&fixture("rime-crane-xhup/xhup.schema.yaml")).expect("解析失败");
        assert_eq!(raw.schema_id, "xhup");
        assert_eq!(raw.name, "小鹤音形");
        assert_eq!(raw.translator_kind, TranslatorKind::Table);
        assert_eq!(raw.primary_dictionary.as_deref(), Some("xhup"));
        assert_eq!(raw.max_code_length, Some(4));
        assert!(raw.is_double_pinyin == false);
        assert!(raw.algebra.is_none(), "table 方案无 algebra");
        assert!(raw.alphabet.contains(";"));
        assert!(raw.alphabet.contains("'"));
    }

    #[test]
    fn missing_schema_id_is_error() {
        let err = parse_schema_str("schema:\n  name: x\n");
        assert!(matches!(err, Err(SchemaParseError::MissingSchemaId)));
    }
}
