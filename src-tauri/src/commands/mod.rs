//! Tauri 命令注册（契约见 `docs/ipc.md` §3）。
//!
//! 约定：命令只做「参数适配 + 调用 crate + 错误转换」，业务逻辑一律留在 crates/*。
//! `generate_handler!` 必须引用**完整模块路径**，`pub use` 重导出不会带出宏生成的命令项。

pub mod backup;
pub mod deploy;
pub mod dict;
pub mod env;
pub mod settings;
pub mod suggest;
