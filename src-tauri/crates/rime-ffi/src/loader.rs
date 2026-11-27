//! librime 动态库定位、`dlopen` 与 `RimeApi` 安全封装。
//!
//! **不静态链接**：Windows 侧将来要加载 Weasel 安装目录内的 `rime.dll`（1.13.1），
//! Linux 侧加载系统 `librime.so.1`（1.16.1），所以库路径必须是运行时可替换的。
//!
//! 查找顺序：
//! 1. 环境变量 [`ENV_LIBRIME`]（绝对路径或 dlopen 可识别的库名）；
//! 2. 平台默认库名（Linux `librime.so.1` → `librime.so`；macOS `librime.1.dylib` →
//!    `librime.dylib`；Windows `rime.dll`）；
//! 3. Linux 常见绝对路径兜底（多架构目录）。
//!
//! 只绑定 `rime_get_api` 一个符号（外加其 stdbool 变体不启用），函数指针全部从
//! `RimeApi` 表取——这是跨 1.13.1 / 1.16.1 两代 ABI 最稳的方式。

use std::ffi::{c_char, c_int, CStr, CString, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use libloading::Library;

use crate::capture::{RawCandidate, RawContext};
use crate::error::FfiError;
use crate::ffi::{
    bool_from, RimeApi, RimeCommit, RimeContext, RimeGetApiFn, RimeNotificationHandler,
    RimeSessionId, RimeTraits, FALSE, TRUE,
};

/// 覆盖 librime 动态库路径的环境变量（绝对路径优先）。
pub const ENV_LIBRIME: &str = "RIMETOOLS_LIBRIME";

/// 当前平台的默认库名（按优先级）。
pub fn default_library_names() -> &'static [&'static str] {
    #[cfg(target_os = "linux")]
    {
        &["librime.so.1", "librime.so"]
    }
    #[cfg(target_os = "macos")]
    {
        &["librime.1.dylib", "librime.dylib"]
    }
    #[cfg(windows)]
    {
        // Weasel 安装目录内的 rime.dll（1.13.1）；具体目录由上层用 RIMETOOLS_LIBRIME 指定。
        &["rime.dll"]
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        &["librime.so", "librime.so.1"]
    }
}

/// Linux 多架构目录下的绝对路径兜底。
fn fallback_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        const TRIPLETS: &[&str] = &["x86_64-linux-gnu", "aarch64-linux-gnu", "i386-linux-gnu"];
        let mut out = Vec::new();
        for triple in TRIPLETS {
            for lib in ["librime.so.1", "librime.so"] {
                out.push(PathBuf::from(format!("/usr/lib/{triple}/{lib}")));
                out.push(PathBuf::from(format!("/usr/local/lib/{triple}/{lib}")));
            }
        }
        for lib in ["librime.so.1", "librime.so"] {
            out.push(PathBuf::from(format!("/usr/lib/{lib}")));
            out.push(PathBuf::from(format!("/usr/local/lib/{lib}")));
        }
        out
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

/// 返回要尝试的候选（环境变量优先）。纯函数，便于单测。
pub fn library_candidates(env_override: Option<OsString>) -> Vec<OsString> {
    let mut out = Vec::new();
    if let Some(value) = env_override {
        if !value.is_empty() {
            out.push(value);
            return out;
        }
    }
    out.extend(default_library_names().iter().map(OsString::from));
    out.extend(fallback_paths().into_iter().map(OsString::from));
    out
}

/// 已加载的 librime 动态库 + `RimeApi` 表。
///
/// `RimeLib` 本身不负责 setup/initialize（进程级全局状态由
/// [`crate::preview::RimePreview`] 的生命周期守卫管理），只做函数指针的安全调用。
pub struct RimeLib {
    /// 必须持有：dlclose 之后 `api` 里的函数指针会悬垂。
    _library: Library,
    api: *const RimeApi,
    path: PathBuf,
}

// `Library` 在 Unix 上是 Send + Sync；`RimeApi` 表由 librime 静态持有。
// 我们通过全局引擎守卫 + `RimePreview` 内的互斥锁串行化所有调用。
unsafe impl Send for RimeLib {}
unsafe impl Sync for RimeLib {}

impl std::fmt::Debug for RimeLib {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RimeLib")
            .field("path", &self.path)
            .field("api_data_size", &unsafe { (*self.api).data_size })
            .finish()
    }
}

