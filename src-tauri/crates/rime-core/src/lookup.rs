//! 只读反查：在多个词库文件里按文本或编码检索。
//!
//! 用途（M2）：
//! - 某个词「有没有打法」——按文本查它在哪些词典里、编码是什么；
//! - 某个编码下有哪些候选——按编码查；
//! - 顺带回答「这个字是不是别的方案里也有」。
//!
//! 本模块只对**已加载**的数据做查询，不做 IO；扫描与加载在 `rime-platform`。
//! 相同 (文本, 编码) 在多个文件出现时合并为一条，来源列为列表。

use serde::{Deserialize, Serialize};

/// 反查来源的性质。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceOrigin {
    /// 用户目录（可编辑的个人词库）
    User,
    /// 系统共享目录（只读预设）
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupEntry {
    pub text: String,
    pub code: String,
    pub weight: Option<u32>,
    pub line_no: u32,
}

/// 一个可反查的词库文件（含已解析的条目）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupSource {
    /// 稳定标识（文件绝对路径）
    pub id: String,
    /// 展示名（文件名）
    pub name: String,
    pub origin: SourceOrigin,
    /// 归属方案（能推断时）
    pub schema_id: Option<String>,
    /// 文件格式（`dict.yaml` / `custom_phrase` / `txt`）
    pub format: String,
    pub entry_count: u32,
    /// 加载失败的原因（此时 entry_count 为 0）
    pub error: Option<String>,
    /// 解析出的条目；不参与序列化（可能很大）
    #[serde(skip)]
    pub entries: Vec<LookupEntry>,
}

impl LookupSource {
    pub fn summary(&self) -> LookupSourceSummary {
        LookupSourceSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            origin: self.origin,
            schema_id: self.schema_id.clone(),
            format: self.format.clone(),
            entry_count: self.entry_count,
            error: self.error.clone(),
        }
    }
}

