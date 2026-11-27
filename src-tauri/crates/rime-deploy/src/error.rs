//! 部署错误构造器：统一错误码、面向用户的说明与下一步建议（PRD FR-5.5、ipc §1.1）。
//!
//! `AppError.message` 是可直接展示给用户的中文说明，`hint` 是可执行的下一步建议。

use rime_core::types::{AppError, ErrorCode};

/// 前端不可用 / 没有可用的部署通道：上层应降级提示用户手动部署。
pub(crate) fn unavailable(message: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::DeployUnavailable, message).with_hint(
        "请确认 fcitx5 正在运行且已启用 Rime 输入法；仍失败时可在终端执行 \
         rime_deployer --build <用户目录> /usr/share/rime-data <用户目录>/build，然后重启 fcitx5",
    )
}

/// 部署超时（含等待 build/ 产物变化超时）。
pub(crate) fn timed_out(message: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::DeployTimeout, message)
        .with_hint("Rime 可能仍在后台构建，请稍候重试；若长时间无响应，可重启 fcitx5 后再次部署")
}

/// 部署失败，附带可诊断的技术细节。
pub(crate) fn failed(
    message: impl Into<String>,
    detail: impl Into<String>,
    hint: impl Into<String>,
) -> AppError {
    AppError::new(ErrorCode::DeployFailed, message)
        .with_detail(detail)
        .with_hint(hint)
}

/// 用户目录无效。
pub(crate) fn invalid_user_dir(user_dir: &std::path::Path) -> AppError {
    AppError::new(
        ErrorCode::UserDirInvalid,
        format!("Rime 用户目录不存在或不是目录：{}", user_dir.display()),
    )
    .with_hint("请在设置中选择正确的 Rime 用户目录（fcitx5 通常为 ~/.local/share/fcitx5/rime）")
}

/// 部署执行了但未生效（产物比源文件旧）。
pub(crate) fn deploy_failed(message: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::DeployFailed, message).with_hint(
        "请重启 fcitx5（fcitx5 -r -d）后重新部署；仍失败可查看 rime 日志确认编译错误",
    )
}
