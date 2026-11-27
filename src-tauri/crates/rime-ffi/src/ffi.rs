//! librime C API 的最小绑定。
//!
//! 以真实头文件 `/usr/include/rime_api.h`（librime 1.16.1）为准逐字段转写。
//!
//! ## 为什么只声明到 `simulate_key_sequence` 为止
//!
//! librime ≥ 1.10 的 C ABI 入口只有 `RIME_API RimeApi* rime_get_api(void)`，
//! 旧版全局符号（`RimeSetup`/`RimeInitialize`/…）不再以 C 符号导出（1.16.1 的
//! `.so` 里只剩 C++ mangled 名字）。因此动态加载后必须按 `RimeApi` 表的布局取函数指针。
//!
//! `RimeApi` 是“只追加”的版本化结构：字段顺序自 1.0 起不变，新字段一律追加在尾部。
//! 我们只声明用到的最长前缀（到 `simulate_key_sequence`，位于 1.16.1 结构的中段），
//! 这样同一份布局对 1.13.1（Weasel 0.17.4）和 1.16.1（fcitx5-rime 5.1.13）都成立；
//! 同时用 `RimeApi::data_size` 做运行时校验，避免图书馆版本过旧时越界读取。
//!
//! 函数指针字段用 `Option<unsafe extern "C" fn(...)>`，与 C 的裸函数指针同布局
//! （函数指针非空，`Option` 用空指针 niche 表示），同时便于检测旧版本缺失的 API。

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use std::ffi::{c_char, c_int, c_void};

/// librime 的 C `Bool`（`int`）；stdbool 变体（`rime_get_api_stdbool`）不使用。
pub type Bool = c_int;
pub const TRUE: Bool = 1;
pub const FALSE: Bool = 0;

/// 会话 id（`uintptr_t`）。
pub type RimeSessionId = usize;

/// `Bool` → Rust `bool`。
pub fn bool_from(v: Bool) -> bool {
    v != FALSE
}

/// librime 通知回调：`on_notification(context_object, session_id, message_type, message_value)`。
pub type RimeNotificationHandler =
    Option<unsafe extern "C" fn(*mut c_void, RimeSessionId, *const c_char, *const c_char)>;

/// `RimeTraits`（rime_api.h v1.6+ 布局）。
///
/// 调用方负责让其中所有 `*const c_char` 指向的内存在 librime 存活期间有效
/// （见 `preview::TraitsStorage`）。
#[repr(C)]
#[derive(Debug)]
pub struct RimeTraits {
    pub data_size: c_int,
    pub shared_data_dir: *const c_char,
    pub user_data_dir: *const c_char,
    pub distribution_name: *const c_char,
    pub distribution_code_name: *const c_char,
    pub distribution_version: *const c_char,
    pub app_name: *const c_char,
    pub modules: *const *const c_char,
    pub min_log_level: c_int,
    pub log_dir: *const c_char,
    pub prebuilt_data_dir: *const c_char,
    pub staging_dir: *const c_char,
}

impl RimeTraits {
    /// 等价于 C 宏 `RIME_STRUCT_INIT(RimeTraits, var)`。
    pub fn nulled() -> Self {
        let mut traits: Self = unsafe { std::mem::zeroed() };
        traits.data_size = (std::mem::size_of::<Self>() - std::mem::size_of::<c_int>()) as c_int;
        traits
    }
}

#[repr(C)]
#[derive(Debug, Default)]
pub struct RimeComposition {
    pub length: c_int,
    pub cursor_pos: c_int,
    pub sel_start: c_int,
    pub sel_end: c_int,
    pub preedit: *mut c_char,
}

#[repr(C)]
#[derive(Debug, Default)]
pub struct RimeCandidate {
    pub text: *mut c_char,
    pub comment: *mut c_char,
    pub reserved: *mut c_void,
}

#[repr(C)]
#[derive(Debug, Default)]
pub struct RimeMenu {
    pub page_size: c_int,
    pub page_no: c_int,
    pub is_last_page: Bool,
    pub highlighted_candidate_index: c_int,
    pub num_candidates: c_int,
    pub candidates: *mut RimeCandidate,
    pub select_keys: *mut c_char,
}

#[repr(C)]
#[derive(Debug, Default)]
pub struct RimeCommit {
    pub data_size: c_int,
    pub text: *mut c_char,
}

impl RimeCommit {
    pub fn nulled() -> Self {
        let mut commit: Self = unsafe { std::mem::zeroed() };
        commit.data_size = (std::mem::size_of::<Self>() - std::mem::size_of::<c_int>()) as c_int;
        commit
    }
}

#[repr(C)]
#[derive(Debug, Default)]
pub struct RimeContext {
    pub data_size: c_int,
    pub composition: RimeComposition,
    pub menu: RimeMenu,
    pub commit_text_preview: *mut c_char,
    pub select_labels: *mut *mut c_char,
}

