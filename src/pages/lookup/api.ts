/**
 * 反查页的 IPC 门面（M2）。
 *
 * 数据来源与工作台一致：
 * - Tauri 宿主内 → `call()`
 * - 纯浏览器 → `mockInvoke()`（`src/ipc/mock.ts`）
 *
 * 后端命令见 `docs/ipc.md` §3.8 `lookup_*` / `symbol_query`。
 */

import { call, isTauri } from "../../ipc/client";
import { mockInvoke } from "../../ipc/mock";
import type {
  LookupQuery,
  LookupResult,
  LookupSourcesResult,
  SymbolHit,
} from "../../ipc/types";

async function backend<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    return mockInvoke<T>(cmd, args ?? {});
  }
  return call<T>(cmd, args);
}

export const lookupApi = {
  /**
   * 确保后端已注册方案列表。
   *
   * 方案是在 `env_list_schemes` 里注册进 `AppState` 的，而直接深链到反查页
   * （`?tab=lookup`）时工作台并未挂载，若不先调一次，方案下拉会是空的。
   */
  ensureSchemes: async (): Promise<void> => {
    try {
      await backend("env_list_schemes", { userDir: null });
    } catch {
      // 环境未就绪时忽略：反查自身会退回「全部文件」
    }
  },

  /**
   * 列出指定方案的词表与符号表来源。
   * `schemaId` 为空时用后端当前激活方案；传 "*" 表示扫描全部文件。
   */
  sources: (schemaId?: string | null): Promise<LookupSourcesResult> =>
    backend<LookupSourcesResult>("lookup_sources", { schemaId: schemaId ?? null }),

  /** 执行反查（在指定方案的词表范围内） */
  query: (request: LookupQuery, schemaId?: string | null): Promise<LookupResult> =>
    backend<LookupResult>("lookup_query", {
      request,
      schemaId: schemaId ?? null,
    }),

  /** 重建索引（用户改了词表文件后） */
  rebuild: (schemaId?: string | null): Promise<number> =>
    backend<number>("lookup_rebuild", { schemaId: schemaId ?? null }),

  /** 符号查询：可传符号本身（《）或触发键（<） */
  symbol: (keyword: string, limit = 50, schemaId?: string | null): Promise<SymbolHit[]> =>
    backend<SymbolHit[]>("symbol_query", { keyword, limit, schemaId: schemaId ?? null }),
};

/** 结果行的浮层提示：把来源列表压成一行 */
export function describeSources(hit: { sources: { name: string; lineNo: number }[] }): string {
  return hit.sources.map((s) => `${s.name}:${s.lineNo}`).join("、");
}