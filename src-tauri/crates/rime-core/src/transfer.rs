//! 词库导入导出。
//!
//! 设计要点：
//! - 导出走「查询当前 store → 生成目标格式」，不经过行模型，避免把暂存态带出去；
//! - 导入先解析成中性条目，再与现有词库做冲突比对，产出**计划**（不直接写），
//!   由上层确认后转成 `SaveRequest` 走既有的「暂存 → 校验 → 备份 → 原子写」流程；
//! - 因此导入同样受指纹保护：导入期间文件被外部改动会被拒绝，而非覆盖。

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::parse::ParsedDict;
use crate::store::DictStore;
use crate::types::{
    ColumnKind, EntryKey, EntryQuery, EntryUpdate, FileFormat, NewEntry, SaveRequest,
};

// ───────────────────────── 格式 ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferFormat {
    /// `文本<Tab>编码[<Tab>权重]`
    Txt,
    /// `text,code[,weight]`（首行可为表头）
    Csv,
    /// Rime 词典 YAML（`*.dict.yaml`）
    DictYaml,
}

impl TransferFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Txt => "txt",
            Self::Csv => "csv",
            Self::DictYaml => "yaml",
        }
    }

    /// 按扩展名推断；`.yml` / `.yaml` / `.dict.yaml` 都视为词典 YAML。
    pub fn from_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
        if name.ends_with(".dict.yaml") || name.ends_with(".dict.yml") {
            return Some(Self::DictYaml);
        }
        match path
            .extension()?
            .to_string_lossy()
            .to_ascii_lowercase()
            .as_str()
        {
            "txt" | "text" => Some(Self::Txt),
            "csv" => Some(Self::Csv),
            "yaml" | "yml" => Some(Self::DictYaml),
            _ => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("无法从文件名推断格式：{0}")]
    UnknownFormat(String),
    #[error("解析导入内容失败：{0}")]
    Parse(String),
    #[error("导入内容为空，没有可用的词条")]
    Empty,
}

// ───────────────────────── 导出 ─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOutput {
    pub content: String,
    /// 建议文件名（不含目录）
    pub suggested_name: String,
    pub format: TransferFormat,
    pub entry_count: u32,
}

/// 导出当前词库（**不含**未保存的暂存改动之外的东西：暂存改动在内存里，导出的是所见内容）。
pub fn export(store: &DictStore, format: TransferFormat) -> ExportOutput {
    let page = store.query(&EntryQuery {
        limit: u32::MAX,
        ..EntryQuery::default()
    });
    let entries = &page.items;

    let has_weight =
        store.columns().contains(&ColumnKind::Weight) || entries.iter().any(|e| e.weight.is_some());

    let stem = store
        .file_path()
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "dictionary".to_owned());

    let content = match format {
        TransferFormat::Txt => to_txt(entries, has_weight),
        TransferFormat::Csv => to_csv(entries, has_weight),
        TransferFormat::DictYaml => to_dict_yaml(&stem, entries, has_weight),
    };

    ExportOutput {
        content,
        suggested_name: format!("{stem}.{}", transfer_suffix(format)),
        format,
        entry_count: entries.len() as u32,
    }
}

fn transfer_suffix(format: TransferFormat) -> &'static str {
    match format {
        TransferFormat::Txt => "txt",
        TransferFormat::Csv => "csv",
        TransferFormat::DictYaml => "dict.yaml",
    }
}

fn to_txt(entries: &[crate::types::Entry], has_weight: bool) -> String {
    let mut out = String::new();
    if has_weight {
        out.push_str("# 文本\t编码\t权重\n");
    } else {
        out.push_str("# 文本\t编码\n");
    }
    for e in entries {
        out.push_str(&e.key.text);
        out.push('\t');
        out.push_str(&e.key.code);
        if has_weight {
            out.push('\t');
            out.push_str(&e.weight.unwrap_or(0).to_string());
        }
        out.push('\n');
    }
    out
}

