/**
 * IPC 类型定义的 TypeScript 侧。
 * 与 `docs/ipc.md`（冻结契约 v1）及 `src-tauri/crates/rime-core/src/types.rs` 一一对应。
 *
 * 改动规则：先改 docs/ipc.md → 同步 Rust 与 TS。禁止单侧改动。
 */

// ───────────────────────── 错误 ─────────────────────────

export type ErrorCode =
  | "frontendNotFound" | "userDirInvalid" | "userDirNotWritable" | "schemeNotDetected"
  | "fileNotFound" | "fileNotWritable" | "fileFormatInvalid" | "externalModified" | "corruptEncoding"
  | "validationFailed" | "duplicateEntry" | "codeCharsetInvalid" | "codeTooLong"
  | "backupFailed" | "atomicWriteFailed" | "diskFull" | "permissionDenied"
  | "deployUnavailable" | "deployTimeout" | "deployFailed"
  | "notFound" | "internal";

export interface AppError {
  code: ErrorCode;
  message: string;
  detail?: string;
  hint?: string;
}

// ───────────────────────── 环境 ─────────────────────────

export type FrontendKind = "weasel" | "fcitx5Rime" | "ibusRime" | "unknown";

export interface DeployCapability {
  supported: boolean;
  /** "dbus-setconfig" | "rime-deployer" | "weasel-deployer" | "none" */
  method: string;
  note?: string;
}

export interface FrontendInfo {
  kind: FrontendKind;
  displayName: string;
  version?: string;
  librimeVersion?: string;
  userDir: string;
  sharedDir?: string;
  deployCapability: DeployCapability;
}

// ───────────────────────── 方案 ─────────────────────────

export type TranslatorKind = "script" | "table" | "mixed" | "unknown";
export type FileFormat = "customPhrase" | "rimeDictYaml" | "plainTxt" | "csv";
export type FileRole =
  | "customPhrase" | "userDict" | "userTopDict" | "personalDict" | "readOnlyDict";
export type ColumnKind = "text" | "code" | "weight";

export type FileId = string;

export interface ManagedFileRef {
  id: FileId;
  path: string;
  format: FileFormat;
  role: FileRole;
  columns: ColumnKind[];
}

export interface AlgebraSpec {
  rules: string[];
  alphabet: string;
}

export interface SchemeInfo {
  schemaId: string;
  name: string;
  version?: string;
  isActive: boolean;
  isCurrent: boolean;
  translatorKind: TranslatorKind;
  alphabet: string;
  maxCodeLength?: number;
  managedFiles: ManagedFileRef[];
  isDoublePinyin: boolean;
  algebra?: AlgebraSpec;
}

// ───────────────────────── 词条 ─────────────────────────

export interface EntryKey {
  fileId: FileId;
  text: string;
  code: string;
}

export type EntryOrigin = "file" | "userAdded" | "userEdited";

export interface Entry {
  key: EntryKey;
  /** null 表示该文件无权重列（如 xhup 用户码表） */
  weight: number | null;
  group: string | null;
  lineNo: number | null;
  origin: EntryOrigin;
}

// ───────────────────────── 查询 ─────────────────────────

export type SortField = "text" | "code" | "weight" | "lineNo";

export interface SortSpec {
  field: SortField;
  desc: boolean;
}

export interface EntryQuery {
  fileId?: FileId | null;
  keyword?: string | null;
  group?: string | null;
  sort: SortSpec;
  offset: number;
  limit: number;
}

export interface GroupCount {
  name: string;
  count: number;
}

export interface EntryPage {
  items: Entry[];
  total: number;
  groups: GroupCount[];
}

// ───────────────────────── 建议与预览 ─────────────────────────

export type SuggestionKind =
  | "fullPinyin" | "doublePinyin" | "abbrev" | "shapeWord" | "shapeChar" | "english" | "history";

export type Confidence = "high" | "medium" | "low";

export type SuggestionWarning =
  | "blocksLearning" | "exceedsMaxCodeLength" | "unsupportedCharset" | "polyphone";

export interface CodeSuggestion {
  code: string;
  kind: SuggestionKind;
  label: string;
  reading?: string;
  confidence: Confidence;
  warnings: SuggestionWarning[];
}

export type PreviewMode = "live" | "static" | "unavailable";

export interface Candidate {
  text: string;
  comment?: string;
  weight?: number;
  isTarget: boolean;
}

export interface CandidatePreview {
  mode: PreviewMode;
  items: Candidate[];
  note?: string;
}

