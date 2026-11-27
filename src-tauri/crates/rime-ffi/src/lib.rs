//! librime 动态加载 + 隔离会话候选预览（PRD FR-3.24 P1 实时预览 / §6.1 / §6.5 R1）。
//!
//! ## 设计要点
//!
//! - **动态加载，不静态链接**：用 `libloading` 打开 `librime.so.1` / `librime.dylib` /
//!   `rime.dll`，路径可由 `RIMETOOLS_LIBRIME` 覆盖（Windows 将来指向 Weasel 安装目录内的
//!   `rime.dll`，实机 1.13.1）。函数指针一律从 `rime_get_api()` 返回的 `RimeApi` 表取，
//!   并做表长校验——这是兼容 1.13.1 / 1.16.1 两代 ABI 最稳的方式。
//! - **隔离会话**：`RimeTraits` 的 `user_data_dir`/`shared_data_dir` 全部指向临时目录副本
//!   （[`IsolatedRimeHome`]），只把用户目录当**只读复制来源**；绝不指向或写入
//!   `~/.local/share/fcitx5/rime` 等用户正在使用的输入法环境。
//! - **逐键模拟**：每个按键调用一次 `RimeSimulateKeySequence`，随后 `RimeGetCommit` +
//!   `RimeGetContext`；context 用后立即 `RimeFreeContext`（不泄漏）。
//! - **xhup 4 码自动上屏**：隔离副本通过 `<schema>.custom.yaml` 关闭
//!   `speller/auto_select`（只改临时副本）；同时保留逐键快照回退——若调用方目录没关
//!   auto_select，第 4 键上屏后回退到前缀候选并在 `note` 中说明，不静默返回空。
//! - **失败可降级**：所有失败返回 [`FfiError`]；上层用 [`unavailable_preview`] 生成
//!   `PreviewMode::Unavailable` 预览并保留静态权重预览作为 P0 路径。
//!
//! ## 最小示例
//!
//! ```no_run
//! use rime_ffi::{HomeContent, HomeSpec, IsolatedRimeHome, RimePreview};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let home = IsolatedRimeHome::prepare(
//!     HomeSpec::new("double_pinyin_flypy")
//!         .preset_source("/usr/share/rime-data")
//!         .user_file(HomeContent::copy(
//!             "/path/to/user/double_pinyin_flypy.schema.yaml",
//!             "double_pinyin_flypy.schema.yaml",
//!         )),
//! )?;
//! let preview = RimePreview::open(home)?;
//! let candidates = preview.preview("nihao", Some("你好"))?;
//! for item in &candidates.items {
//!     println!("{}{}", item.text, if item.is_target { " ← 当前文本" } else { "" });
//! }
//! # Ok(())
//! # }
//! ```

pub mod capture;
pub mod error;
pub mod ffi;
pub mod home;
mod loader;
pub mod preview;

pub use capture::{Capture, RawCandidate, RawContext, SessionIo, StepResult};
pub use error::FfiError;
pub use home::{HomeContent, HomeSpec, IsolatedRimeHome};
pub use loader::{
    default_library_names, engine_available, library_candidates, probe, shared_library,
    LibraryProbe, RimeLib, ENV_LIBRIME,
};
pub use preview::{
    last_deploy_state, probe_librime, unavailable_preview, PreviewRequest, RimePreview,
};

// 供上层直接使用的类型（来自 rime-core，保持单一事实源）。
pub use rime_core::types::{Candidate, CandidatePreview, PreviewMode};
/// 编译期保证：上层可把 `RimePreview` 放进 Tauri 的 `State`（要求 `Send + Sync`）。
#[allow(dead_code)]
fn _assert_preview_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<RimePreview>();
    assert_send_sync::<FfiError>();
}
