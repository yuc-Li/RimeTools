//! 导入导出命令（`docs/ipc.md` §3.2 `dict_export` / `dict_import_*`）。
//!
//! 导入**不改写解析逻辑**：先把内容解析成中性条目 → 与现有词库比对产出计划
//! → 用户确认后转成 `SaveRequest`，复用 `dict_save` 的「暂存 / 校验 / 备份 / 原子写」，
//! 因此同样受指纹保护，不会覆盖期间被外部改动的文件。

use crate::commands::dict::{apply_save, not_opened};
use crate::state::AppState;
use rime_core::transfer::{
    export, parse_import, plan_import, to_save_request, ExportOutput, ImportPlan, ImportPolicy,
    TransferFormat,
};
use rime_core::types::{AppError, ErrorCode, FileId, SaveResult};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub file_id: FileId,
    pub format: TransferFormat,
    /// 目标路径；为空时只返回内容（供前端复制或自行保存）
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: Option<String>,
    pub suggested_name: String,
    pub format: TransferFormat,
    pub entry_count: u32,
    pub bytes: u32,
    /// 未指定路径时返回内容
    pub content: Option<String>,
}

/// 导出当前词库。指定路径则写文件，否则返回内容。
#[tauri::command]
pub fn dict_export(
    request: ExportRequest,
    state: State<'_, AppState>,
) -> Result<ExportResult, AppError> {
    let ExportOutput {
        content,
        suggested_name,
        format,
        entry_count,
    } = state
        .with_store_mut(&request.file_id, |store| export(store, request.format))
        .ok_or_else(|| not_opened(&request.file_id))?;

    let bytes = content.len() as u32;
    match request.path {
        Some(path) => {
            let target = std::path::PathBuf::from(&path);
            if let Some(dir) = target.parent() {
                if !dir.as_os_str().is_empty() && !dir.exists() {
                    std::fs::create_dir_all(dir).map_err(|e| {
                        AppError::new(ErrorCode::FileNotWritable, format!("无法创建导出目录：{e}"))
                    })?;
                }
            }
            std::fs::write(&target, content.as_bytes()).map_err(|e| {
                AppError::new(ErrorCode::FileNotWritable, format!("写入导出文件失败：{e}"))
                    .with_hint("检查目标目录是否存在且可写")
            })?;
            Ok(ExportResult {
                path: Some(path),
                suggested_name,
                format,
                entry_count,
                bytes,
                content: None,
            })
        }
        None => Ok(ExportResult {
            path: None,
            suggested_name,
            format,
            entry_count,
            bytes,
            content: Some(content),
        }),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreviewRequest {
    pub file_id: FileId,
    /// 来源文件路径；与 `content` 二选一
    pub path: Option<String>,
    pub content: Option<String>,
    /// 未指定时按扩展名推断，仍失败则按 txt
    pub format: Option<TransferFormat>,
    pub policy: ImportPolicy,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportApplyRequest {
    pub file_id: FileId,
    pub path: Option<String>,
    pub content: Option<String>,
    pub format: Option<TransferFormat>,
    pub policy: ImportPolicy,
    #[serde(default)]
    pub deploy_after: bool,
}

/// 读取导入来源并解析。路径优先于内联内容。
fn read_import_source(
    path: Option<&str>,
    content: Option<&str>,
    format: Option<TransferFormat>,
) -> Result<(String, TransferFormat), AppError> {
    let (text, fmt) = match (path, content) {
        (Some(p), _) => {
            let path = std::path::PathBuf::from(p);
            if !path.is_file() {
                return Err(AppError::new(
                    ErrorCode::FileNotFound,
                    format!("导入文件不存在：{p}"),
                ));
            }
            let text = std::fs::read(&path)
                .map_err(|e| {
                    AppError::new(ErrorCode::FileNotFound, format!("读取导入文件失败：{e}"))
                })
                .and_then(|bytes| {
                    String::from_utf8(bytes).map_err(|_| {
                        AppError::new(ErrorCode::CorruptEncoding, "导入文件不是合法 UTF-8")
                    })
                })?;
            let fmt = format
                .or_else(|| TransferFormat::from_path(&path))
                .unwrap_or(TransferFormat::Txt);
            (text, fmt)
        }
        (None, Some(c)) => (c.to_owned(), format.unwrap_or(TransferFormat::Txt)),
        (None, None) => {
            return Err(AppError::new(
                ErrorCode::FileFormatInvalid,
                "未提供导入来源（path 或 content）",
            ))
        }
    };
    Ok((text, fmt))
}

/// 预览导入：解析 + 冲突比对，不改动词库。
#[tauri::command]
pub fn dict_import_preview(
    request: ImportPreviewRequest,
    state: State<'_, AppState>,
) -> Result<ImportPlan, AppError> {
    let (text, format) = read_import_source(
        request.path.as_deref(),
        request.content.as_deref(),
        request.format,
    )?;
    let parsed = parse_import(&text, format).map_err(|e| {
        AppError::new(ErrorCode::FileFormatInvalid, e.to_string())
            .with_hint("支持 txt（制表符分隔）、csv、Rime 词典 YAML")
    })?;

    let (alphabet, max_code_length) =
        crate::commands::dict::scheme_limits(&state, &request.file_id);

    state
        .with_store_mut(&request.file_id, |store| {
            plan_import(store, &parsed, request.policy, &alphabet, max_code_length)
        })
        .ok_or_else(|| not_opened(&request.file_id))
}

/// 应用导入：重新计算计划（保证基于最新文件状态）后走标准保存流程。
#[tauri::command]
pub fn dict_import_apply(
    request: ImportApplyRequest,
    state: State<'_, AppState>,
) -> Result<SaveResult, AppError> {
    let (text, format) = read_import_source(
        request.path.as_deref(),
        request.content.as_deref(),
        request.format,
    )?;
    let parsed = parse_import(&text, format)
        .map_err(|e| AppError::new(ErrorCode::FileFormatInvalid, e.to_string()))?;
    let (alphabet, max_code_length) =
        crate::commands::dict::scheme_limits(&state, &request.file_id);

    let save_request = state
        .with_store_mut(&request.file_id, |store| {
            let plan = plan_import(store, &parsed, request.policy, &alphabet, max_code_length);
            to_save_request(store, &plan, request.deploy_after)
        })
        .ok_or_else(|| not_opened(&request.file_id))?;

    if save_request.add.is_empty() && save_request.update.is_empty() {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            "没有需要写入的改动（可能全部冲突被跳过）",
        ));
    }

    apply_save(save_request, &state)
}

/// 选择导入文件（系统文件对话框）。
#[tauri::command]
pub fn pick_import_file(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .add_filter("词库文件", &["txt", "csv", "yaml", "yml"])
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().to_string())
}

/// 选择导出保存位置。
#[tauri::command]
pub fn pick_export_path(app: AppHandle, default_name: String) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .set_file_name(default_name)
        .blocking_save_file()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().to_string())
}
