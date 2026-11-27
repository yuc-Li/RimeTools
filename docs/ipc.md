# RimeTools IPC 契约（冻结版 v1）

| 项 | 内容 |
| --- | --- |
| 版本 | **v1（冻结）** |
| 冻结日期 | 2026-11-26 |
| 作用 | Web UI ↔ Rust 核心的唯一接口约定。**任何变更须走 §9 流程**，禁止私改 |
| 序列化 | JSON（Serde）；字段一律 `camelCase`（`#[serde(rename_all = "camelCase")]`） |
| 类型约定 | Rust `u64` → JS `number`（词条量级安全）；时间一律 **Unix 毫秒** `i64`；路径一律**绝对路径字符串**（UTF-8，非 UTF-8 路径拒绝并报错） |

---

## 1. 通用约定

### 1.1 错误模型

所有命令返回 `Result<T, AppError>`：

```rust
struct AppError {
    code: ErrorCode,        // 机器可判定
    message: String,        // 面向用户的中文说明（可直接展示）
    detail: Option<String>, // 技术详情，折叠展示
    hint: Option<String>,   // 下一步建议
}

enum ErrorCode {
    // 环境
    FrontendNotFound, UserDirInvalid, UserDirNotWritable, SchemeNotDetected,
    // 文件
    FileNotFound, FileNotWritable, FileFormatInvalid, ExternalModified, CorruptEncoding,
    // 词条
    ValidationFailed, DuplicateEntry, CodeCharsetInvalid, CodeTooLong,
    // 写入
    BackupFailed, AtomicWriteFailed, DiskFull, PermissionDenied,
    // 部署
    DeployUnavailable, DeployTimeout, DeployFailed,
    // 通用
    NotFound, Internal,
}
```

### 1.2 命令命名

`snake_case` 命令名，按域前缀分组：`env_*`、`dict_*`、`suggest_*`、`config_*`、`deploy_*`、`backup_*`、`settings_*`。

### 1.3 事件

后端 → 前端单向推送，前端用 `listen()` 订阅。事件名 `domain://action`。

---

## 2. 领域类型