// ───────────────────────── 部署 ─────────────────────────

export type DeployTrigger = "manual" | "afterSave" | "scheduled";
export type DeployPhase =
  | "idle" | "preparing" | "running" | "verifying" | "success" | "failed" | "cancelled";

export interface DeployResult {
  trigger: DeployTrigger;
  success: boolean;
  durationMs: number;
  /** 完成时刻（Unix 毫秒）；历史列表用它显示时间（契约 v1.1 新增） */
  finishedAt?: number | null;
  outputVerified: boolean;
  error?: AppError;
  logPath?: string;
}

export interface DeployState {
  phase: DeployPhase;
  startedAt: number | null;
  finishedAt: number | null;
  message?: string;
  lastResult?: DeployResult;
}

// ───────────────────────── 备份与历史 ─────────────────────────

export type BackupReason =
  | "beforeWrite" | "manual" | "scheduled" | "beforeRestore" | "beforeDeploy";

export interface BackupRef {
  id: string;
  path: string;
  sourcePath: string;
  createdAt: number;
  size: number;
  reason: BackupReason;
}

export interface DiffLine {
  text: string;
  code: string;
  weight?: number;
}

export interface DiffSummary {
  added: number;
  removed: number;
  modified: number;
  unchanged: number;
}

export interface FileDiff {
  sourcePath: string;
  added: DiffLine[];
  removed: DiffLine[];
  summary: DiffSummary;
}

// ───────────────────────── 写入 ─────────────────────────

export interface Fingerprint {
  mtimeMs: number;
  size: number;
  sha256: string;
}

export interface NewEntry {
  text: string;
  code: string;
  weight?: number | null;
  group?: string | null;
}

export interface EntryUpdate {
  key: EntryKey;
  newText?: string | null;
  newCode?: string | null;
  weight?: number | null;
  group?: string | null;
}

export interface SaveRequest {
  fileId: FileId;
  expectedFingerprint: Fingerprint;
  add: NewEntry[];
  update: EntryUpdate[];
  delete: EntryKey[];
  reorder?: EntryKey[] | null;
  deployAfter: boolean;
}

export interface SaveResult {
  written: number;
  backup?: BackupRef;
  fingerprint: Fingerprint;
  deploy?: DeployResult;
}

export interface ValidationIssue {
  code: ErrorCode;
  message: string;
  lineNo?: number;
  text?: string;
  entryCode?: string;
}

// ───────────────────────── 设置 ─────────────────────────

export type Theme = "system" | "light" | "dark";

export interface BackupSettings {
  enabled: boolean;
  dir?: string | null;
  keepCount: number;
  scheduledEnabled: boolean;
  scheduledIntervalHours: number;
}

export interface AppSettings {
  userDir?: string | null;
  frontend?: FrontendKind | null;
  managedFiles: string[];
  autoDeployAfterSave: boolean;
  backup: BackupSettings;
  theme: Theme;
  language: string;
}

// ───────────────────────── 事件 ─────────────────────────

export interface IndexProgressEvent {
  fileId: FileId;
  phase: string;
  done: number;
  total: number;
}

export interface ExternalChangeEvent {
  fileId: FileId;
  fingerprint: Fingerprint;
}

export const IPC_EVENTS = {
  deployState: "deploy://state",
  deployLog: "deploy://log",
  indexProgress: "dict://index-progress",
  externalChange: "dict://external-change",
  frontendChange: "env://frontend-change",
  backupCreated: "backup://created",
} as const;

// ───────────────────────── 导入导出（M2） ─────────────────────────

/** 导入/导出的文件格式（与后端 `TransferFormat` 对齐） */
export type TransferFormat = "txt" | "csv" | "dictYaml";

/** 冲突处理策略 */
export type ImportPolicy = "skipExisting" | "overwrite" | "keepHigherWeight";

/** 单条导入的处理结果 */
export type ImportAction = "added" | "updated" | "skipped" | "unchanged";

/** 解析阶段跳过的行（格式问题） */
export interface ImportIssue {
  lineNo: number;
  message: string;
  raw: string;
}

/** 与现有词库冲突的条目及处理方式 */
export interface ImportConflict {
  text: string;
  code: string;
  existingWeight: number | null;
  incomingWeight: number | null;
  action: ImportAction;
}

