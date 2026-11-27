//! 文件解析：把文件内容拆成行模型，保留一切原始信息。

use crate::types::FileFormat;
use crate::{EntryLine, Line, Newline};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("文件内容不是合法 UTF-8：{0}")]
    InvalidUtf8(String),
    #[error("YAML 头部未找到 `...` 分隔符，无法确定数据区起点")]
    MissingYamlTerminator,
    #[error("文件过长，超过 {limit} 字节上限")]
    TooLarge { limit: u64 },
}

/// 解析后的字典文件。
#[derive(Debug, Clone)]
pub struct ParsedDict {
    pub format: FileFormat,
    /// 是否带 UTF-8 BOM（写入时还原，避免破坏用户文件特征）。
    pub bom: bool,
    pub newline: Newline,
    /// 原文件是否以换行符结尾。必须保留：部分 Rime 文件（如 xhup.dict.yaml）
    /// 结尾无换行，擅自补上会造成非必要 diff。
    pub ends_with_newline: bool,
    /// 全部行，顺序即原文件顺序。
    pub lines: Vec<Line>,
}

impl ParsedDict {
    /// 解析文件内容。
    ///
    /// `format` 决定是否需要寻找 YAML `...` 分隔符。
    pub fn parse(content: &str, format: FileFormat) -> Result<Self, ParseError> {
        let (bom, body) = match content.strip_prefix('\u{feff}') {
            Some(rest) => (true, rest),
            None => (false, content),
        };

        // 换行风格：出现第一个 \r\n 即判定 CRLF
        let newline = if body.contains("\r\n") { Newline::Crlf } else { Newline::Lf };
        let ends_with_newline = body.ends_with('\n');
        let normalized = body.replace("\r\n", "\n");

        let mut lines = Vec::new();
        let mut in_data = format != FileFormat::RimeDictYaml;
        let mut saw_terminator = false;

        for raw in normalized.split('\n') {
            // 末尾空串来自 split 的尾随换行，跳过（序列化时补回）
            if !in_data {
                lines.push(Line::Raw(raw.to_owned()));
                if raw.trim() == "..." {
                    in_data = true;
                    saw_terminator = true;
                }
                continue;
            }
            if raw.is_empty() && !saw_terminator && format == FileFormat::RimeDictYaml {
                // YAML 头部内的空行
                lines.push(Line::Raw(raw.to_owned()));
                continue;
            }
            match EntryLine::parse(raw) {
                Some(entry) => lines.push(Line::Entry(entry)),
                None => lines.push(Line::Raw(raw.to_owned())),
            }
        }

        // 尾随换行导致的最后一个空行：移除，序列化时统一补
        if matches!(lines.last(), Some(Line::Raw(s)) if s.is_empty()) {
            lines.pop();
        }

        if format == FileFormat::RimeDictYaml && !saw_terminator {
            return Err(ParseError::MissingYamlTerminator);
        }

        Ok(Self { format, bom, newline, ends_with_newline, lines })
    }

    /// 词条数量。
    pub fn entry_count(&self) -> usize {
        self.lines.iter().filter(|l| matches!(l, Line::Entry(_))).count()
    }

    /// 按文本+编码查找词条下标。
    pub fn find_entry(&self, text: &str, code: &str) -> Option<usize> {
        self.lines.iter().position(|l| match l {
            Line::Entry(e) => e.text == text && e.code == code,
            Line::Raw(_) => false,
        })
    }

    /// 遍历全部词条。
    pub fn entries(&self) -> impl Iterator<Item = (usize, &EntryLine)> {
        self.lines.iter().enumerate().filter_map(|(i, l)| match l {
            Line::Entry(e) => Some((i, e)),
            Line::Raw(_) => None,
        })
    }

    /// 是否存在已编辑但未序列化的行。
    pub fn has_dirty(&self) -> bool {
        self.entries().any(|(_, e)| e.is_dirty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUSTOM_PHRASE: &str = "\
# Rime table
# coding: utf-8
#@/db_name\tcustom_phrase_double.txt
#
# 此行之后不能写注释

一个\tig
美团\tmwtr\t100
";

    #[test]
    fn parses_custom_phrase_with_comments() {
        let d = ParsedDict::parse(CUSTOM_PHRASE, FileFormat::CustomPhrase).unwrap();
        assert_eq!(d.entry_count(), 2);
        assert!(!d.bom);
        assert_eq!(d.newline, Newline::Lf);
        let (_, e) = d.entries().next().unwrap();
        assert_eq!(e.text, "一个");
        assert_eq!(e.code, "ig");
        assert_eq!(e.weight, None);
    }

    #[test]
    fn parses_dict_yaml_with_header_and_terminator() {
        let src = "\
# Rime dictionary
# encoding: utf-8
#---config@码表分类=主码-用户码表

---
name: xhup.user
version: \"v10.9s\"
sort: by_weight
...

报关单\tbgd
美团\tmwtr
";
        let d = ParsedDict::parse(src, FileFormat::RimeDictYaml).unwrap();
        assert_eq!(d.entry_count(), 2);
        // 头部（含 `...` 之前的全部）必须逐行保留
        let raws: Vec<&str> = d
            .lines
            .iter()
            .filter_map(|l| match l {
                crate::Line::Raw(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        assert!(raws.iter().any(|s| s.contains("#---config@码表分类=主码-用户码表")));
        assert!(raws.iter().any(|s| s.trim() == "..."));
    }

    #[test]
    fn dict_yaml_without_terminator_is_error() {
        let err = ParsedDict::parse("name: x\n", FileFormat::RimeDictYaml);
        assert!(matches!(err, Err(ParseError::MissingYamlTerminator)));
    }

    #[test]
    fn detects_bom_and_crlf() {
        let src = "\u{feff}一个\tig\r\n";
        let d = ParsedDict::parse(src, FileFormat::CustomPhrase).unwrap();
        assert!(d.bom);
        assert_eq!(d.newline, Newline::Crlf);
        assert_eq!(d.entry_count(), 1);
    }

    #[test]
    fn finds_entry_by_text_and_code() {
        let d = ParsedDict::parse(CUSTOM_PHRASE, FileFormat::CustomPhrase).unwrap();
        assert!(d.find_entry("美团", "mwtr").is_some());
        assert!(d.find_entry("美团", "wrong").is_none());
    }
}