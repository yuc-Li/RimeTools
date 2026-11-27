//! 编码建议与候选预览命令（契约 `docs/ipc.md` §3.3 / PRD FR-3.23、FR-3.24）。

use crate::state::AppState;
use rime_core::types::{AppError, CandidatePreview, CodeSuggestion, ErrorCode, FileRole};
use rime_ffi::preview::{probe_librime, unavailable_preview, RimePreview};
use rime_suggest::SuggestEngine;
use tauri::State;

/// 生成编码建议（多候选，含警告）。
///
/// 产品约束（PRD FR-3.23）：
/// - 建议只是**默认填充**，永远可编辑；
/// - `exclude_existing` 用于剔除词库中已有的编码，避免重复入库；
/// - `known_codes` 让用户自定义码（无法算法推导）优先展示。
#[tauri::command]
pub fn suggest_codes(
    text: String,
    schema_id: Option<String>,
    exclude_existing: Option<Vec<String>>,
    known_codes: Option<Vec<String>>,
    role: Option<FileRole>,
    state: State<'_, AppState>,
) -> Result<Vec<CodeSuggestion>, AppError> {
    let scheme = match schema_id {
        Some(id) => state.scheme(&id),
        None => state.active_scheme(),
    }
    .ok_or_else(|| {
        AppError::new(
            ErrorCode::SchemeNotDetected,
            "未确定当前方案，无法生成编码建议",
        )
        .with_hint("请先完成环境探测")
    })?;

    // 编码建议的拼音数据源：优先用户本机字表（覆盖 8105 字、含多音字），
    // 退化时用 rime-suggest 的内置常用字表（约 326 字）。
    let mut engine = SuggestEngine::from_scheme(&scheme);
    if let Some(dir) = state.settings().user_dir.or_else(|| {
        rime_platform::detect_frontends()
            .into_iter()
            .next()
            .map(|f| f.user_dir)
    }) {
        if let Some(map) = state.pinyin_source(&dir) {
            // Arc<HashMap> 未实现 trait，这里克隆一份（8000 条，量级可接受）
            engine = engine.with_pinyin(Box::new((*map).clone()));
        }
    }
    let mut opts = rime_suggest::SuggestOptions::from_scheme(&scheme);

    opts.role = role.or(Some(FileRole::CustomPhrase));
    if let Some(ex) = exclude_existing {
        opts.exclude = ex;
    }
    if let Some(known) = known_codes {
        opts.known_codes = known;
    }

    Ok(engine.suggest(&text, &opts))
}

/// 候选预览（PRD FR-3.24）。
///
/// 本阶段策略：librime 不可用时返回 `Unavailable` 并附说明，由 UI 明确告知用户，
/// **不静默降级成假数据**。实时预览（隔离会话）在 M2 接入。
#[tauri::command]
pub fn suggest_preview(
    code: String,
    schema_id: Option<String>,
    text: Option<String>,
    _state: State<'_, AppState>,
) -> Result<CandidatePreview, AppError> {
    match probe_librime() {
        Ok(_probe) => {
            // 实时预览需要隔离目录 + 方案文件收集，M2 实现；此处明确说明而非假装成功
            let _ = (code, schema_id, text);
            Ok(unavailable_preview(
                "实时候选预览将在 M2 接入（需要把方案与码表复制到隔离目录后运行 librime 会话）。",
            ))
        }
        Err(e) => Ok(unavailable_preview(format!(
            "本机未找到可用的 librime，实时预览不可用：{e}"
        ))),
    }
}

/// 供后续 M2 使用的占位：确认隔离会话能力是否就绪。
#[allow(dead_code)]
pub(crate) fn preview_ready() -> bool {
    probe_librime().is_ok() && std::mem::size_of::<RimePreview>() > 0
}
