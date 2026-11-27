//! 设置命令（PRD FR-8）。
//!
//! M1 阶段：内存态 + 默认值。持久化（tauri-plugin-store）在 T1.9 接入。

use crate::state::AppState;
use rime_core::types::{AppError, AppSettings};
use tauri::State;

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> Result<AppSettings, AppError> {
    Ok(state.settings())
}

#[tauri::command]
pub fn settings_update(
    patch: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<AppSettings, AppError> {
    let updated = state.update_settings(|s| {
        // 只覆盖显式给出的字段，未提供的保持原值（部分更新语义）
        if let Some(v) = patch.get("userDir") {
            s.user_dir = v.as_str().map(std::path::PathBuf::from);
        }
        if let Some(v) = patch.get("autoDeployAfterSave").and_then(|v| v.as_bool()) {
            s.auto_deploy_after_save = v;
        }
        if let Some(v) = patch.get("theme").and_then(|v| v.as_str()) {
            s.theme = match v {
                "light" => rime_core::types::Theme::Light,
                "dark" => rime_core::types::Theme::Dark,
                _ => rime_core::types::Theme::System,
            };
        }
        if let Some(v) = patch.get("language").and_then(|v| v.as_str()) {
            s.language = v.to_owned();
        }
        if let Some(b) = patch.get("backup") {
            if let Some(v) = b.get("enabled").and_then(|v| v.as_bool()) {
                s.backup.enabled = v;
            }
            if let Some(v) = b.get("dir") {
                s.backup.dir = v.as_str().map(std::path::PathBuf::from);
            }
            if let Some(v) = b.get("keepCount").and_then(|v| v.as_u64()) {
                s.backup.keep_count = v as u32;
            }
            if let Some(v) = b.get("scheduledEnabled").and_then(|v| v.as_bool()) {
                s.backup.scheduled_enabled = v;
            }
            if let Some(v) = b.get("scheduledIntervalHours").and_then(|v| v.as_u64()) {
                s.backup.scheduled_interval_hours = v as u32;
            }
        }
    });
    Ok(updated)
}

/// 在文件管理器中打开某个目录（契约 `docs/ipc.md` §3.7 `settings_open_dir`）。
///
/// `which` 取值：`user`（Rime 用户目录）、`backup`（备份目录）、`log`（日志目录）、
/// 或直接传绝对路径。
#[tauri::command]
pub fn settings_open_dir(
    which: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    use tauri_plugin_opener::OpenerExt;

    let settings = state.settings();
    let dir = match which.as_str() {
        "user" => settings.user_dir.or_else(|| {
            rime_platform::detect_frontends()
                .into_iter()
                .next()
                .map(|f| f.user_dir)
        }),
        "backup" => Some(state.backup_dir()),
        "log" => {
            crate::settings_file::settings_path().and_then(|p| p.parent().map(|d| d.to_path_buf()))
        }
        other => Some(std::path::PathBuf::from(other)),
    }
    .ok_or_else(|| {
        AppError::new(
            rime_core::types::ErrorCode::NotFound,
            format!("无法确定要打开的目录：{which}"),
        )
    })?;

    if !dir.exists() {
        // 备份目录可能在首次备份前不存在，顺手建出来比报错更友好
        if std::fs::create_dir_all(&dir).is_err() {
            return Err(AppError::new(
                rime_core::types::ErrorCode::NotFound,
                format!("目录不存在：{}", dir.display()),
            ));
        }
    }

    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| {
            AppError::new(rime_core::types::ErrorCode::Internal, "打开目录失败")
                .with_detail(e.to_string())
        })?;
    Ok(serde_json::json!({ "ok": true, "path": dir.to_string_lossy() }))
}

#[tauri::command]
pub fn settings_reset(state: State<'_, AppState>) -> Result<AppSettings, AppError> {
    Ok(state.update_settings(|s| *s = AppSettings::default()))
}
