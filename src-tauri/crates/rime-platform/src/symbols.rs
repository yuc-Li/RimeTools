//! 符号表解析：回答「这个符号怎么打」。
//!
//! 数据来源（按覆盖顺序）：
//! 1. 系统共享目录的 `symbols.yaml`（预设）
//! 2. 用户目录的 `symbols.yaml`（若有，覆盖预设同名表）
//! 3. 各方案文件里的 `punctuator` 段（方案级覆盖）
//!
//! Rime 的写法是 `punctuator: { half_shape: { '<': [ 《, 〈 ] }, full_shape: {...} }`，
//! 值可以是 `{ commit: X }`（单符号直出）或符号列表（输入 `/键` 出候选）。

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SymbolShape {
    /// 半角模式
    Half,
    /// 全角模式
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolEntry {
    /// 触发键（如 `<`、`/`、`;`）
    pub key: String,
    /// 该键能打出的符号（第一个为默认）
    pub symbols: Vec<String>,
    /// 来源说明，如「预设符号表」「方案 xhup」
    pub scope: String,
    pub shape: SymbolShape,
    /// `commit` = 直接出字；`list` = 需 `/键` 选择
    pub mode: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolTable {
    pub entries: Vec<SymbolEntry>,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolHit {
    pub key: String,
    pub symbols: Vec<String>,
    pub scope: String,
    pub shape: SymbolShape,
    pub mode: String,
    /// 0 = 键完全匹配，1 = 符号完全匹配，2 = 键包含，3 = 符号包含
    pub rank: u8,
}

fn shape_of(name: &str) -> Option<SymbolShape> {
    match name {
        "half_shape" => Some(SymbolShape::Half),
        "full_shape" => Some(SymbolShape::Full),
        _ => None,
    }
}

/// 从一份 YAML 里提取 punctuator 的两张表。
fn collect_from_yaml(value: &Value, scope: &str, out: &mut Vec<SymbolEntry>) {
    let Some(punct) = value.get("punctuator") else {
        return;
    };
    let Some(map) = punct.as_mapping() else {
        return;
    };

    for (k, v) in map {
        let Some(shape_name) = k.as_str() else {
            continue;
        };
        let Some(shape) = shape_of(shape_name) else {
            continue;
        };
        let Some(table) = v.as_mapping() else {
            continue;
        };

        for (key, val) in table {
            let key = match key.as_str() {
                Some(s) => s.to_owned(),
                None => match key {
                    Value::Number(n) => n.to_string(),
                    _ => continue,
                },
            };

            // 形态 1：{ commit: '，' }
            if let Some(commit) = val.get("commit").and_then(|c| c.as_str()) {
                out.push(SymbolEntry {
                    key,
                    symbols: vec![commit.to_owned()],
                    scope: scope.to_owned(),
                    shape,
                    mode: "commit".to_owned(),
                });
                continue;
            }
            // 形态 2：符号列表
            if let Some(seq) = val.as_sequence() {
                let symbols: Vec<String> = seq
                    .iter()
                    .filter_map(|s| s.as_str().map(|t| t.to_owned()))
                    .collect();
                if !symbols.is_empty() {
                    out.push(SymbolEntry {
                        key,
                        symbols,
                        scope: scope.to_owned(),
                        shape,
                        mode: "list".to_owned(),
                    });
                }
            }
        }
    }
}

fn read_yaml(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_yaml::from_str::<Value>(&text).ok()
}

/// 加载符号表。
///
/// `schema_files` 是 `(方案 id, 方案文件路径)` 列表；方案片段只取其 `punctuator`。
pub fn load(
    shared_dir: &Path,
    user_dir: &Path,
    schema_files: &[(String, std::path::PathBuf)],
) -> SymbolTable {
    let mut entries = Vec::new();
    let mut sources = Vec::new();

    // 1) 预设
    for dir in [shared_dir, user_dir] {
        let path = dir.join("symbols.yaml");
        if let Some(v) = read_yaml(&path) {
            let scope = if dir == shared_dir {
                "预设符号表"
            } else {
                "用户 symbols.yaml"
            };
            collect_from_yaml(&v, scope, &mut entries);
            sources.push(scope.to_owned());
        }
    }

    // 2) 方案级覆盖
    for (schema_id, path) in schema_files {
        if let Some(v) = read_yaml(path) {
            let scope = format!("方案 {schema_id}");
            let before = entries.len();
            collect_from_yaml(&v, &scope, &mut entries);
            if entries.len() > before {
                sources.push(scope);
            }
        }
    }

    SymbolTable {
        entries: dedupe(entries),
        sources,
    }
}

/// 后加载的覆盖先加载的（同 shape + key 只保留最后一个），保持原有顺序。
fn dedupe(entries: Vec<SymbolEntry>) -> Vec<SymbolEntry> {
    let mut latest: Vec<SymbolEntry> = Vec::new();
    // 逆序处理：先遇到的是「后加载」的版本，重复项直接跳过（保留更晚的那份）
    for entry in entries.into_iter().rev() {
        if latest
            .iter()
            .any(|e| e.key == entry.key && e.shape == entry.shape)
        {
            continue;
        }
        latest.push(entry);
    }
    latest.reverse();
    latest
}

/// 按关键词检索：可传符号本身（`《`），也可传触发键（`<`）。
pub fn search(table: &SymbolTable, keyword: &str, limit: usize) -> Vec<SymbolHit> {
    let kw = keyword.trim();
    if kw.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<SymbolHit> = Vec::new();

    for entry in &table.entries {
        let rank = if entry.key == kw {
            0
        } else if entry.symbols.iter().any(|s| s == kw) {
            1
        } else if entry.key.contains(kw) {
            2
        } else if entry.symbols.iter().any(|s| s.contains(kw)) {
            3
        } else {
            continue;
        };
        hits.push(SymbolHit {
            key: entry.key.clone(),
            symbols: entry.symbols.clone(),
            scope: entry.scope.clone(),
            shape: entry.shape,
            mode: entry.mode.clone(),
            rank,
        });
    }

    hits.sort_by(|a, b| {
        a.rank
            .cmp(&b.rank)
            .then_with(|| a.key.cmp(&b.key))
            .then_with(|| a.scope.cmp(&b.scope))
    });
    hits.truncate(limit.max(1));
    hits
}

/// 便捷入口：把文件解析成符号表（供测试与上层复用）。
pub fn load_from_texts(texts: &[(&str, &str)]) -> SymbolTable {
    let mut entries = Vec::new();
    let mut sources = Vec::new();
    for (scope, text) in texts {
        if let Ok(v) = serde_yaml::from_str::<Value>(text) {
            let before = entries.len();
            collect_from_yaml(&v, scope, &mut entries);
            if entries.len() > before {
                sources.push((*scope).to_owned());
            }
        }
    }
    SymbolTable {
        entries: dedupe(entries),
        sources,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
config_version: '1.6'
patch:
  punctuator/import_preset: symbols
punctuator:
  full_shape:
    ' ' : { commit: '　' }
    ',' : { commit: ， }
    '<' : [ 《, 〈, « ]
  half_shape:
    '<' : [ <, ≤ ]
    '/' : { commit: / }
"#;

    #[test]
    fn parses_commit_and_list_forms() {
        let table = load_from_texts(&[("预设符号表", SAMPLE)]);
        assert!(table.entries.len() >= 4);
        let comma = table
            .entries
            .iter()
            .find(|e| e.key == "," && e.shape == SymbolShape::Full)
            .unwrap();
        assert_eq!(comma.symbols, vec!["，"]);
        assert_eq!(comma.mode, "commit");

        let lt = table
            .entries
            .iter()
            .find(|e| e.key == "<" && e.shape == SymbolShape::Full)
            .unwrap();
        assert_eq!(lt.symbols, vec!["《", "〈", "«"]);
        assert_eq!(lt.mode, "list");
    }

    #[test]
    fn search_by_symbol_and_by_key() {
        let table = load_from_texts(&[("预设符号表", SAMPLE)]);

        let by_symbol = search(&table, "《", 10);
        assert_eq!(by_symbol[0].key, "<");
        assert_eq!(by_symbol[0].rank, 1);

        let by_key = search(&table, "<", 10);
        assert!(by_key.iter().all(|h| h.key == "<"));
        assert_eq!(by_key[0].rank, 0);
        assert_eq!(by_key.len(), 2, "半角与全角各一条");
    }

    #[test]
    fn later_scope_overrides_earlier() {
        let user = r#"
punctuator:
  half_shape:
    '<' : [ 你 ]
"#;
        let table = load_from_texts(&[("预设", SAMPLE), ("用户", user)]);
        let half: Vec<_> = table
            .entries
            .iter()
            .filter(|e| e.key == "<" && e.shape == SymbolShape::Half)
            .collect();
        assert_eq!(half.len(), 1, "同一 shape+key 应被覆盖而不是重复");
        assert_eq!(half[0].symbols, vec!["你"]);
        assert_eq!(half[0].scope, "用户");
    }

    #[test]
    fn empty_keyword_returns_nothing() {
        let table = load_from_texts(&[("预设", SAMPLE)]);
        assert!(search(&table, "  ", 10).is_empty());
    }

    #[test]
    fn missing_files_yield_empty_table() {
        let dir = tempfile::tempdir().unwrap();
        let table = load(dir.path(), dir.path(), &[]);
        assert!(table.entries.is_empty());
    }
}
