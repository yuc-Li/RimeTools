//! 隔离会话候选预览（PRD FR-3.24 P1 / §6.5 R1）。
//!
//! ## 生命周期
//!
//! librime 的 `RimeSetup`/`RimeInitialize`/`RimeFinalize` 是**进程级全局状态**，
//! 因此 [`RimePreview`] 用全局守卫保证同一进程只有一个存活实例（[`FfiError::EngineBusy`]）。
//! 典型用法：
//!
//! ```no_run
//! # use rime_ffi::{HomeSpec, HomeContent, IsolatedRimeHome, RimePreview};
//! let home = IsolatedRimeHome::prepare(
//!     HomeSpec::new("xhup")
//!         .preset_source("/usr/share/rime-data")
//!         // 用户真实目录只作为复制来源（只读），不会写入
//!         .user_file(HomeContent::copy("/path/to/user/xhup.schema.yaml", "xhup.schema.yaml")),
//! )?;
//! let preview = RimePreview::open(home)?;
//! let result = preview.preview("anui", Some("按时"))?;
//! println!("mode={:?} items={:?}", result.mode, result.items);
//! # Ok::<(), rime_ffi::FfiError>(())
//! ```
//!
//! ## 每个请求
//!
//! 1. `RimeCreateSession`（并 `RimeSelectSchema`）；
//! 2. 逐键 `RimeSimulateKeySequence`；
//! 3. 每键后 `RimeGetCommit` + `RimeGetContext`（context 用完立刻 `RimeFreeContext`）；
//! 4. `RimeDestroySession`（`SessionGuard` 保证异常路径也销毁）。
//!
//! 候选排序、auto_select 回退逻辑在 [`crate::capture`]。

use std::ffi::{c_char, c_void, CString};
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use rime_core::types::{Candidate, CandidatePreview, PreviewMode};

use crate::capture::{collect_candidates, SessionIo, StepResult};
use crate::error::FfiError;
use crate::ffi::{RimeSessionId, RimeTraits};
use crate::home::IsolatedRimeHome;
use crate::loader::{EngineGuard, RimeLib, SessionGuard};

/// 一次预览请求。
#[derive(Debug, Clone, Default)]
pub struct PreviewRequest<'a> {
    /// 要模拟的编码（ASCII；空白字符会被忽略）。
    pub code: &'a str,
    /// 当前编辑的文本；与候选完全相等时置 `is_target = true`。
    pub target_text: Option<&'a str>,
    /// 最多返回几项；默认取 `menu/page_size`（PRD：N = page_size）。
    pub max_items: Option<usize>,
}

impl<'a> PreviewRequest<'a> {
    pub fn new(code: &'a str) -> Self {
        Self {
            code,
            target_text: None,
            max_items: None,
        }
    }

    pub fn target_text(mut self, text: &'a str) -> Self {
        self.target_text = Some(text);
        self
    }

    pub fn max_items(mut self, max_items: usize) -> Self {
        self.max_items = Some(max_items);
        self
    }
}

/// 构造一个“实时预览不可用”的空预览，供上层降级使用（`PreviewMode::Unavailable`）。
pub fn unavailable_preview(note: impl Into<String>) -> CandidatePreview {
    CandidatePreview {
        mode: PreviewMode::Unavailable,
        items: Vec::new(),
        note: Some(note.into()),
    }
}

/// 探测 librime 是否可加载（不初始化、不部署）。UI 可据此提前展示降级提示。
pub fn probe_librime() -> Result<crate::loader::LibraryProbe, FfiError> {
    crate::loader::probe()
}

/// 隔离会话候选预览引擎。
pub struct RimePreview {
    inner: Arc<PreviewInner>,
}

