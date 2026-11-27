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
  /** 列出可反查的来源（用户目录下的词库文件）与符号表来源 */
  sources: (): Promise<LookupSourcesResult> => backend<LookupSourcesResult>("lookup_sources"),

  /** 执行反查（按词查编码 / 按编码查词） */
  query: (request: LookupQuery): Promise<LookupResult> =>
    backend<LookupResult>("lookup_query", { request }),

  /** 重建索引（用户改了其他配置文件后） */
  rebuild: (): Promise<number> => backend<number>("lookup_rebuild"),

  /** 符号查询：可传符号本身（《）或触发键（<） */
  symbol: (keyword: string, limit = 50): Promise<SymbolHit[]> =>
    backend<SymbolHit[]>("symbol_query", { keyword, limit }),
};

/** 结果行的浮层提示：把来源列表压成一行 */
export function describeSources(hit: { sources: { name: string; lineNo: number }[] }): string {
  return hit.sources.map((s) => `${s.name}:${s.lineNo}`).join("、");
}