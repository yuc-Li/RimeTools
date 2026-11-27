//! rime-ffi 的错误类型。
//!
//! 上层（`suggest_preview` 命令）拿到这些错误后降级为静态预览：
//! [`crate::unavailable_preview`] 生成 `PreviewMode::Unavailable` 的空预览并附带 `note`。

use thiserror::Error;

/// librime 动态加载 / 隔离会话 / 候选读取过程中的错误。
#[derive(Debug, Error)]
pub enum FfiError {
    /// 找不到 librime 动态库（环境变量与默认库名都失败）。
    #[error("未找到 librime 动态库，已尝试：{tried}")]
    LibraryNotFound { tried: String },

    /// 动态库存在但 dlopen 失败（架构不匹配、依赖缺失等）。
    #[error("加载 librime 失败（{path}）：{source}")]
    LibraryLoad {
        path: String,
        #[source]
        source: libloading::Error,
    },

    /// 动态库存在但没有 `rime_get_api` 符号。
    #[error("librime 缺少符号 {symbol}：{source}")]
    SymbolMissing {
        symbol: String,
        #[source]
        source: libloading::Error,
    },

    /// `RimeApi` 表缺少所需字段（librime 版本过旧）。
    #[error("librime 版本不兼容：缺少 API 字段 {field}")]
    UnsupportedApi { field: &'static str },

    /// `RimeInitialize` 之后仍无法创建会话。
    #[error("librime 初始化失败：{reason}")]
    Initialize { reason: String },

    /// 隔离目录准备失败（复制方案文件、写 patch 等）。
    #[error("隔离预览目录准备失败：{reason}")]
    Home { reason: String },

    /// 隔离目录里的部署（`RimeStartMaintenance`）失败。
    #[error("隔离预览部署失败：{reason}")]
    Deploy { reason: String },

    /// `RimeCreateSession` 返回 0。
    #[error("创建 librime 会话失败")]
    SessionCreate,

    /// 会话无法切换到目标方案。
    #[error("隔离会话无法加载方案 {schema_id}：{reason}")]
    SchemaSelect { schema_id: String, reason: String },

    /// 模拟按键失败。
    #[error("模拟按键 {key:?} 失败：{reason}")]
    SimulateKey { key: String, reason: String },

    /// 读取 context / commit 失败。
    #[error("读取候选失败：{reason}")]
    Context { reason: String },

    /// 编码为空或包含 NUL。
    #[error("编码无效：{reason}")]
    InvalidCode { reason: String },

    /// 同一进程里已有预览引擎持有 librime（librime 的 setup/initialize 是进程级全局状态）。
    #[error("另一个 librime 预览引擎正在使用中；同一进程只能初始化一次 librime")]
    EngineBusy,

    #[error("I/O 错误：{0}")]
    Io(#[from] std::io::Error),
}

impl FfiError {
    /// 供 UI 展示的一句话降级说明。
    pub fn fallback_note(&self) -> String {
        format!("实时候选预览不可用（{self}），已降级为静态排序预览")
    }
}
