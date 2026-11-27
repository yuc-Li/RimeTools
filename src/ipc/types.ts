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