fn to_csv(entries: &[crate::types::Entry], has_weight: bool) -> String {
    let mut out = String::new();
    out.push_str(if has_weight {
        "text,code,weight\n"
    } else {
        "text,code\n"
    });
    for e in entries {
        out.push_str(&csv_field(&e.key.text));
        out.push(',');
        out.push_str(&csv_field(&e.key.code));
        if has_weight {
            out.push(',');
            out.push_str(&e.weight.unwrap_or(0).to_string());
        }
        out.push('\n');
    }
    out
}

fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

fn to_dict_yaml(stem: &str, entries: &[crate::types::Entry], has_weight: bool) -> String {
    let mut out = String::new();
    out.push_str("# Rime dictionary\n# encoding: utf-8\n#\n# 由 RimeTools 导出\n\n---\n");
    out.push_str(&format!("name: {stem}\n"));
    out.push_str("version: \"1\"\nsort: by_weight\n...\n\n");
    for e in entries {
        out.push_str(&e.key.text);
        out.push('\t');
        out.push_str(&e.key.code);
        if has_weight {
            if let Some(w) = e.weight {
                out.push('\t');
                out.push_str(&w.to_string());
            }
        }
        out.push('\n');
    }
    out
}

// ───────────────────────── 导入解析 ─────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedEntry {
    pub text: String,
    pub code: String,
    pub weight: Option<u32>,
    /// 来源行号（1 起），便于用户定位
    pub line_no: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportParse {
    pub entries: Vec<ImportedEntry>,
    /// 解析过程中跳过的行（格式问题），最多保留前 20 条
    pub issues: Vec<ImportIssue>,
    pub skipped_lines: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportIssue {
    pub line_no: u32,
    pub message: String,
    pub raw: String,
}

const MAX_ISSUES: usize = 20;

/// 解析导入内容。格式非法只是跳过该行并记录 issue，不整体失败。
pub fn parse_import(content: &str, format: TransferFormat) -> Result<ImportParse, TransferError> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let parsed = match format {
        TransferFormat::Txt => parse_delimited(content, '\t'),
        TransferFormat::Csv => parse_csv(content),
        TransferFormat::DictYaml => parse_dict_yaml(content)?,
    };
    if parsed.entries.is_empty() {
        return Err(TransferError::Empty);
    }
    Ok(parsed)
}

fn push_issue(out: &mut ImportParse, line_no: u32, message: impl Into<String>, raw: &str) {
    out.skipped_lines += 1;
    if out.issues.len() < MAX_ISSUES {
        out.issues.push(ImportIssue {
            line_no,
            message: message.into(),
            raw: raw.chars().take(120).collect(),
        });
    }
}

fn parse_delimited(content: &str, sep: char) -> ImportParse {
    let mut out = ImportParse {
        entries: Vec::new(),
        issues: Vec::new(),
        skipped_lines: 0,
    };
    for (idx, raw) in content.lines().enumerate() {
        let line_no = idx as u32 + 1;
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split(sep).collect();
        let text = cols.first().map(|s| s.trim()).unwrap_or("");
        let code = cols.get(1).map(|s| s.trim()).unwrap_or("");
        if text.is_empty() || code.is_empty() {
            push_issue(&mut out, line_no, "缺少文本或编码列", line);
            continue;
        }
        let weight = cols.get(2).and_then(|s| s.trim().parse::<u32>().ok());
        if cols.len() > 2 && weight.is_none() && !cols[2].trim().is_empty() {
            push_issue(&mut out, line_no, "权重列不是整数，已忽略权重", line);
        }
        out.entries.push(ImportedEntry {
            text: text.to_owned(),
            code: code.to_owned(),
            weight,
            line_no,
        });
    }
    out
}

