/**
 * 工作台的 IPC 门面：把 `docs/ipc.md` 的命令收敛成强类型函数。
 *
 * 数据来源切换：
 * - Tauri 宿主内 → `call()`（`src/ipc/client.ts`）；
 * - 纯浏览器（`npm run dev`）→ `src/ipc/mock.ts`，保证不装 Rust 也能开发调试。
 *
 * 参数命名注意：Tauri v2 默认把 JS 的 camelCase 参数映射到 Rust 的 snake_case 形参，
 * 所以这里传 `{ fileId }` 对应后端的 `file_id`；结构体参数名以后端签名为准：
 * - `dict_query(query: EntryQuery)` → `{ query }`
 * - `dict_validate(request: SaveRequest)` / `dict_save(request: SaveRequest)` → `{ request }`
 * 若命令层签名调整，只需改本文件。
 */

import { call, isTauri } from "../../ipc/client";
import { mockInvoke } from "../../ipc/mock";
import type {
  CodeSuggestion,
  EntryPage,
  EntryQuery,
  ExportResult,
  FileId,
  FileRole,
  Fingerprint,
  ImportPlan,
  ImportPolicy,
  ManagedFileRef,
  SaveRequest,
  SaveResult,
  SchemeInfo,
  TransferFormat,
  ValidationIssue,
} from "../../ipc/types";

export interface DictOpenResult {
  ref: ManagedFileRef;
  total: number;
  fingerprint: Fingerprint;
}

export interface DictValidateResult {
  ok: boolean;
  errors: ValidationIssue[];
}

/** 当前是否是纯前端 mock 模式（用于展示「模拟数据」标识与调试按钮） */
export function isMockMode(): boolean {
  return !isTauri();
}

async function backend<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    return mockInvoke<T>(cmd, args ?? {});
  }
  return call<T>(cmd, args);
}

export const ipc = {
  /**
   * `docs/ipc.md` §3.1 `env_list_schemes`
   * 后端签名 `env_list_schemes(user_dir: Option<String>)`；传 null 表示自动探测。
   */
  listSchemes: (): Promise<SchemeInfo[]> =>
    backend<SchemeInfo[]>("env_list_schemes", { userDir: null }),

  /** `docs/ipc.md` §3.2 `dict_open` —— 打开词库并取得指纹（保存时必传） */
  openDict: (fileId: FileId): Promise<DictOpenResult> =>
    backend<DictOpenResult>("dict_open", { fileId }),

  /** `docs/ipc.md` §3.2 `dict_query` —— 分页/搜索/排序/分组 */
  query: (query: EntryQuery): Promise<EntryPage> => backend<EntryPage>("dict_query", { query }),

  /** `docs/ipc.md` §3.2 `dict_validate` —— 保存前校验（逐条问题） */
  validate: (request: SaveRequest): Promise<DictValidateResult> =>
    backend<DictValidateResult>("dict_validate", { request }),

  /** `docs/ipc.md` §3.2 `dict_save` —— 原子写 + 写前备份 + 外部修改检测 */
  save: (request: SaveRequest): Promise<SaveResult> => backend<SaveResult>("dict_save", { request }),

  // ── 导入导出（M2） ──

  /** 导出当前词库；`path` 为空时只返回内容（供复制） */
  exportDict: (params: {
    fileId: FileId;
    format: TransferFormat;
    path?: string | null;
  }): Promise<ExportResult> =>
    backend<ExportResult>("dict_export", {
      request: { fileId: params.fileId, format: params.format, path: params.path ?? null },
    }),

  /** 导入预览：解析 + 冲突比对，不写文件 */
  importPreview: (params: {
    fileId: FileId;
    path?: string | null;
    content?: string | null;
    format?: TransferFormat | null;
    policy: ImportPolicy;
  }): Promise<ImportPlan> =>
    backend<ImportPlan>("dict_import_preview", {
      request: {
        fileId: params.fileId,
        path: params.path ?? null,
        content: params.content ?? null,
        format: params.format ?? null,
        policy: params.policy,
      },
    }),

  /** 应用导入（走标准保存流程：校验 + 写前备份 + 原子写） */
  importApply: (params: {
    fileId: FileId;
    path?: string | null;
    content?: string | null;
    format?: TransferFormat | null;
    policy: ImportPolicy;
    deployAfter?: boolean;
  }): Promise<SaveResult> =>
    backend<SaveResult>("dict_import_apply", {
      request: {
        fileId: params.fileId,
        path: params.path ?? null,
        content: params.content ?? null,
        format: params.format ?? null,
        policy: params.policy,
        deployAfter: params.deployAfter ?? false,
      },
    }),

  /** 选择导入文件（系统文件对话框；mock 模式返回 null） */
  pickImportFile: (): Promise<string | null> => backend<string | null>("pick_import_file", {}),

  /** 选择导出保存位置 */
  pickExportPath: (defaultName: string): Promise<string | null> =>
    backend<string | null>("pick_export_path", { defaultName }),

  /** `docs/ipc.md` §3.3 `suggest_codes` —— 编码建议（多候选、含警告） */
  suggestCodes: (params: {
    text: string;
    schemaId?: string;
    excludeExisting?: string[];
    knownCodes?: string[];
    role?: FileRole;
  }): Promise<CodeSuggestion[]> =>
    backend<CodeSuggestion[]>("suggest_codes", {
      text: params.text,
      schemaId: params.schemaId,
      excludeExisting: params.excludeExisting,
      knownCodes: params.knownCodes,
      role: params.role,
    }),
};

export default ipc;