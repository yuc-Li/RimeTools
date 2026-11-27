/**
 * 词条表格：表头排序 + 虚拟滚动行渲染 + 草稿标记（PRD FR-3.6/3.7）。
 *
 * 行高固定为 `ENTRY_ROW_HEIGHT`，`VirtualList` 只渲染可视区附近的行；
 * 新增（暂存）行固定排在最前面，服务端行紧接其后，索引换算见 `handleRangeChange`。
 */
import { useCallback, type ReactNode } from "react";
import { Badge, EmptyState, ErrorAlert, Spinner } from "../../components/Feedback";
import { VirtualList } from "../../components/VirtualList";
import type { AppError, Entry, SortField, SortSpec } from "../../ipc/types";
import type { Draft, DraftAdd } from "./draft";
import { draftUpdateFor, isDraftDeleted } from "./draft";

export const ENTRY_ROW_HEIGHT = 34;

export interface EntryTableProps {
  adds: DraftAdd[];
  draft: Draft;
  /** 服务端过滤后总条数 */
  total: number;
  entryAt: (serverIndex: number) => Entry | undefined;
  hasWeightColumn: boolean;
  loading: boolean;
  error: AppError | null;
  sort: SortSpec;
  /** 是否有筛选条件（决定空状态里是否展示「清除筛选」） */
  hasFilters: boolean;
  onToggleSort: (field: SortField) => void;
  onRangeChange: (serverStart: number, serverEnd: number) => void;
  onEditEntry: (entry: Entry) => void;
  onEditAdd: (add: DraftAdd) => void;
  onDeleteEntry: (entry: Entry) => void;
  onRemoveAdd: (localId: string) => void;
  onClearFilters: () => void;
  onReload: () => void;
  resetToken: string;
  /** 只读文件（系统词典）：不允许编辑/删除 */
  readOnly?: boolean;
}

