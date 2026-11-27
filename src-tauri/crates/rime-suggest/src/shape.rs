//! 形码反查（音形方案单字「双拼 + 鹤形」建议）。
//!
//! 依据实测结论：反查码表
//! （如 `xhup_reverse`）每字有 15–45 条变体，混有含 `` ` `` 的**万能码**，必须
//! 过滤掉才能取到规范形。
//!
//! 数据来源：**运行时**从用户本地方案码表反查（PRD §2.4 「编码建议的数据来源」）。
//! 本 crate 不含任何形码表内容，只定义 [`ShapeSource`] trait 与规范化逻辑。

use std::collections::HashMap;

/// 形码数据源：返回该字的**原始码表条目**（形如 `anf` = 双拼 `an` + 形码 `f`）。
///
/// 实现方可以是本地 `xhup.primary` / `xhup.secondary` / `xhup_reverse` 码表的内存索引。
pub trait ShapeSource: Send + Sync {
    /// 按码表出现顺序返回条目（顺序影响「同长度取最先出现」的规范选择）。
    fn entries(&self, ch: char) -> Vec<String>;
}

/// 内存形码表（`char → 条目列表`），便于上层从码表文件装载后注入。
#[derive(Debug, Clone, Default)]
pub struct ShapeMap {
    map: HashMap<char, Vec<String>>,
}

impl ShapeMap {
    pub fn new() -> Self {
        ShapeMap::default()
    }

    pub fn insert(&mut self, ch: char, code: impl Into<String>) {
        self.map.entry(ch).or_default().push(code.into());
    }

    /// 直接从 `"按\tanf"` 形式的码表文本装载（`文本<Tab>编码`，忽略注释与表头）。
    pub fn parse_dict(text: &str) -> Self {
        let mut map = ShapeMap::new();
        let mut in_header = false;
        for line in text.lines() {
            let line = line.trim_end_matches(['\r', '\n']);
            if line.starts_with("#") || line.trim().is_empty() {
                continue;
            }
            if line.trim() == "---" || line.trim() == "..." {
                in_header = !in_header;
                continue;
            }
            if in_header || !line.contains('\t') {
                continue;
            }
            let mut columns = line.split('\t');
            let Some(word) = columns.next() else { continue };
            let Some(code) = columns.next() else { continue };
            let mut chars = word.chars();
            let (Some(ch), None) = (chars.next(), chars.next()) else {
                continue;
            };
            if !code.is_empty() {
                map.insert(ch, code.to_owned());
            }
        }
        map
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl ShapeSource for ShapeMap {
    fn entries(&self, ch: char) -> Vec<String> {
        self.map.get(&ch).cloned().unwrap_or_default()
    }
}

/// 从原始条目里取出**规范形码**（去掉双拼前缀后的鹤形部分）。
///
/// 规则（C4）：
/// 1. 丢弃含万能码 `` ` `` 的变体（以及 `;` `'` 这类引导符条目）；
/// 2. 只保留以当前读音双拼码 `dp_code` 开头的条目（多读音条目属于别的读音）；
/// 3. 取**最短**的一条，同长度取码表中**先出现**的一条。
///
/// 返回 `Some("")` 表示该字该读音在码表里就是纯双拼码（如 `安 an`），
/// 返回 `None` 表示没有可用的规范条目。
pub fn canonical_shape(entries: &[String], dp_code: &str) -> Option<String> {
    let mut best: Option<&str> = None;
    for entry in entries {
        if entry.contains('`') || entry.contains(';') || entry.contains('\'') {
            continue;
        }
        let Some(shape) = entry.strip_prefix(dp_code) else {
            continue;
        };
        // 形码本身必须是字母（圈字母中间态属于异常数据）
        if !shape.chars().all(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        let len = shape.chars().count();
        match best {
            Some(current) if current.chars().count() <= len => {}
            _ => best = Some(shape),
        }
    }
    best.map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn filters_wildcard_variants_and_takes_canonical() {
        // 报告 §4 中「阿」的变体带 ` 万能码；规范条目是 aa（纯双拼，无形码）
        let e = entries(&["aa", "aae`", "eee`", "aa`k", "ee`k", "aa``", "ee``"]);
        assert_eq!(canonical_shape(&e, "aa"), Some(String::new()));
    }

    #[test]
    fn takes_shortest_shape_in_dict_order() {
        let e = entries(&["anf", "an`n", "anb", "an``"]);
        assert_eq!(canonical_shape(&e, "an").as_deref(), Some("f"));
    }

    #[test]
    fn ignores_entries_of_other_readings() {
        // 「行」的 hg/hh 是 hang 读音，xk 才是 xing
        let e = entries(&["hgi`", "xki`", "xk", "hg", "hh"]);
        assert_eq!(canonical_shape(&e, "xk").as_deref(), Some(""));
        // 若无 xk 条目则返回 None（不跨读音凑码）
        let e = entries(&["hgi", "hg"]);
        assert_eq!(canonical_shape(&e, "xk"), None);
    }

    #[test]
    fn rejects_non_alphabetic_shape() {
        let e = entries(&["an;", "an;"]);
        assert_eq!(canonical_shape(&e, "an"), None);
    }

    #[test]
    fn parses_dict_text() {
        let text = "\
# Rime dictionary
---
name: xhup.primary
...
按\tanf
安\tan
阿\taa
报关单\tbgd
";
        let map = ShapeMap::parse_dict(text);
        assert_eq!(map.len(), 3, "多字词条与表头应被忽略");
        assert_eq!(map.entries('按'), vec!["anf".to_string()]);
        assert_eq!(map.entries('安'), vec!["an".to_string()]);
        assert!(map.entries('关').is_empty());
    }
}