impl RimeContext {
    /// 等价于 C 宏 `RIME_STRUCT_INIT(RimeContext, ctx)`。
    pub fn nulled() -> Self {
        let mut ctx: Self = unsafe { std::mem::zeroed() };
        ctx.data_size = (std::mem::size_of::<Self>() - std::mem::size_of::<c_int>()) as c_int;
        ctx
    }
}

#[repr(C)]
#[derive(Debug, Default)]
pub struct RimeStatus {
    pub data_size: c_int,
    pub schema_id: *mut c_char,
    pub schema_name: *mut c_char,
    pub is_disabled: Bool,
    pub is_composing: Bool,
    pub is_ascii_mode: Bool,
    pub is_full_shape: Bool,
    pub is_simplified: Bool,
    pub is_traditional: Bool,
    pub is_ascii_punct: Bool,
}

#[repr(C)]
#[derive(Debug, Default)]
pub struct RimeSchemaListItem {
    pub schema_id: *mut c_char,
    pub name: *mut c_char,
    pub reserved: *mut c_void,
}

#[repr(C)]
#[derive(Debug, Default)]
pub struct RimeSchemaList {
    pub size: usize,
    pub list: *mut RimeSchemaListItem,
}

#[repr(C)]
#[derive(Debug, Default)]
pub struct RimeConfig {
    pub ptr: *mut c_void,
}

#[repr(C)]
#[derive(Debug, Default)]
pub struct RimeConfigIterator {
    pub list: *mut c_void,
    pub map: *mut c_void,
    pub index: c_int,
    pub key: *const c_char,
    pub path: *const c_char,
}

/// `RimeApi` 的用到的最长前缀（字段顺序与 rime_api.h 完全一致）。
#[repr(C)]
#[derive(Debug)]
pub struct RimeApi {
    pub data_size: c_int,

    pub setup: Option<unsafe extern "C" fn(*mut RimeTraits)>,
    pub set_notification_handler:
        Option<unsafe extern "C" fn(RimeNotificationHandler, *mut c_void)>,

    // entry and exit
    pub initialize: Option<unsafe extern "C" fn(*mut RimeTraits)>,
    pub finalize: Option<unsafe extern "C" fn()>,

    pub start_maintenance: Option<unsafe extern "C" fn(Bool) -> Bool>,
    pub is_maintenance_mode: Option<unsafe extern "C" fn() -> Bool>,
    pub join_maintenance_thread: Option<unsafe extern "C" fn()>,

    // deployment
    pub deployer_initialize: Option<unsafe extern "C" fn(*mut RimeTraits)>,
    pub prebuild: Option<unsafe extern "C" fn() -> Bool>,
    pub deploy: Option<unsafe extern "C" fn() -> Bool>,
    pub deploy_schema: Option<unsafe extern "C" fn(*const c_char) -> Bool>,
    pub deploy_config_file: Option<unsafe extern "C" fn(*const c_char, *const c_char) -> Bool>,
    pub sync_user_data: Option<unsafe extern "C" fn() -> Bool>,

    // session management
    pub create_session: Option<unsafe extern "C" fn() -> RimeSessionId>,
    pub find_session: Option<unsafe extern "C" fn(RimeSessionId) -> Bool>,
    pub destroy_session: Option<unsafe extern "C" fn(RimeSessionId) -> Bool>,
    pub cleanup_stale_sessions: Option<unsafe extern "C" fn()>,
    pub cleanup_all_sessions: Option<unsafe extern "C" fn()>,

    // input
    pub process_key: Option<unsafe extern "C" fn(RimeSessionId, c_int, c_int) -> Bool>,
    pub commit_composition: Option<unsafe extern "C" fn(RimeSessionId) -> Bool>,
    pub clear_composition: Option<unsafe extern "C" fn(RimeSessionId)>,

    // output
    pub get_commit: Option<unsafe extern "C" fn(RimeSessionId, *mut RimeCommit) -> Bool>,
    pub free_commit: Option<unsafe extern "C" fn(*mut RimeCommit) -> Bool>,
    pub get_context: Option<unsafe extern "C" fn(RimeSessionId, *mut RimeContext) -> Bool>,
    pub free_context: Option<unsafe extern "C" fn(*mut RimeContext) -> Bool>,
    pub get_status: Option<unsafe extern "C" fn(RimeSessionId, *mut RimeStatus) -> Bool>,
    pub free_status: Option<unsafe extern "C" fn(*mut RimeStatus) -> Bool>,

    // runtime options
    pub set_option: Option<unsafe extern "C" fn(RimeSessionId, *const c_char, Bool)>,
    pub get_option: Option<unsafe extern "C" fn(RimeSessionId, *const c_char) -> Bool>,
    pub set_property: Option<unsafe extern "C" fn(RimeSessionId, *const c_char, *const c_char)>,
    pub get_property:
        Option<unsafe extern "C" fn(RimeSessionId, *const c_char, *mut c_char, usize) -> Bool>,

