//! 环境探测命令（PRD FR-1）。

use crate::state::AppState;
use rime_core::types::{AppError, ErrorCode, FrontendInfo, SchemeInfo};
use serde::Serialize;
use tauri::State;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvDetectResult {
    pub frontends: Vec<FrontendInfo>,
    pub active: Option<FrontendInfo>,
}

/// 自动探测本机 Rime 前端与用户目录。
///
/// 多前端共存时全部返回，`active` 仅在**唯一确定**时填充，避免静默猜测（FR-1.6）。
#[tauri::command]
pub fn env_detect(state: State<'_, AppState>) -> Result<EnvDetectResult, AppError> {
    let frontends = rime_platform::detect_frontends();

    if frontends.is_empty() {
        return Err(AppError::new(
            ErrorCode::FrontendNotFound,
            "未检测到 Rime 输入法前端",
        )
        .with_hint("请确认已安装 fcitx5-rime（Linux）或小狼毫（Windows），或手动指定用户目录"));
    }

    let active = if frontends.len() == 1 {
        frontends.first().cloned()
    } else {
        None
    };

    if let Some(f) = active.as_ref() {
        state.update_settings(|s| {
            if s.user_dir.is_none() {
                s.user_dir = Some(f.user_dir.clone());
            }
            if s.frontend.is_none() {
                s.frontend = Some(f.kind);
            }
        });
    }

    Ok(EnvDetectResult { frontends, active })
}

/// 列出当前前端下的全部方案及其个人词条受管文件。
///
/// 副作用：把受管文件注册进 `AppState`，供 `dict_*` 命令按 `fileId` 查找。
#[tauri::command]
pub fn env_list_schemes(
    user_dir: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<SchemeInfo>, AppError> {
    let dir = match user_dir {
        Some(p) => std::path::PathBuf::from(p),
        None => state.settings().user_dir.unwrap_or_else(|| {
            rime_platform::detect_frontends()
                .into_iter()
                .next()
                .map(|f| f.user_dir)
                .unwrap_or_default()
        }),
    };

    if dir.as_os_str().is_empty() {
        return Err(AppError::new(
            ErrorCode::FrontendNotFound,
            "未检测到 Rime 前端，无法列出方案",
        )
        .with_hint("请在设置中手动指定 Rime 用户目录"));
    }

    if !dir.is_dir() {
        return Err(
            AppError::new(ErrorCode::UserDirInvalid, format!("用户目录不存在：{}", dir.display()))
                .with_hint("请检查路径是否正确"),
        );
    }

    let schemes = rime_platform::detect_schemes(&dir);
    if schemes.is_empty() {
        return Err(AppError::new(
            ErrorCode::SchemeNotDetected,
            "该目录下未找到任何 *.schema.yaml 方案文件",
        )
        .with_hint("请确认选择的是 Rime 用户目录（而非共享目录）"));
    }

    state.register_schemes(&schemes);
    Ok(schemes)
}

/// 手动指定用户目录（FR-1.4）。
#[tauri::command]
pub fn env_set_user_dir(
    path: String,
    state: State<'_, AppState>,
) -> Result<Vec<SchemeInfo>, AppError> {
    let dir = std::path::PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(AppError::new(
            ErrorCode::UserDirInvalid,
            format!("目录不存在：{path}"),
        ));
    }
    let schemes = rime_platform::detect_schemes(&dir);
    if schemes.is_empty() {
        return Err(AppError::new(
            ErrorCode::SchemeNotDetected,
            "该目录下未找到 Rime 方案文件",
        )
        .with_hint("请选择 Rime 用户目录，例如 ~/.local/share/fcitx5/rime"));
    }
    state.update_settings(|s| s.user_dir = Some(dir));
    state.register_schemes(&schemes);
    Ok(schemes)
}