struct PreviewInner {
    /// 全局引擎守卫：Drop 时先 `RimeFinalize`（再释放全局槽位）。
    /// 必须声明在 `_storage` / `home` 之前——Rust 按声明顺序 drop 字段，
    /// 而 finalize 会读取 traits 里的 C 字符串与 user_data_dir。
    _guard: EngineGuard,
    /// 持有的 C 字符串，必须在 librime 存活期间有效（`RimeTraits` 里的指针）。
    _storage: TraitsStorage,
    home: Arc<IsolatedRimeHome>,
    lib: Arc<RimeLib>,
    /// 串行化所有 librime 调用（会话状态不是为并发调用设计的）。
    call_lock: Mutex<()>,
}

impl std::fmt::Debug for RimePreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RimePreview")
            .field("library", &self.inner.lib.path())
            .field("home", &self.inner.home.root())
            .field("schema", &self.inner.home.schema_id())
            .finish()
    }
}

impl RimePreview {
    /// 按 [`crate::ENV_LIBRIME`] → 默认库名加载 librime，初始化隔离会话并部署。
    ///
    /// 动态库句柄进程级缓存（见 [`crate::loader::shared_library`]）：重建预览引擎
    /// 只重跑 setup/initialize，不卸载库。
    pub fn open(home: IsolatedRimeHome) -> Result<Self, FfiError> {
        let lib = crate::loader::shared_library()?;
        Self::open_with_library(home, lib)
    }

    /// 显式传入已加载的动态库（测试/Windows 上从 Weasel 目录加载时使用）。
    pub fn open_with_library(home: IsolatedRimeHome, lib: Arc<RimeLib>) -> Result<Self, FfiError> {
        let home = Arc::new(home);

        // 先建 C 字符串存储、再拿引擎守卫：错误路径上局部变量按逆序 drop，
        // 保证 `RimeFinalize` 先于 traits 里的 C 字符串释放。
        let storage = TraitsStorage::new(&home)?;
        let mut guard = EngineGuard::acquire(Arc::clone(&lib))?;
        let mut traits = storage.traits();

        lib.set_notification_handler(Some(deploy_notification_handler))?;
        lib.setup(&mut traits)?;
        lib.initialize(&mut traits)?;
        guard.mark_initialized();

        // 隔离目录首次使用时部署一次；重开同一个 home 时跳过（build/ 已就绪）。
        let marker = home.build_dir().join(".rimetools-preview-deployed");
        if !marker.exists() {
            DEPLOY_STATE.store(DEPLOY_UNKNOWN, Ordering::SeqCst);
            let started = lib.start_maintenance(true)?;
            if started {
                lib.join_maintenance_thread()?;
            }
            if DEPLOY_STATE.load(Ordering::SeqCst) == DEPLOY_FAILURE {
                return Err(FfiError::Deploy {
                    reason: format!(
                        "librime 报告部署失败（隔离目录：{}，日志：{}）",
                        home.root().display(),
                        home.log_dir().display()
                    ),
                });
            }
            std::fs::write(&marker, b"1")?;
        }

        // 验证方案真的能加载，否则上层应立即降级。
        validate_schema(&lib, home.schema_id())?;

        Ok(Self {
            inner: Arc::new(PreviewInner {
                _guard: guard,
                _storage: storage,
                home,
                lib,
                call_lock: Mutex::new(()),
            }),
        })
    }

    /// 实际加载的 librime 路径。
    pub fn library_path(&self) -> &Path {
        self.inner.lib.path()
    }

    /// 隔离目录（调试/日志用）。
    pub fn home(&self) -> &IsolatedRimeHome {
        &self.inner.home
    }

    pub fn schema_id(&self) -> &str {
        self.inner.home.schema_id()
    }

    /// 预览一个编码：前 N 项候选（N = `menu/page_size`）。
    pub fn preview(
        &self,
        code: &str,
        target_text: Option<&str>,
    ) -> Result<CandidatePreview, FfiError> {
        self.preview_request(&PreviewRequest {
            code,
            target_text,
            max_items: None,
        })
    }

