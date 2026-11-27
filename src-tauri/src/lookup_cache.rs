//! 反查数据缓存。
//!
//! 反查需要把「其他配置文件的词库」读进内存才能快速检索。这里负责：
//! - 按用户目录扫描候选文件（只扫用户目录，系统预设不纳入，避免几十 MB 常驻）；
//! - 解析成 `rime_core::lookup::LookupSource`；
//! - 同时装配符号表（预设 + 用户 + 各方案 punctuator）。
//!
//! 缓存由 `AppState` 持有；`lookup_rebuild` 可显式重建（用户改了文件后）。

use std::path::Path;
use std::sync::Arc;

use rime_core::lookup::LookupSource;
use rime_platform::{sources, symbols};

pub struct LookupCache {
    pub sources: Vec<LookupSource>,
    pub symbols: symbols::SymbolTable,
    /// 扫描到但解析失败的来源数量（给 UI 提示用）
    pub failed: u32,
}

impl LookupCache {
    /// 构建缓存。`user_dir` 为 Rime 用户目录，`shared_dir` 用于取 symbols.yaml。
    pub fn build(user_dir: &Path, shared_dir: &Path) -> Self {
        // 方案 id 从用户目录的 *.schema.yaml 文件名推断，便于把词库归到方案
        let schema_ids = collect_schema_ids(user_dir);

        let files = sources::scan(user_dir, user_dir, &schema_ids);
        let loaded = sources::load_all(&files);
        let failed = loaded.iter().filter(|s| s.error.is_some()).count() as u32;

        let symbol_table =
            symbols::load(shared_dir, user_dir, &schema_files(user_dir, &schema_ids));

        Self {
            sources: loaded,
            symbols: symbol_table,
            failed,
        }
    }

    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    pub fn entry_count(&self) -> u64 {
        self.sources.iter().map(|s| s.entry_count as u64).sum()
    }
}

pub type SharedLookupCache = Arc<LookupCache>;

fn collect_schema_ids(user_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(user_dir) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.strip_suffix(".schema.yaml")
                .or_else(|| name.strip_suffix(".schema.yml"))
                .map(|s| s.to_owned())
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

fn schema_files(user_dir: &Path, schema_ids: &[String]) -> Vec<(String, std::path::PathBuf)> {
    schema_ids
        .iter()
        .map(|id| {
            let mut path = user_dir.join(format!("{id}.schema.yaml"));
            if !path.exists() {
                path = user_dir.join(format!("{id}.schema.yml"));
            }
            (id.clone(), path)
        })
        .collect()
}