```rust
// ---------- 环境 ----------
enum FrontendKind { Weasel, Fcitx5Rime, IbusRime, Unknown }

struct FrontendInfo {
    kind: FrontendKind,
    display_name: String,        // "小狼毫" / "fcitx5-rime"
    version: Option<String>,     // "5.1.13"
    librime_version: Option<String>, // "1.16.1"
    user_dir: PathBuf,
    shared_dir: Option<PathBuf>,
    deploy_capability: DeployCapability,
}

struct DeployCapability {
    supported: bool,
    method: String,              // "dbus-setconfig" | "rime-deployer" | "weasel-deployer" | "none"
    note: Option<String>,        // 用户可读说明
}

// ---------- 方案 ----------
struct SchemeInfo {
    schema_id: String,           // "xhup"
    name: String,                // "小鹤音形"
    version: Option<String>,
    is_active: bool,             // 是否在 schema_list 中
    is_current: bool,            // 当前正在使用
    translator_kind: TranslatorKind,
    // 拼写约束
    alphabet: String,            // "abc...';"
    max_code_length: Option<u32>,
    // 个人词入口
    managed_files: Vec<ManagedFileRef>,
    // 双拼
    is_double_pinyin: bool,
    algebra: Option<AlgebraSpec>,
}

enum TranslatorKind {
    Script,      // script_translator：全拼/双拼
    Table,       // table_translator：音形/形码
    Mixed,
    Unknown,
}

struct AlgebraSpec {
    rules: Vec<String>,          // 原样保存，供求值器
    alphabet: String,
}

// ---------- 受管文件 ----------
struct ManagedFileRef {
    id: FileId,                  // 稳定标识，用于前端引用
    path: PathBuf,
    format: FileFormat,
    role: FileRole,
    columns: Vec<ColumnKind>,    // 决定写入列风格
}

enum FileFormat { CustomPhrase, RimeDictYaml, PlainTxt, Csv }

enum FileRole {
    CustomPhrase,    // table_translator@custom_phrase 的 user_dict
    UserDict,        // 音形用户码表（*.user.dict.yaml）
    UserTopDict,     // 置顶码表（*.user.top.dict.yaml）
    PersonalDict,    // 用户自建 dict.yaml（mydict）
    ReadOnlyDict,    // 系统词典，只读
}

enum ColumnKind { Text, Code, Weight }

// ---------- 词条 ----------
type FileId = String;

struct EntryKey {
    file_id: FileId,
    text: String,
    code: String,
}

struct Entry {
    key: EntryKey,
    weight: Option<u32>,         // None = 文件无权重列
    group: Option<String>,       // 工具元数据
    line_no: Option<u32>,        // 源文件行号（诊断用）
    origin: EntryOrigin,
}

enum EntryOrigin { File, UserAdded, UserEdited }

// ---------- 查询 ----------
struct EntryQuery {
    file_id: Option<FileId>,     // None = 全部受管文件
    keyword: Option<String>,     // 匹配 text 或 code
    group: Option<String>,
    sort: SortSpec,
    offset: u32,
    limit: u32,
}

struct SortSpec { field: SortField, desc: bool }
enum SortField { Text, Code, Weight, LineNo }

struct EntryPage {
    items: Vec<Entry>,
    total: u32,                  // 过滤后总数
    groups: Vec<GroupCount>,     // 分组计数（侧栏用）
}

struct GroupCount { name: String, count: u32 }

// ---------- 建议 ----------
struct CodeSuggestion {
    code: String,
    kind: SuggestionKind,
    label: String,               // "全拼" / "双拼" / "简拼" / "音形词码" / "英文" / "历史"
    reading: Option<String>,     // "xíng" 便于多音字区分
    confidence: Confidence,
    warnings: Vec<SuggestionWarning>,
}

enum SuggestionKind { FullPinyin, DoublePinyin, Abbrev, ShapeWord, ShapeChar, English, History }

enum Confidence { High, Medium, Low }

enum SuggestionWarning {
    BlocksLearning,   // 完整码会阻断自学习造词（custom_phrase 场景）
    ExceedsMaxCodeLength,
    UnsupportedCharset,
    Polyphone,        // 多音字，请确认读音
}

// ---------- 候选预览 ----------
struct CandidatePreview {
    mode: PreviewMode,
    items: Vec<Candidate>,
    note: Option<String>,
}

enum PreviewMode { Live, Static, Unavailable }   // Live=librime 会话；Static=权重排序

struct Candidate { text: String, comment: Option<String>, weight: Option<f64>, is_target: bool }

// ---------- 部署 ----------
enum DeployTrigger { Manual, AfterSave, Scheduled }

struct DeployState {
    phase: DeployPhase,
    started_at: Option<i64>,
    finished_at: Option<i64>,
    message: Option<String>,
    last_result: Option<DeployResult>,
}

enum DeployPhase { Idle, Preparing, Running, Verifying, Success, Failed, Cancelled }

struct DeployResult {
    trigger: DeployTrigger,
    success: bool,
    duration_ms: u64,
    finished_at: Option<i64>,    // v1.1 新增：完成时刻（Unix 毫秒）
    output_verified: bool,       // build 产物是否变化
    error: Option<AppError>,
    log_path: Option<PathBuf>,
}

// ---------- 备份 / 历史 ----------
struct BackupRef {
    id: String,
    path: PathBuf,
    source_path: PathBuf,
    created_at: i64,
    size: u64,
    reason: BackupReason,
}

enum BackupReason { BeforeWrite, Manual, Scheduled, BeforeRestore, BeforeDeploy }

struct FileDiff {
    source_path: PathBuf,
    added: Vec<DiffLine>,
    removed: Vec<DiffLine>,
    summary: DiffSummary,
}

struct DiffLine { text: String, code: String, weight: Option<u32> }
struct DiffSummary { added: u32, removed: u32, modified: u32, unchanged: u32 }

// ---------- 保存 ----------
struct SaveRequest {
    file_id: FileId,
    expected_fingerprint: Fingerprint,   // 外部修改检测
    add: Vec<NewEntry>,
    update: Vec<EntryUpdate>,
    delete: Vec<EntryKey>,
    reorder: Option<Vec<EntryKey>>,
    deploy_after: bool,
}

struct Fingerprint { mtime_ms: i64, size: u64, sha256: String }

struct NewEntry { text: String, code: String, weight: Option<u32>, group: Option<String> }

struct EntryUpdate { key: EntryKey, new_text: Option<String>, new_code: Option<String>, weight: Option<u32>, group: Option<String> }

struct SaveResult {
    written: u32,
    backup: Option<BackupRef>,
    fingerprint: Fingerprint,    // 写入后新指纹，前端须刷新
    deploy: Option<DeployResult>,
}

// ---------- 设置 ----------
struct AppSettings {
    user_dir: Option<PathBuf>,
    frontend: Option<FrontendKind>,
    managed_files: Vec<PathBuf>,
    auto_deploy_after_save: bool,
    backup: BackupSettings,
    theme: Theme,
    language: String,
}

struct BackupSettings {
    enabled: bool,
    dir: Option<PathBuf>,
    keep_count: u32,
    scheduled_enabled: bool,
    scheduled_interval_hours: u32,
}

enum Theme { System, Light, Dark }
```

