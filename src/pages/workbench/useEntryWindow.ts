/**
 * 词条窗口数据源：按可视区间向 `dict_query` 分页取数（`docs/ipc.md` §3.2）。
 *
 * 为什么不一次性把 10 万条拉到前端：
 * - IPC 传输 + JSON 解析成本高；
 * - 列表只需要可视区附近的行，按页取数即可支撑 10 万行以上的滚动（配合 `VirtualList`）。
 *
 * 缓存策略：按页缓存 `Entry[]`，查询条件（文件/关键词/分组/排序）或 `revision` 变化时整体失效。
 * 不做乐观更新：保存成功由页面把 `revision` +1，这里重新从后端取数（§6.3）。
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { toAppError } from "../../ipc/client";
import type { AppError, Entry, GroupCount, SortSpec } from "../../ipc/types";
import { ipc } from "./api";

/** 每页行数：太小会频繁往返，太大单次传输偏重 */
export const PAGE_SIZE = 200;

export interface EntryWindowOptions {
  fileId: string | null;
  keyword: string;
  group: string | null;
  sort: SortSpec;
  /** 外部触发刷新（保存成功、重新加载后 +1） */
  revision: number;
}

export interface EntryWindow {
  /** 过滤后总条数（由后端返回） */
  total: number;
  groups: GroupCount[];
  loading: boolean;
  error: AppError | null;
  /** 取服务端行（相对过滤结果的下标）；未加载到的页返回 undefined，由列表渲染占位行 */
  entryAt: (serverIndex: number) => Entry | undefined;
  /** VirtualList 的可视区间变化回调 */
  ensureRange: (start: number, end: number) => void;
  /** 清空缓存并重新取数（重新加载按钮用） */
  reload: () => void;
}

interface WindowState {
  total: number;
  groups: GroupCount[];
  pages: Map<number, Entry[]>;
  loading: boolean;
  error: AppError | null;
}

const INITIAL_STATE: WindowState = {
  total: 0,
  groups: [],
  pages: new Map(),
  loading: false,
  error: null,
};

export function useEntryWindow(options: EntryWindowOptions): EntryWindow {
  const { fileId, keyword, group, sort, revision } = options;

  const [state, setState] = useState<WindowState>(INITIAL_STATE);
  const [attempt, setAttempt] = useState(0);

  const genRef = useRef(0);
  const pagesRef = useRef<Map<number, Entry[]>>(new Map());
  const pendingRef = useRef<Set<string>>(new Set());
  const queryRef = useRef({ fileId, keyword, group, sort });
  queryRef.current = { fileId, keyword, group, sort };

  const loadRange = useCallback((start: number, end: number) => {
    const query = queryRef.current;
    if (!query.fileId) return;

    const gen = genRef.current;
    const first = Math.max(0, Math.floor(start / PAGE_SIZE));
    const last = Math.max(first, Math.floor(Math.max(start, end - 1) / PAGE_SIZE));
    let added = false;

    for (let page = first; page <= last; page++) {
      const token = `${gen}:${page}`;
      if (pagesRef.current.has(page) || pendingRef.current.has(token)) continue;
      pendingRef.current.add(token);
      added = true;
      setState((prev) => (prev.loading ? prev : { ...prev, loading: true }));

      ipc
        .query({
          fileId: query.fileId,
          keyword: query.keyword || null,
          group: query.group,
          sort: query.sort,
          offset: page * PAGE_SIZE,
          limit: PAGE_SIZE,
        })
        .then((pageData) => {
          if (genRef.current !== gen) return;
          pagesRef.current.set(page, pageData.items);
          setState((prev) => ({
            ...prev,
            pages: new Map(pagesRef.current),
            total: pageData.total,
            groups: pageData.groups,
            error: null,
          }));
        })
        .catch((error: unknown) => {
          if (genRef.current !== gen) return;
          setState((prev) => ({ ...prev, error: toAppError(error) }));
        })
        .finally(() => {
          pendingRef.current.delete(token);
          if (genRef.current !== gen) return;
          if (pendingRef.current.size === 0) {
            setState((prev) => (prev.loading ? { ...prev, loading: false } : prev));
          }
        });
    }

    if (!added && pendingRef.current.size === 0) {
      setState((prev) => (prev.loading ? { ...prev, loading: false } : prev));
    }
  }, []);

  // 查询条件 / 文件 / revision 变化：清空缓存并取首页（拿到 total 与分组计数）
  useEffect(() => {
    genRef.current += 1;
    pendingRef.current.clear();
    pagesRef.current = new Map();
    setState({ ...INITIAL_STATE, pages: new Map(), loading: Boolean(fileId) });
    if (fileId) loadRange(0, PAGE_SIZE);
    // sort 只依赖 field/desc 两个原始值，避免对象字面量每次新建导致死循环
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fileId, keyword, group, sort.field, sort.desc, revision, attempt, loadRange]);

  const ensureRange = useCallback((start: number, end: number) => loadRange(start, end), [loadRange]);

  const entryAt = useCallback(
    (serverIndex: number): Entry | undefined => {
      const page = state.pages.get(Math.floor(serverIndex / PAGE_SIZE));
      return page?.[serverIndex % PAGE_SIZE];
    },
    [state.pages],
  );

  const reload = useCallback(() => setAttempt((n) => n + 1), []);

  return {
    total: state.total,
    groups: state.groups,
    loading: state.loading,
    error: state.error,
    entryAt,
    ensureRange,
    reload,
  };
}