export function EntryTable({
  adds,
  draft,
  total,
  entryAt,
  hasWeightColumn,
  loading,
  error,
  sort,
  hasFilters,
  onToggleSort,
  onRangeChange,
  onEditEntry,
  onEditAdd,
  onDeleteEntry,
  onRemoveAdd,
  onClearFilters,
  onReload,
  resetToken,
  readOnly = false,
}: EntryTableProps) {
  const rowCount = adds.length + total;

  // 列表索引 → 服务端索引（新增行占用最前面的位置）
  const handleRangeChange = useCallback(
    (start: number, end: number) => {
      const serverStart = Math.max(0, start - adds.length);
      const serverEnd = Math.max(0, end - adds.length);
      if (serverEnd > serverStart) onRangeChange(serverStart, serverEnd);
    },
    [adds.length, onRangeChange],
  );

  const renderRow = useCallback(
    (index: number): ReactNode => {
      if (index < adds.length) {
        const add = adds[index];
        return (
          <div
            className="wb-grid wb-row wb-row--added"
            onDoubleClick={() => {
              if (!readOnly) onEditAdd(add);
            }}
            title={readOnly ? "只读文件" : "双击编辑这条新增词条"}
          >
            <span className="wb-cell wb-cell--text">
              <span className="wb-cell__value">{add.text}</span>
              <Badge tone="accent">新增</Badge>
            </span>
            <span className="wb-cell wb-cell--mono">{add.code}</span>
            <span className="wb-cell wb-cell--mono">
              {hasWeightColumn && add.weight != null ? add.weight : <span className="wb-muted">—</span>}
            </span>
            <span className="wb-cell">{add.group ?? <span className="wb-muted">未分组</span>}</span>
            <span className="wb-cell wb-cell--mono wb-muted">—</span>
            <span className="wb-cell wb-cell--actions">
              {readOnly ? (
                <span className="wb-muted">只读</span>
              ) : (
                <>
                  <button type="button" className="ui-btn ui-btn--sm" onClick={() => onEditAdd(add)}>
                    编辑
                  </button>
                  <button
                    type="button"
                    className="ui-btn ui-btn--sm"
                    onClick={() => onRemoveAdd(add.localId)}
                  >
                    移除
                  </button>
                </>
              )}
            </span>
          </div>
        );
      }

      const serverIndex = index - adds.length;
      const entry = entryAt(serverIndex);
      if (!entry) {
        // 该页还在路上：占位行，避免滚动时出现空白抖动
        return (
          <div className="wb-grid wb-row wb-row--skeleton" aria-hidden="true">
            <span className="wb-cell"><span className="wb-skeleton" style={{ width: "52%" }} /></span>
            <span className="wb-cell"><span className="wb-skeleton" style={{ width: "44%" }} /></span>
            <span className="wb-cell"><span className="wb-skeleton" style={{ width: "30%" }} /></span>
            <span className="wb-cell"><span className="wb-skeleton" style={{ width: "60%" }} /></span>
            <span className="wb-cell" />
            <span className="wb-cell" />
          </div>
        );
      }

      const update = draftUpdateFor(draft, entry.key);
      const deleted = isDraftDeleted(draft, entry.key);
      const text = update?.text ?? entry.key.text;
      const code = update?.code ?? entry.key.code;
      const weight = update ? update.weight : entry.weight;
      const group = update ? update.group : entry.group;

      const className = [
        "wb-grid",
        "wb-row",
        deleted ? "wb-row--deleted" : "",
        update ? "wb-row--updated" : "",
      ]
        .filter(Boolean)
        .join(" ");

      return (
        <div
          className={className}
          onDoubleClick={() => {
            if (!deleted && !readOnly) onEditEntry(entry);
          }}
          title={
            readOnly
              ? `第 ${entry.lineNo ?? "?"} 行 · 只读文件`
              : deleted
                ? "待删除（保存后移除）"
                : `第 ${entry.lineNo ?? "?"} 行 · 双击编辑`
          }
        >
          <span className="wb-cell wb-cell--text">
            <span className="wb-cell__value">{text}</span>
            {entry.origin === "userAdded" ? <Badge tone="accent">新增</Badge> : null}
            {entry.origin === "userEdited" && !update ? <Badge tone="muted">已编辑</Badge> : null}
            {update ? <Badge tone="warn">已修改</Badge> : null}
            {deleted ? <Badge tone="danger">待删除</Badge> : null}
          </span>
          <span className="wb-cell wb-cell--mono">{code}</span>
          <span className="wb-cell wb-cell--mono">
            {weight != null ? (
              weight
            ) : (
              <span
                className="wb-muted"
                title={hasWeightColumn ? "该词条未写权重列" : "该文件没有权重列（两列格式）"}
              >
                —
              </span>
            )}
          </span>
          <span className="wb-cell">{group ?? <span className="wb-muted">未分组</span>}</span>
          <span className="wb-cell wb-cell--mono wb-muted">{entry.lineNo ?? "—"}</span>
          <span className="wb-cell wb-cell--actions">
            {readOnly ? (
              <span className="wb-muted">只读</span>
            ) : deleted ? (
              <button type="button" className="ui-btn ui-btn--sm" onClick={() => onDeleteEntry(entry)}>
                撤销删除
              </button>
            ) : (
              <>
                <button type="button" className="ui-btn ui-btn--sm" onClick={() => onEditEntry(entry)}>
                  编辑
                </button>
                <button
                  type="button"
                  className="ui-btn ui-btn--sm ui-btn--danger"
                  onClick={() => onDeleteEntry(entry)}
                >
                  删除
                </button>
              </>
            )}
          </span>
        </div>
      );
    },
    [adds, draft, entryAt, hasWeightColumn, onDeleteEntry, onEditAdd, onEditEntry, onRemoveAdd, readOnly],
  );

  const sortIndicator = (field: SortField) =>
    sort.field !== field ? "" : sort.desc ? " ▼" : " ▲";
  const ariaSort = (field: SortField): "ascending" | "descending" | "none" =>
    sort.field !== field ? "none" : sort.desc ? "descending" : "ascending";

  const header = (
    <div className="wb-grid wb-row wb-row--head" role="row">
      <button
        type="button"
        className="wb-cell wb-cell--sort"
        role="columnheader"
        aria-sort={ariaSort("text")}
        onClick={() => onToggleSort("text")}
      >
        文本{sortIndicator("text")}
      </button>
      <button
        type="button"
        className="wb-cell wb-cell--sort"
        role="columnheader"
        aria-sort={ariaSort("code")}
        onClick={() => onToggleSort("code")}
      >
        编码{sortIndicator("code")}
      </button>
      <button
        type="button"
        className="wb-cell wb-cell--sort"
        role="columnheader"
        aria-sort={ariaSort("weight")}
        onClick={() => onToggleSort("weight")}
        title={hasWeightColumn ? "按权重排序" : "该文件没有权重列，权重均为空"}
      >
        权重{sortIndicator("weight")}
      </button>
      <span className="wb-cell" role="columnheader">分组</span>
      <button
        type="button"
        className="wb-cell wb-cell--sort"
        role="columnheader"
        aria-sort={ariaSort("lineNo")}
        onClick={() => onToggleSort("lineNo")}
        title="按源文件行号（默认顺序）"
      >
        行号{sortIndicator("lineNo")}
      </button>
      <span className="wb-cell" role="columnheader">操作</span>
    </div>
  );

  let body: ReactNode;
  if (rowCount === 0) {
    if (loading) {
      body = (
        <div className="wb-table__center">
          <Spinner label="正在读取词条…" />
        </div>
      );
    } else if (error) {
      body = (
        <div className="wb-table__center">
          <ErrorAlert error={error} onRetry={onReload} />
        </div>
      );
    } else if (hasFilters) {
      body = (
        <EmptyState
          title="没有匹配的词条"
          description="试试换个关键词，或清除筛选条件。"
          action={
            <button type="button" className="ui-btn" onClick={onClearFilters}>
              清除筛选
            </button>
          }
        />
      );
    } else {
      body = (
        <EmptyState
          title="这个文件还没有词条"
          description="点「新增词条」添加第一条，或者检查受管文件是否选对了。"
        />
      );
    }
  } else {
    body = (
      <VirtualList
        rowCount={rowCount}
        rowHeight={ENTRY_ROW_HEIGHT}
        renderRow={renderRow}
        onRangeChange={handleRangeChange}
        resetKey={resetToken}
        overscan={10}
        ariaLabel="词条列表"
      />
    );
  }

  return (
    <div className="wb-table">
      {header}
      <div className="wb-table__viewport">
        {body}
        {error && rowCount > 0 ? (
          <div className="wb-table__error">
            <ErrorAlert error={error} compact onRetry={onReload} />
          </div>
        ) : null}
      </div>
    </div>
  );
}

export default EntryTable;