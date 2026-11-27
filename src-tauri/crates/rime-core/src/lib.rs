//! RimeTools 核心：词库文件解析与序列化。
//!
//! ## 设计目标（PRD FR-3 / §6.4）
//!
//! 1. **字节级往返保真**：未修改的文件解析后再序列化，必须与原文件逐字节相同。
//!    → 每行保留原始文本，只有被编辑的行才重新格式化。
//! 2. **格式自适应**：`custom_phrase*.txt`（三列）与 `*.dict.yaml` / 两列用户码表
//!    共用一套行模型，通过每行的列数探测决定写入列数，避免把三列写进两列文件。
//! 3. **注释与头部完整保留**：rime-crane 的用户码表带 `#---config@码表分类=...` 头注释，
//!    方案靠它识别码表用途，丢失会破坏方案。
//! 4. **换行符与 BOM 保留**：不擅自统一 CRLF/LF。

use crate::types::{ColumnKind, FileFormat};

pub mod store;
pub mod types;

mod fingerprint;
mod parse;
mod serialize;
mod validate;

pub use fingerprint::{fingerprint_bytes, fingerprint_file, FingerprintError};
pub use parse::{ParseError, ParsedDict};
pub use serialize::SerializeError;
pub use store::{DictStore, UNGROUPED};
pub use validate::validate_dict;

/// 换行风格。保留原文件风格，不擅自统一。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Newline {
    Lf,
    Crlf,
}

impl Newline {
    pub fn as_str(self) -> &'static str {
        match self {
            Newline::Lf => "\n",
            Newline::Crlf => "\r\n",
        }
    }
}

/// 一行内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// 原样保留：注释、空行、YAML 头部、`...` 分隔符、无法识别的行。
    Raw(String),
    /// 数据行（词条）。
    Entry(EntryLine),
}

/// 一条词条行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryLine {
    pub text: String,
    pub code: String,
    pub weight: Option<u32>,
    /// 原始行内容（不含换行符）。未修改时原样输出，保证往返保真。
    raw: String,
    /// 是否被编辑过；未编辑则输出 `raw`，编辑后按列风格重新格式化。
    dirty: bool,
}

impl EntryLine {
    /// 从原始行解析；列数不足或为空则返回 `None`（该行降级为 `Line::Raw`）。
    pub fn parse(raw: &str) -> Option<Self> {
        if raw.starts_with('#') || raw.trim().is_empty() {
            return None;
        }
        let fields: Vec<&str> = raw.split('\t').collect();
        if fields.len() < 2 {
            return None;
        }
        let text = fields[0].trim();
        let code = fields[1].trim();
        if text.is_empty() || code.is_empty() {
            return None;
        }
        // 第三列存在时按权重解析；非数字则视为原样保留（不丢数据）
        let weight = fields.get(2).and_then(|w| {
            let w = w.trim();
            if w.is_empty() {
                None
            } else {
                w.parse::<u32>().ok()
            }
        });
        // 若第三列存在但无法解析成数字，说明格式异常，降级为 Raw 以免写坏
        if fields.len() >= 3
            && !fields[2].trim().is_empty()
            && weight.is_none()
        {
            return None;
        }
        Some(Self {
            text: text.to_owned(),
            code: code.to_owned(),
            weight,
            raw: raw.to_owned(),
            dirty: false,
        })
    }

    /// 新建一条词条行（暂存「新增」用）。
    ///
    /// 新行始终标记为 dirty，序列化时会按 `WriteOptions.columns` 重新格式化，
    /// 因此不会沿用空 `raw`。
    pub fn new(text: impl Into<String>, code: impl Into<String>, weight: Option<u32>) -> Self {
        Self {
            text: text.into(),
            code: code.into(),
            weight,
            raw: String::new(),
            dirty: true,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        let text = text.into();
        if self.text != text {
            self.text = text;
            self.dirty = true;
        }
    }

    pub fn set_code(&mut self, code: impl Into<String>) {
        let code = code.into();
        if self.code != code {
            self.code = code;
            self.dirty = true;
        }
    }

    pub fn set_weight(&mut self, weight: Option<u32>) {
        if self.weight != weight {
            self.weight = weight;
            self.dirty = true;
        }
    }
}

/// 序列化选项。
#[derive(Debug, Clone)]
pub struct WriteOptions {
    /// 输出列顺序。来自 `ManagedFileRef.columns`。
    pub columns: Vec<ColumnKind>,
    /// 目标换行风格。
    pub newline: Newline,
}

impl WriteOptions {
    pub fn new(columns: Vec<ColumnKind>, newline: Newline) -> Self {
        Self { columns, newline }
    }
}

/// 推断文件格式与列风格（依据文件角色与内容）。
pub fn detect_columns(format: FileFormat, lines: &[Line]) -> Vec<ColumnKind> {
    match format {
        // custom_phrase 固定三列（权重可缺省）
        FileFormat::CustomPhrase => vec![ColumnKind::Text, ColumnKind::Code, ColumnKind::Weight],
        FileFormat::RimeDictYaml | FileFormat::PlainTxt | FileFormat::Csv => {
            // 以实际数据行中最常见的列数决定，默认三列
            let mut two = 0usize;
            let mut three = 0usize;
            for line in lines {
                if let Line::Entry(e) = line {
                    if e.weight.is_some() {
                        three += 1;
                    } else {
                        two += 1;
                    }
                }
            }
            if two > three {
                vec![ColumnKind::Text, ColumnKind::Code]
            } else {
                vec![ColumnKind::Text, ColumnKind::Code, ColumnKind::Weight]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_line_parses_three_columns() {
        let e = EntryLine::parse("美团\tmwtr\t100").expect("应解析成功");
        assert_eq!(e.text, "美团");
        assert_eq!(e.code, "mwtr");
        assert_eq!(e.weight, Some(100));
        assert!(!e.is_dirty());
    }

    #[test]
    fn entry_line_parses_two_columns_without_weight() {
        let e = EntryLine::parse("美团\tmwtr").expect("应解析成功");
        assert_eq!(e.weight, None);
    }

    #[test]
    fn entry_line_rejects_comment_and_blank() {
        assert!(EntryLine::parse("# 注释").is_none());
        assert!(EntryLine::parse("   ").is_none());
        assert!(EntryLine::parse("只有一列").is_none());
    }

    #[test]
    fn entry_line_rejects_non_numeric_weight_to_avoid_corruption() {
        // 第三列非数字 → 降级为 Raw，避免写坏文件
        assert!(EntryLine::parse("美团\tmwtr\tabc").is_none());
    }

    #[test]
    fn editing_marks_dirty() {
        let mut e = EntryLine::parse("美团\tmwtr\t100").unwrap();
        assert!(!e.is_dirty());
        e.set_weight(Some(200));
        assert!(e.is_dirty());
        // 同值不标脏
        let mut e2 = EntryLine::parse("美团\tmwtr\t100").unwrap();
        e2.set_text("美团");
        assert!(!e2.is_dirty());
    }
}