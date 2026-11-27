//! RimeTools 应用壳：窗口、托盘、单实例、插件与命令注册。
//!
//! 业务逻辑一律在 `crates/*` 中实现，本 crate 只做 Tauri 集成。
//! 命令注册集中在 [`commands`]，禁止在业务模块里直接写 `#[tauri::command]`（见 docs/ipc.md §8）。

pub mod backup_index;
pub mod boot;
pub mod commands;
mod lookup_cache;
pub mod settings_file;
pub mod state;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};

/// 单实例：第二次启动时聚焦已存在的主窗口。
fn focus_main_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

/// 构建日志插件：stdout 始终启用；文件日志仅在目录可写时启用。
///
/// 两个坑（都已踩过）：
/// 1. `Builder::new()` 的默认目标是 `[Stdout, LogDir]`，**`.target()` 是追加而非替换**，
///    必须显式 `clear_targets()`，否则 LogDir 目标仍会去写只读的家目录并让插件初始化失败。
/// 2. 插件初始化失败会直接让应用启动 panic。日志属于可降级能力，不应拖垮主功能
///    （只读家目录、容器、配额耗尽等场景都会触发）。
fn log_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use tauri_plugin_log::{Target, TargetKind};

    let mut builder = tauri_plugin_log::Builder::new()
        .clear_targets()
        .level(log::LevelFilter::Info)
        .target(Target::new(TargetKind::Stdout));

    if let Some(dir) = writable_log_dir() {
        builder = builder.target(Target::new(TargetKind::Folder {
            path: dir,
            file_name: None,
        }));
    } else {
        // 找不到可写目录就只留 stdout，功能不受影响
        eprintln!("[RimeTools] 日志目录不可写，仅输出到 stdout");
    }
    builder.build()
}

/// 找一个真正可写的日志目录；都不可写则返回 None。
fn writable_log_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(std::path::PathBuf::from))
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from))?;

    let candidate = home.join(".local/share/com.rimetools.app/logs");
    if std::fs::create_dir_all(&candidate).is_err() {
        return None;
    }
    // create_dir_all 成功不代表可写（只读挂载/配额），再实测一次
    let probe = candidate.join(".write-probe");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Some(candidate)
        }
        Err(_) => None,
    }
}

/// 托盘「立即部署」：与前端 `deploy_start` 共用同一控制器（并发请求会被合并）。
fn deploy_from_tray(app: &tauri::AppHandle) {
    use tauri::Manager;
    let state = app.state::<state::AppState>();
    let user_dir = state.settings().user_dir.or_else(|| {
        rime_platform::detect_frontends()
            .into_iter()
            .next()
            .map(|f| f.user_dir)
    });
    let Some(user_dir) = user_dir else {
        log::warn!("托盘部署失败：未确定 Rime 用户目录");
        return;
    };
    let controller = state.deploy_controller(&user_dir);
    match controller.request(rime_core::types::DeployTrigger::Manual) {
        Ok(rime_deploy::RequestOutcome::Started) => log::info!("托盘触发部署"),
        Ok(rime_deploy::RequestOutcome::Merged) => log::info!("部署进行中，本次请求已合并"),
        Err(e) => log::warn!("托盘触发部署失败：{e}"),
    }
}

/// 托盘图标。
///
/// 用专门的 `tray-64.png` 而不是应用图标：应用图标带外层圆角底块，
/// 缩到面板尺寸（约 24px）后细节糊在一起；托盘版去掉了外层底、主体占满画布，
/// 小尺寸下更清楚，且透明底能同时适配浅色/深色面板。
fn tray_image() -> tauri::Result<tauri::image::Image<'static>> {
    // include_bytes! 给的是 'static 切片，解出的 Image 也是 'static，不会借用 app。
    tauri::image::Image::from_bytes(include_bytes!("../icons/tray-64.png"))
        .map_err(|e| tauri::Error::AssetNotFound(format!("托盘图标解码失败：{e}")))
}

/// 构建托盘菜单。
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "打开主窗口", true, None::<&str>)?;
    let deploy = MenuItem::with_id(app, "deploy", "立即部署", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &deploy, &quit])?;

    TrayIconBuilder::with_id("main-tray")
        .icon(tray_image()?)
        .tooltip("RimeTools")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => focus_main_window(app),
            "deploy" => {
                // 托盘直接触发部署：走的仍是 rime-deploy 的状态机与并发合并，
                // 与前端按钮完全同一条路径；进度经 deploy://state 推给界面。
                deploy_from_tray(app);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // 左键点击：显示/聚焦窗口
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                focus_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        // 单实例必须最先注册
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            focus_main_window(app);
        }))
        .plugin(log_plugin())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(state::AppState::new())
        .invoke_handler(tauri::generate_handler![
            // 环境（PRD FR-1）
            commands::env::env_detect,
            commands::env::env_list_schemes,
            commands::env::env_set_user_dir,
            // 词库（PRD FR-3）
            commands::dict::dict_open,
            commands::dict::dict_query,
            commands::dict::dict_groups,
            commands::dict::dict_validate,
            commands::dict::dict_save,
            // 导入导出（M2）
            commands::transfer::dict_export,
            commands::transfer::dict_import_preview,
            commands::transfer::dict_import_apply,
            commands::transfer::pick_import_file,
            commands::transfer::pick_export_path,
            // 只读反查（M2）
            commands::lookup::lookup_sources,
            commands::lookup::lookup_query,
            commands::lookup::lookup_rebuild,
            commands::lookup::symbol_query,
            // 编码建议与预览（PRD FR-3.23 / 3.24）
            commands::suggest::suggest_codes,
            commands::suggest::suggest_preview,
            // 备份与历史（PRD FR-6）
            commands::backup::backup_list,
            commands::backup::backup_now,
            commands::backup::backup_restore,
            commands::backup::backup_delete,
            commands::backup::history_list,
            // 部署（PRD FR-5）
            commands::deploy::deploy_start,
            commands::deploy::deploy_status,
            commands::deploy::deploy_history,
            commands::deploy::deploy_wait,
            commands::deploy::deploy_open_log,
            // 方案配置（M2 设置增强）
            commands::scheme_config::scheme_config_get,
            commands::scheme_config::scheme_config_update,
            // 设置（PRD FR-8）
            commands::settings::settings_get,
            commands::settings::settings_update,
            commands::settings::settings_reset,
            commands::settings::settings_open_dir,
        ])
        .setup(|app| {
            if let Err(e) = build_tray(app.handle()) {
                log::warn!("托盘创建失败（不影响主功能）：{e}");
            }
            // 事件转发与环境预热（失败不阻断启动）
            boot::wire_deploy_events(app.handle());
            boot::warm_up_environment(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            // 关闭窗口默认最小化到托盘，而非退出（PRD FR-7.1）
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            // 不让用户只看到一句 panic 栈：给出可读原因与下一步
            eprintln!("RimeTools 启动失败：{e}");
            eprintln!(
                "如果反复出现，请检查显示服务（DISPLAY/WAYLAND_DISPLAY）与 ~/.local/share 可写性。"
            );
            std::process::exit(1);
        });
}