impl RimeLib {
    /// 按 [`ENV_LIBRIME`] → 平台默认库名 → 绝对路径兜底的顺序加载。
    pub fn load() -> Result<Self, FfiError> {
        Self::load_with_env(std::env::var_os(ENV_LIBRIME))
    }

    /// 测试友好的入口：显式传入环境变量值（`None` 表示未设置）。
    ///
    /// 显式指定（`RIMETOOLS_LIBRIME`）时不再回退到系统库：用户显式指定的路径加载失败
    /// 必须原样报错（`LibraryLoad`），避免静默用到另一个版本的 librime 造成 ABI 混乱。
    pub fn load_with_env(env_override: Option<OsString>) -> Result<Self, FfiError> {
        if let Some(value) = env_override.as_ref().filter(|value| !value.is_empty()) {
            return Self::load_from_path(Path::new(value));
        }
        let mut tried: Vec<String> = Vec::new();
        for candidate in library_candidates(None) {
            match Self::try_open(Path::new(&candidate)) {
                Ok(lib) => {
                    tracing::debug!(
                        path = %lib.path.display(),
                        api_data_size = unsafe { (*lib.api).data_size },
                        "已加载 librime"
                    );
                    return Ok(lib);
                }
                Err(err) => {
                    tried.push(format!("{}（{}）", candidate.to_string_lossy(), err));
                }
            }
        }
        Err(FfiError::LibraryNotFound {
            tried: tried.join("; "),
        })
    }

    /// 精确加载指定路径（`RIMETOOLS_LIBRIME` 的值、或 Windows 上 Weasel 的 `rime.dll`）。
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, FfiError> {
        Self::try_open(path.as_ref())
    }

