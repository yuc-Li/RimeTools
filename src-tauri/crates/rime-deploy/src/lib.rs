//! 部署编排（PRD FR-5 / §6.2）：fcitx5 D-Bus 触发、`build/` 产物判定、状态机与并发合并。
//!
//! # 部署链路（Linux fcitx5，按优先级）
//!
//! 1. D-Bus `org.fcitx.Fcitx5` → `/controller` →
//!    `org.fcitx.Fcitx.Controller1.SetConfig("fcitx://config/addon/rime/deploy", "")`；
//! 2. D-Bus `ReloadAddonConfig("rime")`；
//! 3. 兜底：`rime_deployer --build <user_dir> <shared_dir> <build_dir>`，并提示用户重启 fcitx5。
//!
//! **明确不使用 `fcitx5-remote -r`**：实测它只让 fcitx5 重读全局配置，不会让 Rime 重载
//! `build/`（PRD FR-5.4 已列为禁用路径）。
//!
//! # 结果判定（FR-5.5）
//!
//! 部署是否生效**不看退出码**：部署前后对 `build/` 做 mtime + size 快照比对，
//! 只有产物真的变化才置 `DeployResult.output_verified = true`；无变化则报
//! `DeployTimeout`（无法确认生效），不会静默成功。
//!
//! # 状态机与并发（FR-5.6）
//!
//! `Idle → Preparing → Running → Verifying → Success | Failed`，每次变化回调
//! [`DeployObserver`] 供 Tauri 层发事件；`Running` 期间的新请求只置一个 pending 标志，
//! 当前轮结束后最多补跑一次，不排队 N 次。
//!
//! # 设计取向：D-Bus 走 `gdbus` 子进程
//!
//! D-Bus 只用到 4 个固定方法，`gdbus`（GLib 自带）足够；不引入 zbus/dbus crate 可避免给
//! 整个工作区增加数十个传递依赖（本项目注重安装包体积与构建速度），也避免 `dbus` crate
//! 对 libdbus-1 开发库/Windows 交叉构建的额外要求。探测不到 `gdbus` 时按设计降级到
//! `rime_deployer` 兜底，不会静默失败。若将来改用库绑定，替换 [`DbusClient`] 实现即可，
//! 状态机与 [`Deployer`] 无需改动。
//!
//! # 用法
//!
//! ```no_run
//! use std::sync::Arc;
//! use rime_core::types::DeployTrigger;
//! use rime_deploy::{DeployConfig, DeployController, Fcitx5Deployer};
//!
//! let controller = DeployController::new(
//!     Arc::new(Fcitx5Deployer::system()),
//!     DeployConfig::new("/home/me/.local/share/fcitx5/rime"),
//! );
//! // Tauri 层：注册事件回调 → 请求部署（立即返回，不阻塞 UI）
//! controller.set_observer(Some(Arc::new(|state: &rime_core::types::DeployState| {
//!     println!("deploy phase: {:?}", state.phase);
//! })));
//! controller.request(DeployTrigger::Manual).expect("启动部署");
//! // 或者等待本轮结果（CLI / 测试）
//! let result = controller.deploy_blocking(DeployTrigger::Manual);
//! ```

#![forbid(unsafe_code)]

mod controller;
mod dbus;
mod deployer;
mod error;
pub mod mock;
mod process;
mod snapshot;

pub use controller::{
    build_is_up_to_date, DeployConfig, DeployController, DeployObserver, RequestOutcome,
};
pub use dbus::{
    DbusCallOutcome, DbusClient, GdbusClient, CONTROLLER_INTERFACE, CONTROLLER_PATH,
    DEPLOY_CONFIG_URI, FCITX_SERVICE, RIME_INTERFACE, RIME_PATH, RIME_SERVICE,
};
pub use deployer::{
    probe_linux_deploy_capability, DeployContext, DeployOutcome, Deployer, Fcitx5Deployer,
};
pub use process::{
    BinaryLocator, PathBinaryLocator, ProcessCommand, ProcessOutcome, ProcessRunner,
    SystemProcessRunner,
};
pub use snapshot::{BuildDelta, BuildSnapshot, FileStamp};

// 领域类型统一从 rime-core 复用（docs/ipc.md 冻结契约），此处再导出方便上层只依赖本 crate。
pub use rime_core::types::{
    AppError, AppResult, DeployCapability, DeployPhase, DeployResult, DeployState, DeployTrigger,
    ErrorCode,
};
