//! 启动装配：把应用壳（AppHandle）与业务状态接上。
//!
//! 这里只做「接线」，不含业务逻辑：事件转发、观察者注入等。

use crate::state::AppState;
use tauri::{Emitter, Manager};

/// 注册部署状态转发器：`rime-deploy` 每次状态变化 → 前端 `deploy://state`。
///
/// 契约见 `docs/ipc.md` §4。前端用 `src/ipc/events.ts` 的 `useDeployState()` 订阅，
/// 全局单例，页面切换不丢。
pub fn wire_deploy_events(app: &tauri::AppHandle) {
    let handle = app.clone();
    let observer: std::sync::Arc<dyn Fn(&rime_core::types::DeployState) + Send + Sync> =
        std::sync::Arc::new(move |state| {
            if let Err(e) = handle.emit("deploy://state", state) {
                log::warn!("转发部署状态失败：{e}");
            }
        });
    app.state::<AppState>().attach_deploy_observer(observer);
}

/// 首次启动时做一次环境探测，把用户目录与方案预热进状态。
///
/// 失败不阻断启动（用户可能尚未安装输入法），但会记日志，界面上再引导手动指定。
pub fn warm_up_environment(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let frontends = rime_platform::detect_frontends();
    let Some(active) = frontends.first().cloned() else {
        log::info!("未检测到 Rime 前端，等待用户手动指定用户目录");
        return;
    };

    state.update_settings(|s| {
        if s.user_dir.is_none() {
            s.user_dir = Some(active.user_dir.clone());
        }
        if s.frontend.is_none() {
            s.frontend = Some(active.kind);
        }
    });

    match rime_platform::detect_schemes(&active.user_dir) {
        schemes if schemes.is_empty() => {
            log::warn!("{} 下未识别到方案", active.user_dir.display());
        }
        schemes => {
            log::info!(
                "环境就绪：{} 个方案，用户目录 {}",
                schemes.len(),
                active.user_dir.display()
            );
            state.register_schemes(&schemes);
        }
    }
}