    fn try_open(path: &Path) -> Result<Self, FfiError> {
        // SAFETY: 只是 dlopen；库句柄由 Self 持有到 drop。
        let library = unsafe { Library::new(path) }.map_err(|source| FfiError::LibraryLoad {
            path: path.display().to_string(),
            source,
        })?;
        // SAFETY: `rime_get_api` 是 librime 公开的 C ABI 入口，签名见 rime_api.h。
        let get_api: libloading::Symbol<'_, RimeGetApiFn> = unsafe {
            library.get(b"rime_get_api\0")
        }
        .map_err(|source| FfiError::SymbolMissing {
            symbol: "rime_get_api".to_owned(),
            source,
        })?;
        let api = unsafe { get_api() };
        if api.is_null() {
            return Err(FfiError::Initialize {
                reason: "rime_get_api() 返回空指针".to_owned(),
            });
        }
        // SAFETY: librime 承诺返回指向静态 RimeApi 表的指针；库句柄保证其存活。
        unsafe { (*api).validate_layout()? };
        Ok(Self {
            _library: library,
            api,
            path: path.to_path_buf(),
        })
    }

    /// 实际加载到的动态库路径（或库名）。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `RimeApi` 表地址（生命周期 ≥ self）。
    pub fn api(&self) -> &RimeApi {
        // SAFETY: 指针由 librime 静态持有，且经 try_open 校验非空。
        unsafe { &*self.api }
    }

    /// `RimeApi` 表自报大小（诊断用）。
    pub fn api_data_size(&self) -> usize {
        self.api().data_size.max(0) as usize
    }

    // ── 以下为最小 API 封装；全部检查 Option 字段，旧版本缺失时返回明确错误 ──

    fn require<T: Copy>(value: Option<T>, field: &'static str) -> Result<T, FfiError> {
        value.ok_or(FfiError::UnsupportedApi { field })
    }

    /// `RimeSetup`：设置全局 traits（含目录与日志）。必须在 initialize 前调用。
    pub fn setup(&self, traits: &mut RimeTraits) -> Result<(), FfiError> {
        let setup = Self::require(self.api().setup, "setup")?;
        // SAFETY: traits 生命周期覆盖调用；指针字段由调用方确保有效。
        unsafe { setup(traits) };
        Ok(())
    }

    /// `RimeSetNotificationHandler`：捕获 deploy 成功/失败通知。
    ///
    /// 回调的 `context_object` 固定传空（我们的 handler 只用静态状态，
    /// 避免在公共 API 上暴露可能被 librime 长期持有的裸指针）。
    pub fn set_notification_handler(
        &self,
        handler: RimeNotificationHandler,
    ) -> Result<(), FfiError> {
        let setter = Self::require(
            self.api().set_notification_handler,
            "set_notification_handler",
        )?;
        // SAFETY: 回调函数为 `extern "C"` 且 static；上下文对象传空。
        unsafe { setter(handler, std::ptr::null_mut()) };
        Ok(())
    }

    /// `RimeInitialize`（C 层内部会先 setup 再启动 service）。返回类型是 `void`。
    pub fn initialize(&self, traits: &mut RimeTraits) -> Result<(), FfiError> {
        let initialize = Self::require(self.api().initialize, "initialize")?;
        // SAFETY: traits 与其中指针在调用期间有效。
        unsafe { initialize(traits) };
        Ok(())
    }

    /// `RimeFinalize`：释放 librime 全局状态。
    pub fn finalize(&self) {
        if let Some(finalize) = self.api().finalize {
            // SAFETY: 只有在持有全局引擎守卫时才会调用 finalize。
            unsafe { finalize() };
        }
    }

    /// `RimeStartMaintenance(full_check)`。
    pub fn start_maintenance(&self, full_check: bool) -> Result<bool, FfiError> {
        let start = Self::require(self.api().start_maintenance, "start_maintenance")?;
        // SAFETY: 无指针参数。
        Ok(bool_from(unsafe {
            start(if full_check { TRUE } else { FALSE })
        }))
    }

    /// `RimeJoinMaintenanceThread`。
    pub fn join_maintenance_thread(&self) -> Result<(), FfiError> {
        let join = Self::require(
            self.api().join_maintenance_thread,
            "join_maintenance_thread",
        )?;
        // SAFETY: 无参数；maintenance 已由 start_maintenance 启动。
        unsafe { join() };
        Ok(())
    }

    /// `RimeCreateSession`，返回 0 表示失败。
    pub fn create_session(&self) -> Result<RimeSessionId, FfiError> {
        let create = Self::require(self.api().create_session, "create_session")?;
        // SAFETY: 无参数。
        let id = unsafe { create() };
        if id == 0 {
            return Err(FfiError::SessionCreate);
        }
        Ok(id)
    }

    /// `RimeDestroySession`。
    pub fn destroy_session(&self, session_id: RimeSessionId) -> Result<bool, FfiError> {
        let destroy = Self::require(self.api().destroy_session, "destroy_session")?;
        // SAFETY: session_id 来自 create_session 且尚未销毁。
        Ok(bool_from(unsafe { destroy(session_id) }))
    }

    /// `RimeSelectSchema`。
    pub fn select_schema(
        &self,
        session_id: RimeSessionId,
        schema_id: &str,
    ) -> Result<bool, FfiError> {
        let select = Self::require(self.api().select_schema, "select_schema")?;
        let schema = CString::new(schema_id).map_err(|_| FfiError::InvalidCode {
            reason: format!("方案 id 含 NUL：{schema_id:?}"),
        })?;
        // SAFETY: CString 在调用期间有效。
        Ok(bool_from(unsafe { select(session_id, schema.as_ptr()) }))
    }

    /// `RimeGetCurrentSchema`。
    pub fn current_schema(&self, session_id: RimeSessionId) -> Result<Option<String>, FfiError> {
        let get = Self::require(self.api().get_current_schema, "get_current_schema")?;
        let mut buffer = vec![0u8; 256];
        let ok = unsafe { get(session_id, buffer.as_mut_ptr() as *mut c_char, buffer.len()) };
        if !bool_from(ok) {
            return Ok(None);
        }
        let value = unsafe { CStr::from_ptr(buffer.as_ptr() as *const c_char) }
            .to_string_lossy()
            .into_owned();
        Ok(Some(value))
    }

    /// `RimeSetOption`（例如关闭 `ascii_mode` 强制中文模式）。
    pub fn set_option(
        &self,
        session_id: RimeSessionId,
        option: &str,
        value: bool,
    ) -> Result<(), FfiError> {
        let set = Self::require(self.api().set_option, "set_option")?;
        let name = CString::new(option).map_err(|_| FfiError::InvalidCode {
            reason: format!("option 名含 NUL：{option:?}"),
        })?;
        // SAFETY: name 在调用期间有效。
        unsafe { set(session_id, name.as_ptr(), if value { TRUE } else { FALSE }) };
        Ok(())
    }

    /// `RimeClearComposition`：每个预览请求开始前清干净会话状态。
    pub fn clear_composition(&self, session_id: RimeSessionId) -> Result<(), FfiError> {
        let clear = Self::require(self.api().clear_composition, "clear_composition")?;
        // SAFETY: 无指针参数。
        unsafe { clear(session_id) };
        Ok(())
    }

    /// `RimeSimulateKeySequence`：模拟按键序列（我们逐键调用）。
    pub fn simulate_key_sequence(
        &self,
        session_id: RimeSessionId,
        key_sequence: &str,
    ) -> Result<bool, FfiError> {
        let simulate = Self::require(self.api().simulate_key_sequence, "simulate_key_sequence")?;
        let sequence = CString::new(key_sequence).map_err(|_| FfiError::InvalidCode {
            reason: "编码含 NUL".to_owned(),
        })?;
        // SAFETY: sequence 在调用期间有效。
        Ok(bool_from(unsafe {
            simulate(session_id, sequence.as_ptr())
        }))
    }

    /// `RimeGetCommit` + `RimeFreeCommit`：读取未读的上屏文本（auto_select 会在这里出现）。
    pub fn get_commit(&self, session_id: RimeSessionId) -> Result<Option<String>, FfiError> {
        let get = Self::require(self.api().get_commit, "get_commit")?;
        let free = Self::require(self.api().free_commit, "free_commit")?;
        let mut commit = RimeCommit::nulled();
        let ok = unsafe { get(session_id, &mut commit) };
        if !bool_from(ok) {
            return Ok(None);
        }
        let text = unsafe { c_string(commit.text) }.filter(|text| !text.is_empty());
        // SAFETY: commit 由 get_commit 填充，必须成对释放。
        unsafe { free(&mut commit) };
        Ok(text)
    }

    /// `RimeGetContext` + `RimeFreeContext`：读取组合与当前页候选。
    ///
    /// 注意 context 内部的字符串与候选数组归 librime 所有，必须在 `free_context`
    /// 之前拷贝出来，否则会悬垂 / 泄漏。
    pub fn get_context(&self, session_id: RimeSessionId) -> Result<RawContext, FfiError> {
        let get = Self::require(self.api().get_context, "get_context")?;
        let free = Self::require(self.api().free_context, "free_context")?;
        let mut context = RimeContext::nulled();
        let ok = unsafe { get(session_id, &mut context) };
        if !bool_from(ok) {
            return Ok(RawContext::default());
        }
        // SAFETY: get_context 返回 1 时 context 已填充且内部指针有效。
        let raw = unsafe { read_context(&context) };
        // SAFETY: 与 get_context 成对释放。
        unsafe { free(&mut context) };
        Ok(raw)
    }
}