    /// 预览一个编码（带完整参数）。
    pub fn preview_request(
        &self,
        request: &PreviewRequest<'_>,
    ) -> Result<CandidatePreview, FfiError> {
        let _lock = self
            .inner
            .call_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let lib = Arc::clone(&self.inner.lib);
        let session_id = lib.create_session()?;
        let session = SessionGuard::new(Arc::clone(&lib), session_id);

        let selected = lib.select_schema(session.id(), self.inner.home.schema_id())?;
        if !selected {
            return Err(FfiError::SchemaSelect {
                schema_id: self.inner.home.schema_id().to_owned(),
                reason: "RimeSelectSchema 返回 false".to_owned(),
            });
        }
        // 预览永远是中文候选，不跟随被模拟的 ascii_mode 状态。
        let _ = lib.set_option(session.id(), "ascii_mode", false);
        lib.clear_composition(session.id())?;

        let mut io = LibrimeSessionIo {
            lib: Arc::clone(&lib),
            session_id: session.id(),
        };
        let capture = collect_candidates(&mut io, request.code, request.max_items)?;

        let items = capture
            .candidates
            .into_iter()
            .map(|candidate| Candidate {
                is_target: request
                    .target_text
                    .is_some_and(|target| target == candidate.text),
                text: candidate.text,
                comment: candidate.comment,
                // librime 的 context API 不暴露候选权重，只能留空；静态预览路径负责权重。
                weight: None,
            })
            .collect();

        Ok(CandidatePreview {
            mode: PreviewMode::Live,
            items,
            note: capture.note,
        })
    }
}

/// 真实 librime 会话 I/O：逐键模拟 + 读 commit/context。
struct LibrimeSessionIo {
    lib: Arc<RimeLib>,
    session_id: RimeSessionId,
}

impl SessionIo for LibrimeSessionIo {
    fn step(&mut self, key: char) -> Result<StepResult, FfiError> {
        let accepted = self
            .lib
            .simulate_key_sequence(self.session_id, &key.to_string())
            .map_err(|err| match err {
                FfiError::InvalidCode { .. } => FfiError::SimulateKey {
                    key: key.to_string(),
                    reason: "编码含 NUL".to_owned(),
                },
                other => other,
            })?;
        // 先取 commit：auto_select 上屏后 context 会被清空。
        let committed = self.lib.get_commit(self.session_id)?;
        let context = self.lib.get_context(self.session_id)?;
        Ok(StepResult {
            accepted,
            committed,
            context,
        })
    }
}

/// 方案可用性校验：建一个临时会话、切方案、读回 schema id。
fn validate_schema(lib: &Arc<RimeLib>, schema_id: &str) -> Result<(), FfiError> {
    let session_id = lib.create_session()?;
    let session = SessionGuard::new(Arc::clone(lib), session_id);
    let selected = lib.select_schema(session.id(), schema_id)?;
    let current = lib.current_schema(session.id()).ok().flatten();
    if !selected || current.as_deref() != Some(schema_id) {
        return Err(FfiError::SchemaSelect {
            schema_id: schema_id.to_owned(),
            reason: format!(
                "select_schema={selected}, current_schema={:?}；隔离目录中是否缺少该方案或码表？",
                current
            ),
        });
    }
    Ok(())
}

/// `RimeTraits` 背后的 C 字符串存储。librime 可能长期持有这些指针，必须活到 finalize。
struct TraitsStorage {
    shared_data_dir: CString,
    user_data_dir: CString,
    distribution_name: CString,
    distribution_code_name: CString,
    distribution_version: CString,
    app_name: CString,
    log_dir: CString,
}

