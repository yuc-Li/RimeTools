//! 部署命令（契约 `docs/ipc.md` §3.5 / PRD FR-5）。

use crate::state::AppState;
use rime_core::types::{AppError, DeployResult, DeployState, DeployTrigger, ErrorCode};
use rime_deploy::RequestOutcome;
use serde::Serialize;
use tauri::State;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployStartResult {
    pub accepted: bool,
    /// true 表示已有部署在跑，本次请求被合并（本轮结束后最多补跑一次）
    pub merged: bool,
}

/// 发起部署。并发请求由 `DeployController` 合并，不会排队 N 次。
#[tauri::command]
pub fn deploy_start(
    trigger: Option<DeployTrigger>,
    state: State<'_, AppState>,
) -> Result<DeployStartResult, AppError> {
    let user_dir = resolve_user_dir(&state)?;
    let controller = state.deploy_controller(&user_dir);

    let outcome = controller.request(trigger.unwrap_or(DeployTrigger::Manual))?;

    Ok(match outcome {
        RequestOutcome::Started => DeployStartResult {
            accepted: true,
            merged: false,
        },
        RequestOutcome::Merged => DeployStartResult {
            accepted: false,
            merged: true,
        },
    })
}

/// 查询部署状态与最近结果。
#[tauri::command]
pub fn deploy_status(state: State<'_, AppState>) -> Result<DeployState, AppError> {
    match state.existing_deploy_controller() {
        Some(ctl) => Ok(ctl.state()),
        // 尚未部署过：返回 Idle，而不是报错（前端无需区分"没部署过"与"空闲"）
        None => Ok(DeployState::default()),
    }
}

/// 最近若干次部署结果。
#[tauri::command]
pub fn deploy_history(
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<DeployResult>, AppError> {
    match state.existing_deploy_controller() {
        Some(ctl) => Ok(ctl.recent_results(limit.unwrap_or(10))),
        None => Ok(Vec::new()),
    }
}

/// 等待当前部署结束（上限 `timeout_ms`），主要用于测试与「保存并部署」的同步反馈。
#[tauri::command]
pub fn deploy_wait(timeout_ms: Option<u64>, state: State<'_, AppState>) -> Result<bool, AppError> {
    match state.existing_deploy_controller() {
        Some(ctl) => Ok(ctl.wait_until_idle(std::time::Duration::from_millis(
            timeout_ms.unwrap_or(30_000),
        ))),
        None => Ok(true),
    }
}

/// 打开部署日志（`docs/ipc.md` §3.5 `deploy_open_log`）。
///
/// 只允许打开确实存在的本地文件；路径来自后端自己写入的 `logPath`，
/// 仍做存在性校验，避免前端传入任意路径。
#[tauri::command]
pub fn deploy_open_log(path: String, app: tauri::AppHandle) -> Result<serde_json::Value, AppError> {
    use tauri_plugin_opener::OpenerExt;

    let path_buf = std::path::PathBuf::from(&path);
    if !path_buf.is_file() {
        return Err(
            AppError::new(ErrorCode::NotFound, format!("日志文件不存在：{path}"))
                .with_hint("部署成功且产生日志后才有文件；也可到部署页查看最近结果"),
        );
    }
    app.opener()
        .open_path(path_buf.to_string_lossy(), None::<&str>)
        .map_err(|e| {
            AppError::new(ErrorCode::Internal, "打开日志失败").with_detail(e.to_string())
        })?;
    Ok(serde_json::json!({ "ok": true }))
}

fn resolve_user_dir(state: &AppState) -> Result<std::path::PathBuf, AppError> {
    state
        .settings()
        .user_dir
        .or_else(|| {
            rime_platform::detect_frontends()
                .into_iter()
                .next()
                .map(|f| f.user_dir)
        })
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::DeployUnavailable,
                "未确定 Rime 用户目录，无法部署",
            )
            .with_hint("请先在设置中指定用户目录，或确认 fcitx5-rime / 小狼毫已安装")
        })
}