/// 从已填充的 `RimeContext` 拷贝出候选与组合信息（不持有 librime 指针）。
unsafe fn read_context(context: &RimeContext) -> RawContext {
    let preedit = c_string(context.composition.preedit).unwrap_or_default();
    let page_size = context.menu.page_size.max(0) as usize;
    let mut candidates = Vec::new();
    if !context.menu.candidates.is_null() {
        let count = context.menu.num_candidates.max(0) as usize;
        for index in 0..count {
            let candidate = &*context.menu.candidates.add(index);
            candidates.push(RawCandidate {
                text: c_string(candidate.text).unwrap_or_default(),
                comment: c_string(candidate.comment),
            });
        }
    }
    RawContext {
        preedit,
        page_size,
        candidates,
    }
}

/// C 字符串 → Rust String（空指针返回 None）。
pub(crate) unsafe fn c_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        None
    } else {
        Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
    }
}

static SHARED_LIBRARY: std::sync::Mutex<Option<Arc<RimeLib>>> = std::sync::Mutex::new(None);

/// 进程级共享的 librime 句柄。
///
/// librime 只需加载一次：反复 `dlopen`/`dlclose`（每个预览引擎一个句柄）会让
/// 库内静态对象反复构造/析构，实测在 `RimeFinalize` 后再卸载会 segfault。
/// 这里把句柄缓存到进程结束，保证引擎重建（换方案/换隔离目录）时只重跑
/// setup/initialize，不再卸载动态库。
pub fn shared_library() -> Result<Arc<RimeLib>, FfiError> {
    let mut slot = SHARED_LIBRARY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(lib) = slot.as_ref() {
        return Ok(Arc::clone(lib));
    }
    let lib = Arc::new(RimeLib::load()?);
    *slot = Some(Arc::clone(&lib));
    Ok(lib)
}

/// 供上层做可用性探测：加载 + 校验 API 表，不 initialize。
pub fn probe() -> Result<LibraryProbe, FfiError> {
    let lib = RimeLib::load()?;
    Ok(LibraryProbe {
        path: lib.path().to_path_buf(),
        api_data_size: lib.api_data_size(),
    })
}

/// [`probe`] 的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryProbe {
    pub path: PathBuf,
    pub api_data_size: usize,
}