impl TraitsStorage {
    fn new(home: &IsolatedRimeHome) -> Result<Self, FfiError> {
        fn cstring(label: &str, value: &Path) -> Result<CString, FfiError> {
            CString::new(value.to_string_lossy().as_bytes()).map_err(|_| FfiError::Home {
                reason: format!("{label} 路径含 NUL：{}", value.display()),
            })
        }
        Ok(Self {
            shared_data_dir: cstring("shared_data_dir", home.shared_dir())?,
            user_data_dir: cstring("user_data_dir", home.user_dir())?,
            distribution_name: CString::new("RimeTools").expect("静态字符串"),
            distribution_code_name: CString::new("rimetools").expect("静态字符串"),
            distribution_version: CString::new(env!("CARGO_PKG_VERSION")).expect("版本号"),
            app_name: CString::new(format!("rime.{}", home.app_name())).expect("app name"),
            log_dir: cstring("log_dir", home.log_dir())?,
        })
    }

    /// 构造 `RimeTraits`。只读借用自身字段，寿命由 `self` 保证。
    fn traits(&self) -> RimeTraits {
        let mut traits = RimeTraits::nulled();
        traits.shared_data_dir = self.shared_data_dir.as_ptr();
        traits.user_data_dir = self.user_data_dir.as_ptr();
        traits.distribution_name = self.distribution_name.as_ptr();
        traits.distribution_code_name = self.distribution_code_name.as_ptr();
        traits.distribution_version = self.distribution_version.as_ptr();
        traits.app_name = self.app_name.as_ptr();
        traits.modules = std::ptr::null();
        traits.log_dir = self.log_dir.as_ptr();
        // staging 目录默认 ${user_data_dir}/build；prebuilt 目录默认 ${shared_data_dir}/build。
        traits.prebuilt_data_dir = std::ptr::null();
        traits.staging_dir = std::ptr::null();
        traits
    }
}

const DEPLOY_UNKNOWN: u8 = 0;
const DEPLOY_STARTED: u8 = 1;
const DEPLOY_SUCCESS: u8 = 2;
const DEPLOY_FAILURE: u8 = 3;

static DEPLOY_STATE: AtomicU8 = AtomicU8::new(DEPLOY_UNKNOWN);

/// librime 部署通知回调（`session_id == 0`, `message_type == "deploy"`）。
unsafe extern "C" fn deploy_notification_handler(
    _context_object: *mut c_void,
    _session_id: RimeSessionId,
    message_type: *const c_char,
    message_value: *const c_char,
) {
    if message_type.is_null() || message_value.is_null() {
        return;
    }
    let message_type = std::ffi::CStr::from_ptr(message_type).to_string_lossy();
    if message_type != "deploy" {
        return;
    }
    let value = std::ffi::CStr::from_ptr(message_value).to_string_lossy();
    match value.as_ref() {
        "start" => DEPLOY_STATE.store(DEPLOY_STARTED, Ordering::SeqCst),
        "success" => DEPLOY_STATE.store(DEPLOY_SUCCESS, Ordering::SeqCst),
        "failure" => DEPLOY_STATE.store(DEPLOY_FAILURE, Ordering::SeqCst),
        _ => {}
    }
}

/// 供集成测试/诊断：读取最近一次隔离部署的通知状态。
pub fn last_deploy_state() -> &'static str {
    match DEPLOY_STATE.load(Ordering::SeqCst) {
        DEPLOY_STARTED => "start",
        DEPLOY_SUCCESS => "success",
        DEPLOY_FAILURE => "failure",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_preview_has_mode_and_note() {
        let preview = unavailable_preview("librime 缺失");
        assert_eq!(preview.mode, PreviewMode::Unavailable);
        assert!(preview.items.is_empty());
        assert_eq!(preview.note.as_deref(), Some("librime 缺失"));
    }

    #[test]
    fn request_builder_works() {
        let request = PreviewRequest::new("abcd").target_text("甲").max_items(3);
        assert_eq!(request.code, "abcd");
        assert_eq!(request.target_text, Some("甲"));
        assert_eq!(request.max_items, Some(3));
    }
}