---

## 3. 命令清单

### 3.1 环境 `env_*`（FR-1）

| 命令 | 入参 | 出参 | 说明 |
| --- | --- | --- | --- |
| `env_detect` | — | `{ frontends: Vec<FrontendInfo>, active: Option<FrontendInfo> }` | 自动探测；多前端候选不猜 |
| `env_validate_dir` | `{ path }` | `{ valid: bool, reason: Option<String>, user_dir_writable: bool }` | 手动指定目录校验 |
| `env_set_user_dir` | `{ path }` | `Vec<SchemeInfo>` | 校验目录并持久化；返回该目录下识别出的方案（前端拿不到方案就没法继续） |
| `env_list_schemes` | — | `Vec<SchemeInfo>` | 含 translator 类型、alphabet、受管文件映射 |
| `env_scan_managed_files` | `{ schema_id? }` | `Vec<ManagedFileRef>` | 依方案识别个人词入口 |

### 3.2 词库 `dict_*`（FR-3）

| 命令 | 入参 | 出参 | 说明 |
| --- | --- | --- | --- |
| `dict_open` | `{ file_id }` | `{ ref: ManagedFileRef, total: u32, fingerprint: Fingerprint }` | 打开并建索引（后台） |
| `dict_query` | `EntryQuery` | `EntryPage` | 分页/搜索/排序/分组 |
| `dict_groups` | `{ file_id? }` | `Vec<GroupCount>` | 分组计数 |
| `dict_validate` | `SaveRequest` | `{ ok: bool, errors: Vec<ValidationIssue> }` | 保存前校验 |
| `dict_save` | `SaveRequest` | `SaveResult` | 原子写 + 写前备份 + 可选部署 |
| `dict_import_preview` | `{ path, format, target_file_id }` | `{ sample: Vec<Entry>, total: u32, conflicts: Vec<Conflict> }` | 导入预览 |
| `dict_import` | `{ path, format, target_file_id, conflict_policy }` | `SaveResult` | 导入 |
| `dict_export` | `{ query: EntryQuery, path, format, include_group }` | `{ exported: u32 }` | 导出 |
| `dict_set_group` | `{ keys: Vec<EntryKey>, group: Option<String> }` | `{ updated: u32 }` | 批量分组（仅元数据） |
| `dict_pin` | `{ key, pinned: bool }` | `SaveResult` | 音形置顶/取消（.user ↔ .user.top 移动，FR-3.21） |

### 3.3 建议与预览 `suggest_*`（FR-3.23/3.24）

| 命令 | 入参 | 出参 | 说明 |
| --- | --- | --- | --- |
| `suggest_codes` | `{ text, schema_id, exclude_existing?: Vec<String> }` | `Vec<CodeSuggestion>` | 编码建议（多候选、含警告） |
| `suggest_preview` | `{ code, schema_id, text? }` | `CandidatePreview` | 候选预览；Live 不可用自动降级 Static |

### 3.4 配置 `config_*`（FR-4，M2）

| 命令 | 入参 | 出参 | 说明 |
| --- | --- | --- | --- |
| `config_schema_list` | — | `{ active: Vec<String>, available: Vec<SchemeInfo> }` | 方案启停读取 |
| `config_set_schema_list` | `{ schema_ids: Vec<String> }` | `{ backup: BackupRef, diff: Vec<String> }` | 定点 patch 写入，保留注释 |
| `config_get_switches` | `{ schema_id }` | `Vec<SwitchState>` | 常用开关当前生效值与来源 |
| `config_set_switch` | `{ schema_id, key, value }` | `{ backup: BackupRef, applied: bool }` | 写 patch |

### 3.5 部署 `deploy_*`（FR-5）

| 命令 | 入参 | 出参 | 说明 |
| --- | --- | --- | --- |
| `deploy_start` | `{ trigger }` | `{ accepted: bool, merged: bool }` | 并发请求合并，至多一次补跑 |
| `deploy_status` | — | `DeployState` | 当前状态 + 最近结果 |
| `deploy_history` | `{ limit }` | `Vec<DeployResult>` | 历史 |
| `deploy_open_log` | `{ path }` | `{ ok: bool }` | 打开日志 |

### 3.6 备份与历史 `backup_*`（FR-6）