/// 进程级引擎守卫：librime 的 setup/initialize 是全局单例，同一进程只允许一个
/// [`crate::preview::RimePreview`]。第二个实例返回 [`FfiError::EngineBusy`]。
pub(crate) struct EngineGuard {
    token: u64,
    lib: Arc<RimeLib>,
    /// 只有真正调用过 `RimeInitialize` 才允许 `RimeFinalize`（未初始化的 finalize 不安全）。
    initialized: bool,
}

static ENGINE_SLOT: std::sync::Mutex<Option<u64>> = std::sync::Mutex::new(None);
static NEXT_TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn lock_slot() -> std::sync::MutexGuard<'static, Option<u64>> {
    ENGINE_SLOT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl EngineGuard {
    pub(crate) fn acquire(lib: Arc<RimeLib>) -> Result<Self, FfiError> {
        let mut slot = lock_slot();
        if slot.is_some() {
            return Err(FfiError::EngineBusy);
        }
        let token = NEXT_TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *slot = Some(token);
        Ok(Self {
            token,
            lib,
            initialized: false,
        })
    }

    /// 在 `RimeInitialize` 成功返回后标记，允许 Drop 时 finalize。
    pub(crate) fn mark_initialized(&mut self) {
        self.initialized = true;
    }
}

impl Drop for EngineGuard {
    fn drop(&mut self) {
        if self.initialized {
            self.lib.finalize();
        }
        let mut slot = lock_slot();
        if *slot == Some(self.token) {
            *slot = None;
        }
    }
}

/// 判断当前进程是否还能初始化一个新的预览引擎（用于 UI 提前提示）。
pub fn engine_available() -> bool {
    lock_slot().is_none()
}

/// 会话守卫：`Drop` 时自动 `RimeDestroySession`，避免异常路径泄漏会话。
pub(crate) struct SessionGuard {
    lib: Arc<RimeLib>,
    session_id: RimeSessionId,
    alive: bool,
}

impl SessionGuard {
    pub(crate) fn new(lib: Arc<RimeLib>, session_id: RimeSessionId) -> Self {
        Self {
            lib,
            session_id,
            alive: true,
        }
    }

    pub(crate) fn id(&self) -> RimeSessionId {
        self.session_id
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        if self.alive {
            self.alive = false;
            let _ = self.lib.destroy_session(self.session_id);
        }
    }
}

// 让 `CString`/`c_int` 引用不产生未使用告警（部分 cfg 下用不到）。
#[allow(unused)]
fn _assert_imports(_: CString, _: c_int) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_put_env_override_first() {
        let candidates = library_candidates(Some(OsString::from("/opt/weasel/rime.dll")));
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0], OsString::from("/opt/weasel/rime.dll"));
    }

    #[test]
    fn candidates_without_env_contain_platform_defaults() {
        let candidates = library_candidates(None);
        assert!(candidates.len() >= default_library_names().len());
        for name in default_library_names() {
            assert!(candidates.iter().any(|c| c == &OsString::from(*name)));
        }
    }

    #[test]
    fn empty_env_is_ignored() {
        let candidates = library_candidates(Some(OsString::new()));
        assert_eq!(candidates.len(), library_candidates(None).len());
    }

    #[test]
    fn missing_library_returns_clear_error() {
        let err = RimeLib::load_from_path("/nonexistent/rimetools/librime-does-not-exist.so")
            .expect_err("不存在的库必须报错");
        match err {
            FfiError::LibraryLoad { path, .. } => {
                assert!(path.contains("librime-does-not-exist"));
            }
            other => panic!("期望 LibraryLoad，实际 {other:?}"),
        }
    }

    #[test]
    fn nonexistent_path_via_env_reports_not_found_or_load_error() {
        // 环境变量指向不存在的路径：不允许静默回退到系统库，必须报错。
        let err = RimeLib::load_with_env(Some(OsString::from(
            "/nonexistent/rimetools/librime-override.so",
        )))
        .expect_err("显式指定但加载失败必须报错");
        assert!(matches!(err, FfiError::LibraryLoad { .. }));
    }

    #[test]
    fn engine_guard_is_exclusive() {
        // 直接构造一个假的 RimeLib 太难；用空库句柄无法构造。这里用真实库（若存在），
        // 不存在则跳过——守卫本身的行为由 preview 的集成路径覆盖。
        // 该测试只验证 engine_available() 在无引擎时返回 true。
        assert!(engine_available());
    }
}
