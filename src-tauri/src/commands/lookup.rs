//! 只读反查命令（M2）：按**当前输入法方案**加载它的词表来查词/查码，以及查符号打法。
//!
//! 为什么要按方案：不同方案用的词表完全不同——小鹤音形是 `xhup_dicts/*`，
//! 小鹤双拼是主词典 + `custom_phrase_double.txt`。混在一起查会得到无关结果，
//! 也会把几十万条的附属码表都读进内存。
//!
//! 每个方案一份缓存（`AppState.lookup`），首次查询时惰性构建。

use crate::lookup_cache::{LookupCache, SharedLookupCache};
use crate::state::AppState;
use rime_core::lookup::{query, LookupQuery, LookupResult, LookupSourceSummary};
use rime_core::types::{AppError, ErrorCode, SchemeInfo};
use rime_platform::symbols::{self, SymbolHit};
use serde::Serialize;
use tauri::State;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupSourcesResult {
    /// 实际使用的方案 id；`*` 表示「全部文件」兜底
    pub schema_id: String,
    pub schema_name: String,
    pub sources: Vec<LookupSourceSummary>,
    pub symbol_sources: Vec<String>,
    pub failed: u32,
    pub user_dir: String,
    /// 当前可选的方案列表（供 UI 切换）
    pub schemes: Vec<LookupSchemeOption>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupSchemeOption {
    pub schema_id: String,
    pub name: String,
    pub is_active: bool,
    pub is_double_pinyin: bool,
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

/// 解析要用的方案：显式传入 > 当前激活方案 > 第一个方案 > None（全量兜底）。
fn resolve_scheme(state: &AppState, schema_id: Option<&str>) -> Option<SchemeInfo> {
    if let Some(id) = schema_id {
        if id == "*" {
            return None;
        }
        if let Some(s) = state.scheme(id) {
            return Some(s);
        }
    }
    state.active_scheme()
}

fn cache_key(scheme: Option<&SchemeInfo>) -> String {
    scheme
        .map(|s| s.schema_id.clone())
        .unwrap_or_else(|| "*".to_owned())
}

fn ensure_cache(state: &AppState, schema_id: Option<&str>) -> SharedLookupCache {
    let scheme = resolve_scheme(state, schema_id);
    let key = cache_key(scheme.as_ref());
    if let Some(existing) = state.lookup_cache(&key) {
        return existing;
    }
    let built = std::sync::Arc::new(LookupCache::build(
        &user_dir(state),
        &shared_dir(),
        scheme.as_ref(),
    ));
    state.set_lookup_cache(key, built.clone());
    built
}

/// 列出可反查的来源（不含条目内容），并附方案列表。
#[tauri::command]
pub fn lookup_sources(
    schema_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<LookupSourcesResult, AppError> {
    let cache = ensure_cache(&state, schema_id.as_deref());
    let schemes = state
        .schemes()
        .into_iter()
        .map(|s| LookupSchemeOption {
            schema_id: s.schema_id.clone(),
            name: s.name.clone(),
            is_active: s.is_active,
            is_double_pinyin: s.is_double_pinyin,
        })
        .collect();

    Ok(LookupSourcesResult {
        schema_id: cache.key.clone(),
        schema_name: cache.schema_name.clone(),
        sources: cache.sources.iter().map(|s| s.summary()).collect(),
        symbol_sources: cache.symbols.sources.clone(),
        failed: cache.failed,
        user_dir: user_dir(&state).to_string_lossy().to_string(),
        schemes,
    })
}

/// 执行反查（在指定方案的词表范围内）。
#[tauri::command]
pub fn lookup_query(
    request: LookupQuery,
    schema_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<LookupResult, AppError> {
    let cache = ensure_cache(&state, schema_id.as_deref());
    Ok(query(&cache.sources, &request))
}

/// 重建指定方案（或全部）的反查缓存。
#[tauri::command]
pub fn lookup_rebuild(
    schema_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<u32, AppError> {
    state.clear_lookup_cache();
    let cache = ensure_cache(&state, schema_id.as_deref());
    Ok(cache.sources.len() as u32)
}

/// 符号查询：可传符号本身或触发键；按方案取对应的 punctuator。
#[tauri::command]
pub fn symbol_query(
    keyword: String,
    limit: Option<u32>,
    schema_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<SymbolHit>, AppError> {
    let cache = ensure_cache(&state, schema_id.as_deref());
    if cache.symbols.entries.is_empty() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "没有找到符号表（检查系统 symbols.yaml 是否存在）",
        ));
    }
    Ok(symbols::search(
        &cache.symbols,
        &keyword,
        limit.unwrap_or(50) as usize,
    ))
}