/** 导入预览：只做比对，不写文件 */
export interface ImportPlan {
  total: number;
  added: number;
  updated: number;
  skipped: number;
  unchanged: number;
  /** 导入内容自身的重复条数 */
  duplicates: number;
  /** 冲突明细（最多 200 条） */
  conflicts: ImportConflict[];
  /** 解析阶段的问题行 */
  issues: ImportIssue[];
  skippedLines: number;
  /** 编码不在字母表内或超长的条目（保存时会被拦下） */
  invalidCodes: ImportIssue[];
  /** 目标文件无权重列，导入的权重被忽略 */
  weightIgnored: boolean;
}

export interface ExportResult {
  /** 写了文件时为目标路径；只返回内容时为 null */
  path: string | null;
  suggestedName: string;
  format: TransferFormat;
  entryCount: number;
  bytes: number;
  /** 未指定路径时返回的内容（供复制） */
  content: string | null;
}

// ───────────────────────── 只读反查（M2） ─────────────────────────

/** 来源性质：用户目录（可编辑） / 系统预设（只读） */
export type SourceOrigin = "user" | "system";

/** 可反查的词库文件（不含条目内容） */
export interface LookupSourceSummary {
  id: string;
  name: string;
  origin: SourceOrigin;
  schemaId: string | null;
  format: string;
  entryCount: number;
  error: string | null;
}

/** 反查页可选的方案（输入法）*/
export interface LookupSchemeOption {
  schemaId: string;
  name: string;
  isActive: boolean;
  isDoublePinyin: boolean;
}

export interface LookupSourcesResult {
  /** 实际使用的方案 id；`*` 表示「全部文件」兜底 */
  schemaId: string;
  schemaName: string;
  sources: LookupSourceSummary[];
  /** 参与符号查询的来源说明 */
  symbolSources: string[];
  /** 解析失败的来源数量 */
  failed: number;
  userDir: string;
  /** 可切换的方案列表 */
  schemes: LookupSchemeOption[];
}

/** 反查模式：按词查编码 / 按编码查词 */
export type LookupMode = "text" | "code";

export interface LookupQuery {
  mode: LookupMode;
  keyword: string;
  /** true 只匹配完全相等，false 匹配前缀与包含 */
  exact: boolean;
  /** 限定来源（空数组 = 全部） */
  sourceIds: string[];
  limit: number;
}

/** 某个命中来自哪个文件 */
export interface HitSource {
  id: string;
  name: string;
  origin: SourceOrigin;
  schemaId: string | null;
  weight: number | null;
  lineNo: number;
}

export interface LookupHit {
  text: string;
  code: string;
  /** 0=完全匹配 1=前缀 2=包含 */
  rank: number;
  sources: HitSource[];
}

export interface LookupResult {
  hits: LookupHit[];
  /** 命中总数（可能大于 hits.length） */
  total: number;
  truncated: boolean;
  scannedSources: number;
}

export type SymbolShape = "half" | "full";

/** 符号查询结果：一个触发键及其能打出的符号 */
export interface SymbolHit {
  key: string;
  symbols: string[];
  scope: string;
  shape: SymbolShape;
  /** commit = 直接出字；list = 需 /键 选择 */
  mode: string;
  rank: number;
}

// ───────────────────────── 方案配置（M2 设置增强） ─────────────────────────

/** 方案 speller 段的可编辑项 */
export interface SpellerConfig {
  autoSelect: boolean | null;
  maxCodeLength: number | null;
  autoSelectPattern: string | null;
  autoClear: string | null;
  alphabet: string | null;
}

/** `import_tables` 里的一张词表 */
export interface MountEntry {
  name: string;
  enabled: boolean;
  comment: string | null;
  /** 解析到的实际文件；不存在为 null */
  path: string | null;
  entryCount: number | null;
  /** 用户码表：建议保持挂载 */
  isUserTable: boolean;
}

export interface SchemeConfigSnapshot {
  schemaId: string;
  schemaName: string;
  /** 方案文件路径（只读） */
  schemaPath: string;
  /** 覆盖文件路径（可能尚不存在） */
  customPath: string;
  customExists: boolean;
  speller: SpellerConfig;
  /** 生效值来源：schema | custom */
  spellerSource: string;
  /** 主词典路径（import_tables 所在文件） */
  dictPath: string | null;
  mounts: MountEntry[];
  deployHint: string;
}

export interface MountToggle {
  name: string;
  enabled: boolean;
}

export interface SchemeConfigUpdateResult {
  snapshot: SchemeConfigSnapshot;
  /** 实际写入的文件 */
  written: string[];
  deploy: DeployResult | null;
}
