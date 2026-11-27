//! RimeTools 核心领域类型。
//!
//! 本文件是 `docs/ipc.md`（冻结契约 v1）的 Rust 侧唯一映射。
//! **改动此文件必须同步 `docs/ipc.md`。**

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ───────────────────────── 错误模型 ─────────────────────────

/// 机器可判定的错误码（契约 §1.1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    // 环境
    FrontendNotFound,
    UserDirInvalid,
    UserDirNotWritable,
    SchemeNotDetected,
    // 文件
    FileNotFound,
    FileNotWritable,
    FileFormatInvalid,
    ExternalModified,
    CorruptEncoding,
    // 词条
    ValidationFailed,
    DuplicateEntry,
    CodeCharsetInvalid,
    CodeTooLong,
    // 写入
    BackupFailed,
    AtomicWriteFailed,
    DiskFull,
    PermissionDenied,
    // 部署
    DeployUnavailable,
    DeployTimeout,
    DeployFailed,
    // 通用
    NotFound,
    Internal,
}

/// 统一错误载体。`message` 面向用户，可直接展示。
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

pub type AppResult<T> = Result<T, AppError>;

// ───────────────────────── 环境 ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FrontendKind {
    Weasel,
    Fcitx5Rime,
    IbusRime,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployCapability {
    pub supported: bool,
    /// "dbus-setconfig" | "rime-deployer" | "weasel-deployer" | "none"
    pub method: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendInfo {
    pub kind: FrontendKind,
    pub display_name: String,
    pub version: Option<String>,
    pub librime_version: Option<String>,
    pub user_dir: PathBuf,
    pub shared_dir: Option<PathBuf>,
    pub deploy_capability: DeployCapability,
}

// ───────────────────────── 方案 ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TranslatorKind {
    /// script_translator：全拼/双拼
    Script,
    /// table_translator：音形/形码
    Table,
    Mixed,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlgebraSpec {
    /// 原样保存的 algebra 规则，供求值器逐条执行（顺序敏感）。
    pub rules: Vec<String>,
    pub alphabet: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileFormat {
    CustomPhrase,
    RimeDictYaml,
    PlainTxt,
    Csv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileRole {
    /// table_translator@custom_phrase 的 user_dict
    CustomPhrase,
    /// 音形用户码表 *.user.dict.yaml
    UserDict,
    /// 置顶码表 *.user.top.dict.yaml
    UserTopDict,
    /// 用户自建 dict.yaml（mydict）
    PersonalDict,
    /// 系统词典，只读
    ReadOnlyDict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColumnKind {
    Text,
    Code,
    Weight,
}

pub type FileId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedFileRef {
    pub id: FileId,
    pub path: PathBuf,
    pub format: FileFormat,
    pub role: FileRole,
    /// 决定写入列风格：三列（含权重）还是两列（xhup 用户码表）。
    pub columns: Vec<ColumnKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemeInfo {
    pub schema_id: String,
    pub name: String,
    pub version: Option<String>,
    pub is_active: bool,
    pub is_current: bool,
    pub translator_kind: TranslatorKind,
    /// 允许的编码字符集（来自 speller/alphabet）
    pub alphabet: String,
    pub max_code_length: Option<u32>,
    pub managed_files: Vec<ManagedFileRef>,
    pub is_double_pinyin: bool,
    pub algebra: Option<AlgebraSpec>,
}

// ───────────────────────── 词条 ─────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryKey {
    pub file_id: FileId,
    pub text: String,
    pub code: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryOrigin {
    File,
    UserAdded,
    UserEdited,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub key: EntryKey,
    /// `None` 表示该文件无权重列（如 xhup 用户码表）。
    pub weight: Option<u32>,
    /// 工具级元数据，不写入 Rime 文件。
    pub group: Option<String>,
    pub line_no: Option<u32>,
    pub origin: EntryOrigin,
}

// ───────────────────────── 查询 ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortField {
    Text,
    Code,
    Weight,
    LineNo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SortSpec {
    pub field: SortField,
    pub desc: bool,
}

impl Default for SortSpec {
    fn default() -> Self {
        Self {
            field: SortField::LineNo,
            desc: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryQuery {
    pub file_id: Option<FileId>,
    /// 同时匹配 text 与 code
    pub keyword: Option<String>,
    pub group: Option<String>,
    pub sort: SortSpec,
    pub offset: u32,
    pub limit: u32,
}

impl Default for EntryQuery {
    fn default() -> Self {
        Self {
            file_id: None,
            keyword: None,
            group: None,
            sort: SortSpec::default(),
            offset: 0,
            limit: 100,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupCount {
    pub name: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryPage {
    pub items: Vec<Entry>,
    pub total: u32,
    pub groups: Vec<GroupCount>,
}

// ───────────────────────── 建议与预览 ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SuggestionKind {
    FullPinyin,
    DoublePinyin,
    Abbrev,
    ShapeWord,
    ShapeChar,
    English,
    History,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SuggestionWarning {
    /// 完整码会阻断自学习造词（custom_phrase 场景，FR-3.22）
    BlocksLearning,
    ExceedsMaxCodeLength,
    UnsupportedCharset,
    /// 多音字，请确认读音
    Polyphone,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeSuggestion {
    pub code: String,
    pub kind: SuggestionKind,
    pub label: String,
    pub reading: Option<String>,
    pub confidence: Confidence,
    pub warnings: Vec<SuggestionWarning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreviewMode {
    /// librime 隔离会话实时预览
    Live,
    /// 按权重/导入顺序静态排序
    Static,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub text: String,
    pub comment: Option<String>,
    pub weight: Option<f64>,
    pub is_target: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePreview {
    pub mode: PreviewMode,
    pub items: Vec<Candidate>,
    pub note: Option<String>,
}

// ───────────────────────── 部署 ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeployTrigger {
    Manual,
    AfterSave,
    Scheduled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeployPhase {
    Idle,
    Preparing,
    Running,
    Verifying,
    Success,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployResult {
    pub trigger: DeployTrigger,
    pub success: bool,
    pub duration_ms: u64,
    /// 完成时刻（Unix 毫秒）。历史列表靠它显示时间，
    /// 否则只能从日志文件名反推（契约 v1.1 新增，向后兼容）。
    pub finished_at: Option<i64>,
    /// build 产物是否实际变化（不以退出码 alone 判定，FR-5.5）
    pub output_verified: bool,
    pub error: Option<AppError>,
    pub log_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployState {
    pub phase: DeployPhase,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub message: Option<String>,
    pub last_result: Option<DeployResult>,
}

impl Default for DeployState {
    fn default() -> Self {
        Self {
            phase: DeployPhase::Idle,
            started_at: None,
            finished_at: None,
            message: None,
            last_result: None,
        }
    }
}

// ───────────────────────── 备份与历史 ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackupReason {
    BeforeWrite,
    Manual,
    Scheduled,
    BeforeRestore,
    BeforeDeploy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupRef {
    pub id: String,
    pub path: PathBuf,
    pub source_path: PathBuf,
    pub created_at: i64,
    pub size: u64,
    pub reason: BackupReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub text: String,
    pub code: String,
    pub weight: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffSummary {
    pub added: u32,
    pub removed: u32,
    pub modified: u32,
    pub unchanged: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub source_path: PathBuf,
    pub added: Vec<DiffLine>,
    pub removed: Vec<DiffLine>,
    pub summary: DiffSummary,
}

// ───────────────────────── 写入 ─────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fingerprint {
    pub mtime_ms: i64,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewEntry {
    pub text: String,
    pub code: String,
    pub weight: Option<u32>,
    pub group: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryUpdate {
    pub key: EntryKey,
    pub new_text: Option<String>,
    pub new_code: Option<String>,
    pub weight: Option<u32>,
    pub group: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveRequest {
    pub file_id: FileId,
    /// 打开文件时获得的指纹；不一致即拒绝写入（FR 写入安全协议）
    pub expected_fingerprint: Fingerprint,
    pub add: Vec<NewEntry>,
    pub update: Vec<EntryUpdate>,
    pub delete: Vec<EntryKey>,
    pub reorder: Option<Vec<EntryKey>>,
    pub deploy_after: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveResult {
    pub written: u32,
    pub backup: Option<BackupRef>,
    pub fingerprint: Fingerprint,
    pub deploy: Option<DeployResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationIssue {
    pub code: ErrorCode,
    pub message: String,
    pub line_no: Option<u32>,
    pub text: Option<String>,
    pub entry_code: Option<String>,
}

// ───────────────────────── 设置 ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSettings {
    pub enabled: bool,
    pub dir: Option<PathBuf>,
    pub keep_count: u32,
    pub scheduled_enabled: bool,
    pub scheduled_interval_hours: u32,
}

impl Default for BackupSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            dir: None,
            keep_count: 30,
            scheduled_enabled: false,
            scheduled_interval_hours: 24,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub user_dir: Option<PathBuf>,
    pub frontend: Option<FrontendKind>,
    pub managed_files: Vec<PathBuf>,
    pub auto_deploy_after_save: bool,
    pub backup: BackupSettings,
    pub theme: Theme,
    pub language: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            user_dir: None,
            frontend: None,
            managed_files: Vec::new(),
            auto_deploy_after_save: true,
            backup: BackupSettings::default(),
            theme: Theme::System,
            language: "zh-CN".to_owned(),
        }
    }
}
