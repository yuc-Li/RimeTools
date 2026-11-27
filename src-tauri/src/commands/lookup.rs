//! 只读反查命令（M2）：跨词库查词/查码，以及符号打法查询。
//!
//! 数据由 `crate::lookup_cache` 装配并缓存在 `AppState`；用户改动了词库文件后
//! 可调用 `lookup_rebuild` 重建。反查全程只读，不影响正在使用的输入法。

use crate::lookup_cache::LookupCache;
use crate::state::AppState;
use rime_core::lookup::{query, LookupQuery, LookupResult, LookupSourceSummary};
use rime_platform::symbols::{self, SymbolHit};
use serde::Serialize;
use tauri::State;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupSourcesResult {
    pub sources: Vec<LookupSourceSummary>,
    pub symbol_sources: Vec<String>,
    /// 解析失败的来源数量
    pub failed: u32,
    pub user_dir: String,
}

fn user_dir(state: &AppState) -> std::path::PathBuf {
    if let Some(dir) = state.settings().user_dir {
        return dir;
    }
    rime_platform::detect_frontends()
        .into_iter()
        .next()
        .map(|f| f.user_dir)
        .unwrap_or_default()
}

fn shared_dir() -> std::path::PathBuf {
    rime_platform::fcitx5_shared_dir()
}

/// 取缓存；没有则构建一次。
fn cache(state: &AppState) -> std::sync::Arc<LookupCache> {
    if let Some(existing) = state.lookup_cache() {
        return existing;
    }
    let built = std::sync::Arc::new(LookupCache::build(&user_dir(state), &shared_dir()));
    state.set_lookup_cache(built.clone());
    built
}

/// 列出可反查的来源（不含条目内容）。
#[tauri::command]
pub fn lookup_sources(state: State<'_, AppState>) -> Result<LookupSourcesResult, AppErrorBox> {
    let cache = cache(&state);
    Ok(LookupSourcesResult {
        sources: cache.sources.iter().map(|s| s.summary()).collect(),
        symbol_sources: cache.symbols.sources.clone(),
        failed: cache.failed,
        user_dir: user_dir(&state).to_string_lossy().to_string(),
    })
}

type AppErrorBox = rime_core::types::AppError;

/// 执行反查。
#[tauri::command]
pub fn lookup_query(
    request: LookupQuery,
    state: State<'_, AppState>,
) -> Result<LookupResult, AppErrorBox> {
    let cache = cache(&state);
    Ok(query(&cache.sources, &request))
}

/// 重建反查缓存（用户改了其他配置文件后调用）。
#[tauri::command]
pub fn lookup_rebuild(state: State<'_, AppState>) -> Result<u32, AppErrorBox> {
    let built = std::sync::Arc::new(LookupCache::build(&user_dir(&state), &shared_dir()));
    let count = built.sources.len() as u32;
    state.set_lookup_cache(built);
    Ok(count)
}

/// 符号查询：可传符号本身或触发键。
#[tauri::command]
pub fn symbol_query(
    keyword: String,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Vec<SymbolHit>, AppErrorBox> {
    let cache = cache(&state);
    Ok(symbols::search(
        &cache.symbols,
        &keyword,
        limit.unwrap_or(50) as usize,
    ))
}