/// 最小 CSV 解析：支持双引号包裹、字段内逗号与 `""` 转义；首行是表头则跳过。
fn parse_csv(content: &str) -> ImportParse {
    let mut out = ImportParse {
        entries: Vec::new(),
        issues: Vec::new(),
        skipped_lines: 0,
    };
    let records = split_csv(content);
    let mut start = 0;
    if let Some(first) = records.first() {
        let head = first
            .first()
            .map(|s| s.trim().to_ascii_lowercase())
            .unwrap_or_default();
        if head.starts_with("text") || head.starts_with("文本") {
            start = 1; // 表头
        }
    }
    for (idx, fields) in records.iter().enumerate().skip(start) {
        let line_no = idx as u32 + 1;
        let text = fields.first().map(|s| s.trim()).unwrap_or("");
        let code = fields.get(1).map(|s| s.trim()).unwrap_or("");
        if text.is_empty() || code.is_empty() {
            if fields.iter().all(|f| f.trim().is_empty()) {
                continue;
            }
            push_issue(&mut out, line_no, "缺少文本或编码列", &fields.join(","));
            continue;
        }
        let weight = fields.get(2).and_then(|s| s.trim().parse::<u32>().ok());
        out.entries.push(ImportedEntry {
            text: text.to_owned(),
            code: code.to_owned(),
            weight,
            line_no,
        });
    }
    out
}

fn split_csv(content: &str) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut field = String::new();
    let mut record = Vec::new();
    let mut in_quotes = false;
    let mut chars = content.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            }
            '"' if field.is_empty() && !in_quotes => in_quotes = true,
            ',' if !in_quotes => {
                record.push(std::mem::take(&mut field));
            }
            '\r' if !in_quotes => {}
            '\n' if !in_quotes => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    records
}

fn parse_dict_yaml(content: &str) -> Result<ImportParse, TransferError> {
    let parsed = ParsedDict::parse(content, FileFormat::RimeDictYaml)
        .map_err(|e| TransferError::Parse(e.to_string()))?;
    let mut out = ImportParse {
        entries: Vec::new(),
        issues: Vec::new(),
        skipped_lines: 0,
    };
    for (line_idx, entry) in parsed.entries() {
        let text = entry.text.trim();
        let code = entry.code.trim();
        if text.is_empty() || code.is_empty() {
            push_issue(
                &mut out,
                (line_idx + 1) as u32,
                "缺少文本或编码列",
                &entry.text,
            );
            continue;
        }
        out.entries.push(ImportedEntry {
            text: text.to_owned(),
            code: code.to_owned(),
            weight: entry.weight,
            line_no: (line_idx + 1) as u32,
        });
    }
    Ok(out)
}