| 命令 | 入参 | 出参 | 说明 |
| --- | --- | --- | --- |
| `backup_list` | `{ file_id? }` | `Vec<BackupRef>` | 备份列表 |
| `backup_now` | `{ file_ids?: Vec<FileId> }` | `Vec<BackupRef>` | 立即备份 |
| `backup_restore` | `{ backup_id, dry_run: bool }` | `{ diff: FileDiff, result: Option<SaveResult> }` | 恢复前差异 → 恢复 |
| `backup_delete` | `{ backup_ids }` | `{ deleted: u32 }` | 删除 |
| `history_list` | `{ file_id, limit }` | `Vec<HistoryRef>` | 修改历史 |
| `history_diff` | `{ history_id }` | `FileDiff` | 历史 diff |
| `history_rollback` | `{ history_id, dry_run }` | `{ diff: FileDiff, result: Option<SaveResult> }` | 回滚（回滚前自动保护） |

### 3.7 设置 `settings_*`（FR-8）

| 命令 | 入参 | 出参 |
| --- | --- | --- |
| `settings_get` | — | `AppSettings` |
| `settings_update` | `Partial<AppSettings>` | `AppSettings` |
| `settings_reset` | `{ keep_dict_data: bool }` | `AppSettings` |
| `settings_open_dir` | `{ which }` | `{ ok: bool }` |

---

### 3.8 导入导出 `dict_export` / `dict_import_*`（M2）

| 命令 | 入参 | 出参 |
| --- | --- | --- |
| `dict_export` | `ExportRequest{ fileId, format, path? }` | `ExportResult` |
| `dict_import_preview` | `ImportPreviewRequest{ fileId, path?, content?, format?, policy }` | `ImportPlan` |
| `dict_import_apply` | `ImportApplyRequest{ …, policy, deploy_after }` | `SaveResult` |
| `pick_import_file` | — | `string \| null` |
| `pick_export_path` | `{ defaultName }` | `string \| null` |

`TransferFormat = "txt" | "csv" | "dictYaml"`；`ImportPolicy = "skipExisting" | "overwrite" | "keepHigherWeight"`。

约定：

- 导入**两段式**：`preview` 只解析并比对，返回 `ImportPlan`（新增/更新/跳过/未变化/自身重复/冲突明细/非法编码），
  用户确认后再 `apply`；
- `apply` 会**重新计算**计划（基于最新文件与指纹），并复用 `dict_save` 的写入流程，
  因此同样有校验、写前备份、原子写与外部修改保护；
- `path` 与 `content` 二选一：桌面端用路径（系统对话框），浏览器端用 `content`（`<input type=file>`）；
- 两列文件（无权重列）导入时权重被忽略，`ImportPlan.weight_ignored` 为 true。

### 3.9 只读反查 `lookup_*` / `symbol_query`（M2）

| 命令 | 入参 | 出参 |
| --- | --- | --- |
| `lookup_sources` | — | `LookupSourcesResult` |
| `lookup_query` | `LookupQuery{ mode, keyword, exact, sourceIds, limit }` | `LookupResult` |
| `lookup_rebuild` | — | `number`（重建的索引文件数） |
| `symbol_query` | `{ keyword, limit? }` | `SymbolHit[]` |

约定：

- 只读：不写任何文件，也不触发部署；
- 索引范围是**用户目录**下的 `*.dict.yaml` 与 `*.txt`（排除 `build/`、`sync/`、`*.bak`），
  系统预设不纳入，避免几十 MB 常驻内存；
- 相同（文本, 编码）在多文件中出现时合并为一条，`sources` 列出全部来源；
- `rank`：0 完全匹配 / 1 前缀 / 2 包含；
- 符号表来源是系统 `symbols.yaml` + 用户 `symbols.yaml` + 各方案 `punctuator`，同名表后者覆盖前者。

---

## 4. 事件

| 事件 | payload | 触发时机 |
| --- | --- | --- |
| `deploy://state` | `DeployState` | 部署阶段变化 |
| `deploy://log` | `{ line: String }` | 部署输出流 |
| `dict://index-progress` | `{ file_id, phase, done, total }` | 索引后台构建 |
| `dict://external-change` | `{ file_id, fingerprint }` | 检测到外部修改（**前端须提示，禁止静默覆盖**） |
| `env://frontend-change` | `FrontendInfo` | 前端/目录变化 |
| `backup://created` | `BackupRef` | 自动备份完成 |

---

## 5. 关键交互时序

### 5.1 保存（含外部修改检测）

