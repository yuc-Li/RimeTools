//! 序列化：把行模型写回文件内容。
//!
//! **往返保真约定**：未编辑的行原样输出，因此「解析 → 序列化」对未修改文件
//! 必须逐字节相同（BOM、换行风格、注释、表格外的任何字符都不变）。

use crate::types::ColumnKind;
use crate::{EntryLine, Line, ParsedDict, WriteOptions};

#[derive(Debug, thiserror::Error)]
pub enum SerializeError {
    #[error("写入列配置非法：必须包含 text 与 code")]
    InvalidColumns,
    #[error("词条缺少必要字段：text 或 code 为空")]
    EmptyField,
}

/// 按给定列顺序格式化一条词条。
fn format_entry(entry: &EntryLine, columns: &[ColumnKind]) -> Result<String, SerializeError> {
    if entry.text.is_empty() || entry.code.is_empty() {
        return Err(SerializeError::EmptyField);
    }
    let mut out: Vec<String> = Vec::with_capacity(columns.len());
    for col in columns {
        match col {
            ColumnKind::Text => out.push(entry.text.clone()),
            ColumnKind::Code => out.push(entry.code.clone()),
            ColumnKind::Weight => {
                // 目标格式含权重列：有权重写权重，无权重写 0（Rime 语义：0 为默认）
                out.push(entry.weight.unwrap_or(0).to_string());
            }
        }
    }
    Ok(out.join("\t"))
}

impl ParsedDict {
    /// 序列化为文件内容（含 BOM 与换行风格还原）。
    pub fn serialize(&self, opts: &WriteOptions) -> Result<String, SerializeError> {
        if !opts.columns.contains(&ColumnKind::Text) || !opts.columns.contains(&ColumnKind::Code) {
            return Err(SerializeError::InvalidColumns);
        }

        let nl = opts.newline.as_str();
        let mut out = String::new();
        if self.bom {
            out.push('\u{feff}');
        }

        let mut first = true;
        for line in &self.lines {
            // 行间分隔：仅在第一行之后插入
            if !first {
                out.push_str(nl);
            }
            first = false;
            match line {
                Line::Raw(s) => out.push_str(s),
                Line::Entry(e) => {
                    if !e.is_dirty() {
                        // 未编辑：原样输出，保证保真
                        out.push_str(strip_eol(&e.raw));
                    } else {
                        out.push_str(&format_entry(e, &opts.columns)?);
                    }
                }
            }
        }

        // 仅在原文件以换行结尾时才补尾随换行，避免制造无意义 diff
        if self.ends_with_newline && !out.is_empty() {
            out.push_str(nl);
        }
        Ok(out)
    }

    /// 便捷方法：按文件自身风格与本文件探测到的列风格序列化。
    pub fn serialize_default(&self) -> Result<(String, WriteOptions), SerializeError> {
        let columns = crate::detect_columns(self.format, &self.lines);
        let opts = WriteOptions::new(columns, self.newline);
        let content = self.serialize(&opts)?;
        Ok((content, opts))
    }
}

impl EntryLine {
    /// 保存成功后固化当前字段：`raw` 更新为按列风格格式化后的行并清除 dirty。
    ///
    /// 供 `DictStore::mark_saved` 调用，保证保存后内存状态与磁盘字节一致
    /// （否则下次序列化会用旧的 `raw` 覆盖掉刚保存的内容）。
    pub fn commit(&mut self, columns: &[ColumnKind]) -> Result<(), SerializeError> {
        self.raw = format_entry(self, columns)?;
        self.dirty = false;
        Ok(())
    }
}

fn strip_eol(s: &str) -> &str {
    s.strip_suffix('\r').unwrap_or(s)
}

impl Line {
    /// 取出原始文本（供保真比较）。
    pub fn raw_text(&self) -> Option<&str> {
        match self {
            Line::Raw(s) => Some(s),
            Line::Entry(e) => Some(&e.raw),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FileFormat;

    const SAMPLE_CP: &str = "\
# Rime table
# coding: utf-8
#
# 此行之后不能写注释

一个\tig
美团\tmwtr\t100
";

    #[test]
    fn roundtrip_is_byte_identical_custom_phrase() {
        let d = ParsedDict::parse(SAMPLE_CP, FileFormat::CustomPhrase).unwrap();
        let (out, _) = d.serialize_default().unwrap();
        assert_eq!(out, SAMPLE_CP, "往返必须逐字节一致");
    }

    #[test]
    fn roundtrip_is_byte_identical_dict_yaml() {
        let src = "\
# Rime dictionary
#---config@码表分类=主码-用户码表

---
name: xhup.user
sort: by_weight
...

报关单\tbgd
美团\tmwtr
";
        let d = ParsedDict::parse(src, FileFormat::RimeDictYaml).unwrap();
        let (out, _) = d.serialize_default().unwrap();
        assert_eq!(out, src, "YAML 头部与注释必须完整保留");
    }

    #[test]
    fn roundtrip_preserves_bom_and_crlf() {
        let src = "\u{feff}一个\tig\r\n";
        let d = ParsedDict::parse(src, FileFormat::CustomPhrase).unwrap();
        let (out, _) = d.serialize_default().unwrap();
        assert_eq!(out, src);
    }

    #[test]
    fn dirty_entry_is_reformatted_others_untouched() {
        let mut d = ParsedDict::parse(SAMPLE_CP, FileFormat::CustomPhrase).unwrap();
        let idx = d.find_entry("美团", "mwtr").unwrap();
        if let Line::Entry(e) = &mut d.lines[idx] {
            e.set_weight(Some(200));
        }
        let (out, _) = d.serialize_default().unwrap();
        assert!(out.contains("美团\tmwtr\t200"));
        assert!(out.contains("一个\tig"), "未编辑行必须保持原样（无权重列）");
        assert!(out.starts_with("# Rime table"), "注释必须保留");
    }

    #[test]
    fn two_column_file_does_not_gain_weight_column() {
        let src = "\
---
name: xhup.user
...

报关单\tbgd
美团\tmwtr
";
        let d = ParsedDict::parse(src, FileFormat::RimeDictYaml).unwrap();
        let (out, opts) = d.serialize_default().unwrap();
        assert_eq!(opts.columns.len(), 2, "两列文件不应被推断为三列");
        assert_eq!(out, src);
    }
}