// ───────────────────────── 冲突计划 ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportPolicy {
    /// 已存在同（文本, 编码）则跳过
    SkipExisting,
    /// 已存在则用导入的权重覆盖
    Overwrite,
    /// 已存在时仅在导入权重更大时更新
    KeepHigherWeight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportAction {
    Added,
    Updated,
    Skipped,
    Unchanged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportConflict {
    pub text: String,
    pub code: String,
    pub existing_weight: Option<u32>,
    pub incoming_weight: Option<u32>,
    pub action: ImportAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPlan {
    pub total: u32,
    pub added: u32,
    pub updated: u32,
    pub skipped: u32,
    pub unchanged: u32,
    /// 导入内容自身的重复（同文本同编码）
    pub duplicates: u32,
    /// 逐条冲突明细（最多 200 条，供预览展示）
    pub conflicts: Vec<ImportConflict>,
    pub issues: Vec<ImportIssue>,
    pub skipped_lines: u32,
    /// 编码字符集/长度不合法的条目（保存时会被拦下，提前告知）
    pub invalid_codes: Vec<ImportIssue>,
    /// 目标文件无权重列时，导入的权重会被忽略
    pub weight_ignored: bool,

    #[serde(skip)]
    pub new_entries: Vec<NewEntry>,
    #[serde(skip)]
    pub updates: Vec<EntryUpdate>,
}

const MAX_CONFLICTS: usize = 200;

/// 计算导入计划：只做比对，不改动词库。
pub fn plan_import(
    store: &DictStore,
    parsed: &ImportParse,
    policy: ImportPolicy,
    alphabet: &str,
    max_code_length: Option<u32>,
) -> ImportPlan {
    let existing = store.query(&EntryQuery {
        limit: u32::MAX,
        ..EntryQuery::default()
    });
    let mut by_key: std::collections::HashMap<(String, String), Option<u32>> = existing
        .items
        .iter()
        .map(|e| ((e.key.text.clone(), e.key.code.clone()), e.weight))
        .collect();

    let has_weight_col = store.columns().contains(&ColumnKind::Weight);
    let file_id = store.file_id().clone();

    let mut plan = ImportPlan {
        total: parsed.entries.len() as u32,
        added: 0,
        updated: 0,
        skipped: 0,
        unchanged: 0,
        duplicates: 0,
        conflicts: Vec::new(),
        issues: parsed.issues.clone(),
        skipped_lines: parsed.skipped_lines,
        invalid_codes: Vec::new(),
        weight_ignored: false,
        new_entries: Vec::new(),
        updates: Vec::new(),
    };

    let mut seen: HashSet<(String, String)> = HashSet::new();

    for item in &parsed.entries {
        if item.code.chars().any(|c| !alphabet.contains(c)) {
            plan.invalid_codes.push(ImportIssue {
                line_no: item.line_no,
                message: "编码含字母表之外的字符".to_owned(),
                raw: format!("{}\t{}", item.text, item.code),
            });
        }
        if let Some(max) = max_code_length {
            if item.code.chars().count() as u32 > max {
                plan.invalid_codes.push(ImportIssue {
                    line_no: item.line_no,
                    message: format!("编码超过最大长度 {max}"),
                    raw: format!("{}\t{}", item.text, item.code),
                });
            }
        }

        let key = (item.text.clone(), item.code.clone());
        if !seen.insert(key.clone()) {
            plan.duplicates += 1;
            continue;
        }

        match by_key.get(&key).copied() {
            None => {
                plan.added += 1;
                by_key.insert(key, item.weight);
                plan.new_entries.push(NewEntry {
                    text: item.text.clone(),
                    code: item.code.clone(),
                    weight: if has_weight_col { item.weight } else { None },
                    group: None,
                });
            }
            Some(existing_weight) => {
                if !has_weight_col {
                    // 两列文件没有权重概念：存在即视为相同
                    plan.weight_ignored |= item.weight.is_some();
                    plan.unchanged += 1;
                    push_conflict(&mut plan, item, existing_weight, ImportAction::Unchanged);
                    continue;
                }
                let action = match policy {
                    ImportPolicy::SkipExisting => ImportAction::Skipped,
                    ImportPolicy::Overwrite => {
                        if existing_weight == item.weight.or(Some(0)) {
                            ImportAction::Unchanged
                        } else {
                            ImportAction::Updated
                        }
                    }
                    ImportPolicy::KeepHigherWeight => match (existing_weight, item.weight) {
                        (Some(old), Some(new)) if new > old => ImportAction::Updated,
                        (None, Some(new)) if new > 0 => ImportAction::Updated,
                        _ => ImportAction::Skipped,
                    },
                };
                match action {
                    ImportAction::Updated => {
                        plan.updated += 1;
                        plan.updates.push(EntryUpdate {
                            key: EntryKey {
                                file_id: file_id.clone(),
                                text: item.text.clone(),
                                code: item.code.clone(),
                            },
                            new_text: None,
                            new_code: None,
                            weight: Some(item.weight.unwrap_or(0)),
                            group: None,
                        });
                    }
                    ImportAction::Unchanged => plan.unchanged += 1,
                    _ => plan.skipped += 1,
                }
                push_conflict(&mut plan, item, existing_weight, action);
            }
        }
    }

    plan
}

fn push_conflict(
    plan: &mut ImportPlan,
    item: &ImportedEntry,
    existing_weight: Option<u32>,
    action: ImportAction,
) {
    if plan.conflicts.len() < MAX_CONFLICTS {
        plan.conflicts.push(ImportConflict {
            text: item.text.clone(),
            code: item.code.clone(),
            existing_weight,
            incoming_weight: item.weight,
            action,
        });
    }
}

/// 把计划转成保存请求（复用既有写入流程：暂存 / 校验 / 备份 / 原子写）。
pub fn to_save_request(store: &DictStore, plan: &ImportPlan, deploy_after: bool) -> SaveRequest {
    SaveRequest {
        file_id: store.file_id().clone(),
        expected_fingerprint: store.fingerprint().clone(),
        add: plan.new_entries.clone(),
        update: plan.updates.clone(),
        delete: Vec::new(),
        reorder: None,
        deploy_after,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn txt_import_parses_three_columns() {
        let parsed = parse_import(
            "你好\tnihc\t5\n世界\tui\n# 注释\n\n坏行\n",
            TransferFormat::Txt,
        )
        .unwrap();
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].weight, Some(5));
        assert_eq!(parsed.entries[1].weight, None);
        assert_eq!(parsed.skipped_lines, 1);
        assert_eq!(parsed.entries[0].line_no, 1);
    }

    #[test]
    fn csv_import_skips_header_and_handles_quotes() {
        let csv = "text,code,weight\n\"你好\",nihc,5\n\"带,逗号\",dh,\n";
        let parsed = parse_import(csv, TransferFormat::Csv).unwrap();
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[1].text, "带,逗号");
        assert_eq!(parsed.entries[1].code, "dh");
    }

    #[test]
    fn dict_yaml_import_reads_entries() {
        let text = "---\nname: t\n...\n\n你好\tnihc\t3\n中国\tvsgo\n";
        let parsed = parse_import(text, TransferFormat::DictYaml).unwrap();
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].weight, Some(3));
        assert_eq!(parsed.entries[0].line_no, 5);
    }

    #[test]
    fn empty_import_is_an_error() {
        assert!(matches!(
            parse_import("\n# 只有注释\n", TransferFormat::Txt),
            Err(TransferError::Empty)
        ));
    }

    #[test]
    fn format_inferred_from_path() {
        assert_eq!(
            TransferFormat::from_path(Path::new("a.txt")),
            Some(TransferFormat::Txt)
        );
        assert_eq!(
            TransferFormat::from_path(Path::new("a.dict.yaml")),
            Some(TransferFormat::DictYaml)
        );
        assert_eq!(TransferFormat::from_path(Path::new("a.bin")), None);
    }

    #[test]
    fn parse_csv_splits_records_with_quotes() {
        let rows = split_csv("a,\"b,c\",d\nx,y\n");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec!["a", "b,c", "d"]);
        assert_eq!(rows[1], vec!["x", "y"]);
    }

    fn sample_dict(n: usize) -> String {
        let mut text = String::from("---\nname: t\n...\n\n");
        for i in 0..n {
            text.push_str(&format!("词{i}\tmi{i}\n"));
        }
        text
    }

    #[test]
    fn export_txt_and_csv_contain_all_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.dict.yaml");
        std::fs::write(&path, sample_dict(3)).unwrap();
        let store = DictStore::open(
            &path,
            FileFormat::RimeDictYaml,
            crate::types::FileRole::UserDict,
        )
        .unwrap();
        let txt = export(&store, TransferFormat::Txt);
        assert_eq!(txt.entry_count, 3);
        assert!(txt.content.contains("词0\tmi0"));
        assert!(txt.suggested_name.ends_with(".txt"));

        let csv = export(&store, TransferFormat::Csv);
        assert!(csv.content.starts_with("text,code"));
    }
}