/// 不含条目的来源信息（给前端列表用，避免传输整个词库）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupSourceSummary {
    pub id: String,
    pub name: String,
    pub origin: SourceOrigin,
    pub schema_id: Option<String>,
    pub format: String,
    pub entry_count: u32,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LookupMode {
    /// 按词条文本查 → 得到编码
    Text,
    /// 按编码查 → 得到候选词
    Code,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupQuery {
    pub mode: LookupMode,
    pub keyword: String,
    /// true = 完全相等；false = 前缀 + 包含
    pub exact: bool,
    /// 只在这些来源里查（空 = 全部）
    pub source_ids: Vec<String>,
    pub limit: u32,
}

impl Default for LookupQuery {
    fn default() -> Self {
        Self {
            mode: LookupMode::Text,
            keyword: String::new(),
            exact: false,
            source_ids: Vec::new(),
            limit: 200,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HitSource {
    pub id: String,
    pub name: String,
    pub origin: SourceOrigin,
    pub schema_id: Option<String>,
    pub weight: Option<u32>,
    pub line_no: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupHit {
    pub text: String,
    pub code: String,
    /// 0 = 完全匹配，1 = 前缀匹配，2 = 包含匹配
    pub rank: u8,
    pub sources: Vec<HitSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupResult {
    pub hits: Vec<LookupHit>,
    /// 命中总数（可能大于 hits.len()，超过 limit 时截断）
    pub total: u32,
    pub truncated: bool,
    /// 参与检索的来源数
    pub scanned_sources: u32,
}

/// 执行查询。关键词为空时返回空结果（避免把整个词库传出去）。
pub fn query(sources: &[LookupSource], q: &LookupQuery) -> LookupResult {
    let keyword = q.keyword.trim().to_lowercase();
    let filtered: Vec<&LookupSource> = sources
        .iter()
        .filter(|s| s.error.is_none())
        .filter(|s| q.source_ids.is_empty() || q.source_ids.iter().any(|id| id == &s.id))
        .collect();

    if keyword.is_empty() {
        return LookupResult {
            hits: Vec::new(),
            total: 0,
            truncated: false,
            scanned_sources: filtered.len() as u32,
        };
    }

    // (text, code) → 命中聚合
    let mut merged: Vec<(LookupHit, Vec<HitSource>)> = Vec::new();
    let mut index: std::collections::HashMap<(String, String), usize> =
        std::collections::HashMap::new();

    for source in &filtered {
        for entry in &source.entries {
            let haystack = match q.mode {
                LookupMode::Text => &entry.text,
                LookupMode::Code => &entry.code,
            }
            .to_lowercase();

            let rank = if haystack == keyword {
                0
            } else if !q.exact && haystack.starts_with(&keyword) {
                1
            } else if !q.exact && haystack.contains(&keyword) {
                2
            } else {
                continue;
            };

            let hit_source = HitSource {
                id: source.id.clone(),
                name: source.name.clone(),
                origin: source.origin,
                schema_id: source.schema_id.clone(),
                weight: entry.weight,
                line_no: entry.line_no,
            };

            let key = (entry.text.clone(), entry.code.clone());
            match index.get(&key) {
                Some(&pos) => {
                    merged[pos].0.rank = merged[pos].0.rank.min(rank);
                    merged[pos].1.push(hit_source);
                }
                None => {
                    index.insert(key, merged.len());
                    merged.push((
                        LookupHit {
                            text: entry.text.clone(),
                            code: entry.code.clone(),
                            rank,
                            sources: Vec::new(),
                        },
                        vec![hit_source],
                    ));
                }
            }
        }
    }

    // 排序：匹配等级 → 文本 → 编码，保证结果稳定
    merged.sort_by(|a, b| {
        a.0.rank
            .cmp(&b.0.rank)
            .then_with(|| a.0.text.cmp(&b.0.text))
            .then_with(|| a.0.code.cmp(&b.0.code))
    });

    let total = merged.len() as u32;
    let limit = q.limit.max(1) as usize;
    let truncated = merged.len() > limit;

    let hits = merged
        .into_iter()
        .take(limit)
        .map(|(mut hit, sources)| {
            hit.sources = sources;
            hit
        })
        .collect();

    LookupResult {
        hits,
        total,
        truncated,
        scanned_sources: filtered.len() as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(
        id: &str,
        origin: SourceOrigin,
        entries: &[(&str, &str, Option<u32>)],
    ) -> LookupSource {
        LookupSource {
            id: id.to_owned(),
            name: format!("{id}.dict.yaml"),
            origin,
            schema_id: None,
            format: "dict.yaml".to_owned(),
            entry_count: entries.len() as u32,
            error: None,
            entries: entries
                .iter()
                .enumerate()
                .map(|(i, (t, c, w))| LookupEntry {
                    text: (*t).to_owned(),
                    code: (*c).to_owned(),
                    weight: *w,
                    line_no: i as u32 + 1,
                })
                .collect(),
        }
    }

    #[test]
    fn text_lookup_finds_codes_across_sources() {
        let sources = vec![
            source(
                "a",
                SourceOrigin::User,
                &[("你好", "nihc", None), ("世界", "uijm", None)],
            ),
            source("b", SourceOrigin::System, &[("你好", "ni'hc", Some(3))]),
        ];
        let r = query(
            &sources,
            &LookupQuery {
                mode: LookupMode::Text,
                keyword: "你好".into(),
                exact: true,
                ..Default::default()
            },
        );
        assert_eq!(r.total, 2, "同词不同码应各算一条");
        assert_eq!(r.hits.len(), 2);
        assert!(r.hits.iter().all(|h| h.rank == 0));
    }

    #[test]
    fn same_word_and_code_merges_sources() {
        let sources = vec![
            source("a", SourceOrigin::User, &[("你好", "nihc", None)]),
            source("b", SourceOrigin::System, &[("你好", "nihc", None)]),
        ];
        let r = query(
            &sources,
            &LookupQuery {
                mode: LookupMode::Text,
                keyword: "你好".into(),
                exact: true,
                ..Default::default()
            },
        );
        assert_eq!(r.total, 1);
        assert_eq!(r.hits[0].sources.len(), 2);
    }

    #[test]
    fn code_lookup_supports_prefix() {
        let sources = vec![source(
            "a",
            SourceOrigin::User,
            &[("你好", "nihc", None), ("中国", "vsgo", None)],
        )];
        let r = query(
            &sources,
            &LookupQuery {
                mode: LookupMode::Code,
                keyword: "ni".into(),
                exact: false,
                ..Default::default()
            },
        );
        assert_eq!(r.total, 1);
        assert_eq!(r.hits[0].text, "你好");
        assert_eq!(r.hits[0].rank, 1);
    }

    #[test]
    fn exact_mode_rejects_partial() {
        let sources = vec![source("a", SourceOrigin::User, &[("你好", "nihc", None)])];
        let r = query(
            &sources,
            &LookupQuery {
                mode: LookupMode::Text,
                keyword: "你".into(),
                exact: true,
                ..Default::default()
            },
        );
        assert_eq!(r.total, 0);
    }

    #[test]
    fn empty_keyword_returns_nothing() {
        let sources = vec![source("a", SourceOrigin::User, &[("你好", "nihc", None)])];
        let r = query(&sources, &LookupQuery::default());
        assert_eq!(r.total, 0);
        assert_eq!(r.scanned_sources, 1);
    }

    #[test]
    fn limit_truncates_but_keeps_total() {
        let entries: Vec<(&str, &str, Option<u32>)> =
            (0..10).map(|_| ("词", "ci", None)).collect::<Vec<_>>();
        let mut s = source("a", SourceOrigin::User, &entries);
        for (i, e) in s.entries.iter_mut().enumerate() {
            e.text = format!("词{i}");
        }
        let r = query(
            &[s],
            &LookupQuery {
                mode: LookupMode::Text,
                keyword: "词".into(),
                limit: 3,
                ..Default::default()
            },
        );
        assert_eq!(r.hits.len(), 3);
        assert_eq!(r.total, 10);
        assert!(r.truncated);
    }

    #[test]
    fn source_filter_is_respected() {
        let sources = vec![
            source("a", SourceOrigin::User, &[("你好", "nihc", None)]),
            source("b", SourceOrigin::System, &[("你好", "nihc", None)]),
        ];
        let r = query(
            &sources,
            &LookupQuery {
                mode: LookupMode::Text,
                keyword: "你好".into(),
                source_ids: vec!["b".into()],
                ..Default::default()
            },
        );
        assert_eq!(r.hits[0].sources.len(), 1);
        assert_eq!(r.hits[0].sources[0].id, "b");
        assert_eq!(r.scanned_sources, 1);
    }

    #[test]
    fn errored_source_is_skipped() {
        let mut s = source("a", SourceOrigin::User, &[("你好", "nihc", None)]);
        s.error = Some("解析失败".into());
        let r = query(
            &[s],
            &LookupQuery {
                mode: LookupMode::Text,
                keyword: "你好".into(),
                ..Default::default()
            },
        );
        assert_eq!(r.total, 0);
        assert_eq!(r.scanned_sources, 0);
    }
}
