//! 反查数据缓存（按方案隔离）。
//!
//! 反查的关键是「只看当前输入法用到的词表」：
//! - 小鹤音形 → `xhup.dict.yaml` 的 `import_tables`（`xhup_dicts/*`）+ 它的个人码表；
//! - 小鹤双拼 → 主词典 + `custom_phrase_double.txt`。
//!
//! 每个方案一份缓存，互不干扰；切换方案时按需构建，`lookup_rebuild` 可显式重建。

use std::path::Path;
use std::sync::Arc;

use rime_core::lookup::LookupSource;
use rime_core::types::SchemeInfo;
use rime_platform::{sources, symbols};

pub struct LookupCache {
    /// 缓存键：方案 id，`*` 表示「全部文件」兜底模式
    pub key: String,
    pub schema_name: String,
    pub sources: Vec<LookupSource>,
    pub symbols: symbols::SymbolTable,
    /// 解析失败的来源数量
    pub failed: u32,
}

impl LookupCache {
    /// 按方案构建。`scheme` 为 None 时退回「扫描用户目录全部文件」。
    pub fn build(user_dir: &Path, shared_dir: &Path, scheme: Option<&SchemeInfo>) -> Self {
        let files = match scheme {
            Some(s) => {
                let scoped = sources::scan_for_scheme(user_dir, s);
                if scoped.is_empty() {
                    // 方案没声明词典（如纯英文方案）：退回全量扫描，避免空白页
                    sources::scan(user_dir, user_dir, &[])
                } else {
                    scoped
                }
            }
            None => sources::scan(user_dir, user_dir, &[]),
        };

        let loaded = sources::load_all(&files);
        let failed = loaded.iter().filter(|s| s.error.is_some()).count() as u32;

        // 符号表：预设 + 用户 + 当前方案的 punctuator
        let schema_files: Vec<(String, std::path::PathBuf)> = scheme
            .map(|s| {
                let mut path = user_dir.join(format!("{}.schema.yaml", s.schema_id));
                if !path.exists() {
                    path = user_dir.join(format!("{}.schema.yml", s.schema_id));
                }
                vec![(s.schema_id.clone(), path)]
            })
            .unwrap_or_default();
        let symbol_table = symbols::load(shared_dir, user_dir, &schema_files);

        Self {
            key: scheme
                .map(|s| s.schema_id.clone())
                .unwrap_or_else(|| "*".to_owned()),
            schema_name: scheme
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "全部文件".to_owned()),
            sources: loaded,
            symbols: symbol_table,
            failed,
        }
    }

    pub fn entry_count(&self) -> u64 {
        self.sources.iter().map(|s| s.entry_count as u64).sum()
    }
}

pub type SharedLookupCache = Arc<LookupCache>;