```
UI                    Rust 核心
 │ dict_validate ────────>│ 校验列/字符集/码长/重复
 │<──── ok / errors ──────│
 │ dict_save ────────────>│ 1) 读文件 + sha256，与 expected_fingerprint 比对
 │                        │    不一致 → Err(ExternalModified)，不写
 │                        │ 2) 写前备份
 │                        │ 3) 临时文件 → fsync → 原子 rename
 │                        │ 4) 更新索引/历史
 │                        │ 5) deploy_after ? 触发部署
 │<── SaveResult ─────────│
```

### 5.2 部署并发合并

```
deploy_start(Manual)  ──┐
deploy_start(AfterSave)─┼──> 若 Running：置 pending_flag=true，返回 {accepted:false, merged:true}
deploy_start(Manual)  ──┘    当前轮结束后补跑一次（不排队 N 次）

状态流：Idle → Preparing → Running → Verifying → Success|Failed
事件：每阶段 emit deploy://state
```

---

## 6. 前端状态约束

1. **草稿模型**：所有编辑先在 UI 内存中标记 dirty，`dict_save` 一次性提交（FR-3.10）。
2. **指纹必传**：`dict_save` 必须携带打开时获得的 `expected_fingerprint`；收到 `ExternalModified` 时弹差异确认，**不得自动重试覆盖**。
3. **不做乐观更新**：写成功后以后端返回的 `fingerprint` 与重查结果为准。
4. **事件订阅**：`deploy://state`、`dict://external-change` 必须全局订阅（不随页面卸载）。

---

## 7. 与需求的映射

| 契约部分 | 对应需求 |
| --- | --- |
| `env_*` | FR-1.1–1.8 |
| `dict_*` | FR-3.1–3.13、3.17、3.21 |
| `suggest_*` | FR-3.23（原 FR-2.6）、FR-3.24（原 FR-2.8） |
| `config_*` | FR-4.1–4.5（M2） |
| `deploy_*` | FR-5.1–5.8 |
| `backup_*` / `history_*` | FR-6.1–6.7 |
| `settings_*` | FR-8 |
| `Entry.group` / `dict_set_group` | FR-3.4 |
| `ManagedFileRef.role` / `columns` | FR-3.1、FR-3.5 |
| `SuggestionWarning.BlocksLearning` | FR-3.22 |
| `dict_pin` | FR-3.21 |

**已暂缓能力**（不在 v1 契约内）：划词采集相关命令（`capture_*`）、跨方案导入（`dict_import_across_schema`）。若后续恢复，按 §9 以 v2 增量加入。

---

## 8. 实现落点

| 契约域 | Rust crate | 前端页面 |
| --- | --- | --- |
| `env_*` | `rime-platform` | `pages/settings`（环境区） |
| `dict_*`（读写/校验/索引） | `rime-core` | `pages/workbench`、`pages/io` |
| `suggest_*` | `rime-suggest`（+ `rime-ffi` for Live） | `pages/workbench` |
| `backup_*` / `history_*` / `dict_save` 写入 | `rime-safety` | `pages/backup` |
| `deploy_*` | `rime-deploy` | `pages/deploy` |
| `settings_*` / 托盘 / 窗口 | `src-tauri/src`（shell） | `pages/settings` |

**Tauri command 注册统一在 `src-tauri/src/commands/mod.rs`**，按域分文件，禁止在业务模块里直接 `#[tauri::command]`。

---

## 9. 变更流程

1. 提出变更 → 修改本文档并标注 `v1 → v1.x`（兼容）或 `v2`（破坏性）。
2. 涉及破坏性变更须 **lead 审查 + 受影响任务同步通知**。
3. 后端先加、前端后用；删除字段须先确认无引用的两个发布周期。
4. 契约变更与实现同 PR，禁止"先实现后补契约"。

---

## 10. 变更记录

| 版本 | 日期 | 变更 |
| --- | --- | --- |
| v1 | 2026-11-26 | 冻结首版：环境/词库/建议/配置/部署/备份/设置 七域；划词采集与跨方案导入不在内（已暂缓） |
| v1.1 | 2026-11-26 | 兼容性增量：`DeployResult.finished_at`（部署历史需要显示完成时刻，否则只能从日志文件名反推）；`deploy_open_log` 后端已实现 |
| v1.2 | 2026-11-27 | M2 新增两域：§3.8 导入导出（`dict_export` / `dict_import_preview` / `dict_import_apply` / 两个文件对话框）、§3.9 只读反查（`lookup_sources` / `lookup_query` / `lookup_rebuild` / `symbol_query`）。均为新增，不改动 v1 已有命令 |