    pub get_schema_list: Option<unsafe extern "C" fn(*mut RimeSchemaList) -> Bool>,
    pub free_schema_list: Option<unsafe extern "C" fn(*mut RimeSchemaList)>,
    pub get_current_schema: Option<unsafe extern "C" fn(RimeSessionId, *mut c_char, usize) -> Bool>,
    pub select_schema: Option<unsafe extern "C" fn(RimeSessionId, *const c_char) -> Bool>,

    // configuration
    pub schema_open: Option<unsafe extern "C" fn(*const c_char, *mut RimeConfig) -> Bool>,
    pub config_open: Option<unsafe extern "C" fn(*const c_char, *mut RimeConfig) -> Bool>,
    pub config_close: Option<unsafe extern "C" fn(*mut RimeConfig) -> Bool>,
    pub config_get_bool:
        Option<unsafe extern "C" fn(*mut RimeConfig, *const c_char, *mut Bool) -> Bool>,
    pub config_get_int:
        Option<unsafe extern "C" fn(*mut RimeConfig, *const c_char, *mut c_int) -> Bool>,
    pub config_get_double:
        Option<unsafe extern "C" fn(*mut RimeConfig, *const c_char, *mut f64) -> Bool>,
    pub config_get_string:
        Option<unsafe extern "C" fn(*mut RimeConfig, *const c_char, *mut c_char, usize) -> Bool>,
    pub config_get_cstring:
        Option<unsafe extern "C" fn(*mut RimeConfig, *const c_char) -> *const c_char>,
    pub config_update_signature:
        Option<unsafe extern "C" fn(*mut RimeConfig, *const c_char) -> Bool>,
    pub config_begin_map: Option<
        unsafe extern "C" fn(*mut RimeConfigIterator, *mut RimeConfig, *const c_char) -> Bool,
    >,
    pub config_next: Option<unsafe extern "C" fn(*mut RimeConfigIterator) -> Bool>,
    pub config_end: Option<unsafe extern "C" fn(*mut RimeConfigIterator)>,

    // testing
    pub simulate_key_sequence: Option<unsafe extern "C" fn(RimeSessionId, *const c_char) -> Bool>,
}

/// `RimeApi` 表入口（唯二的 C 导出符号之一）。
pub type RimeGetApiFn = unsafe extern "C" fn() -> *const RimeApi;

impl RimeApi {
    /// 校验 `rime_get_api()` 返回的表是否覆盖我们声明的最长前缀。
    ///
    /// `data_size` 是 `sizeof(RimeApi) - sizeof(data_size)`，因此表实际大小
    /// 为 `data_size + sizeof(int)`。若小于本结构大小，说明 librime 过旧。
    pub fn validate_layout(&self) -> Result<(), crate::error::FfiError> {
        let actual = self.data_size as i64 + std::mem::size_of::<c_int>() as i64;
        let needed = std::mem::size_of::<RimeApi>() as i64;
        if self.data_size <= 0 || actual < needed {
            return Err(crate::error::FfiError::UnsupportedApi {
                field: "simulate_key_sequence（RimeApi 表过短，librime 版本过旧）",
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_api(data_size: c_int) -> RimeApi {
        let mut api: RimeApi = unsafe { std::mem::zeroed() };
        api.data_size = data_size;
        api
    }

    #[test]
    fn layout_validation_accepts_current_size() {
        let api =
            fake_api((std::mem::size_of::<RimeApi>() - std::mem::size_of::<c_int>()) as c_int);
        assert!(api.validate_layout().is_ok());
    }

    #[test]
    fn layout_validation_rejects_too_short_table() {
        let api = fake_api(64);
        assert!(matches!(
            api.validate_layout(),
            Err(crate::error::FfiError::UnsupportedApi { .. })
        ));
        let api = fake_api(0);
        assert!(api.validate_layout().is_err());
        let api = fake_api(-1);
        assert!(api.validate_layout().is_err());
    }

    #[test]
    fn rime_traits_data_size_matches_c_macro() {
        let traits = RimeTraits::nulled();
        assert_eq!(
            traits.data_size as usize,
            std::mem::size_of::<RimeTraits>() - std::mem::size_of::<c_int>()
        );
        let ctx = RimeContext::nulled();
        assert_eq!(
            ctx.data_size as usize,
            std::mem::size_of::<RimeContext>() - std::mem::size_of::<c_int>()
        );
        let commit = RimeCommit::nulled();
        assert_eq!(
            commit.data_size as usize,
            std::mem::size_of::<RimeCommit>() - std::mem::size_of::<c_int>()
        );
    }
}
