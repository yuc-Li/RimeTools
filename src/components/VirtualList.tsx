/**
 * 虚拟滚动列表：固定行高的窗口化渲染，不依赖第三方库。
 *
 * 设计：
 * - 只渲染可视区 + `overscan` 行，容器内用等高 spacer 撑开滚动条，所以 10 万行也不会有 10 万个 DOM 节点；
 * - 行高固定（调用方保证），可通过 `getRowKey` 之外的 index 定位，避免测量每行高度带来的抖动；
 * - `onRangeChange` 报告当前可视行区间（已含 overscan 的裁剪），供调用方按需分页取数；
 * - `resetKey` 变化时把滚动位置归零（切换文件/搜索/排序时需要）。
 */
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import "./components.css";

export interface VirtualListProps {
  /** 总行数（未知/未加载时可以小于真实条数，列表会显示占位行） */
  rowCount: number;
  rowHeight: number;
  renderRow: (index: number) => ReactNode;
  /** 可视区上下各多渲染几行，防止快速滚动时出现白屏 */
  overscan?: number;
  /** 可视行区间变化（start 含、end 不含） */
  onRangeChange?: (start: number, end: number) => void;
  /** 该值变化时滚回顶部 */
  resetKey?: unknown;
  className?: string;
  ariaLabel?: string;
}

export function VirtualList({
  rowCount,
  rowHeight,
  renderRow,
  overscan = 8,
  onRangeChange,
  resetKey,
  className,
  ariaLabel,
}: VirtualListProps) {
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const rafRef = useRef(0);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);

  // 测量可视高度：字体/窗口尺寸变化都可能导致高度变化，所以用 ResizeObserver
  useEffect(() => {
    const el = viewportRef.current;
    if (!el) return;
    const measure = () => setViewportHeight(el.clientHeight);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // 切换查询条件时回顶部
  useEffect(() => {
    const el = viewportRef.current;
    if (!el) return;
    el.scrollTop = 0;
    setScrollTop(0);
  }, [resetKey]);

  // 行数变少时夹住滚动位置，避免停在空白区
  useEffect(() => {
    const el = viewportRef.current;
    if (!el) return;
    const max = Math.max(0, rowCount * rowHeight - el.clientHeight);
    if (el.scrollTop > max) {
      el.scrollTop = max;
      setScrollTop(max);
    }
  }, [rowCount, rowHeight]);

  // 滚动回调用 rAF 合并，避免每次 scroll 事件都触发 React 渲染
  const handleScroll = () => {
    if (rafRef.current) return;
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = 0;
      const el = viewportRef.current;
      if (el) setScrollTop(el.scrollTop);
    });
  };

  useEffect(() => () => {
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
  }, []);

  const { start, end } = useMemo(() => {
    const first = Math.floor(scrollTop / rowHeight);
    const last = Math.ceil((scrollTop + viewportHeight) / rowHeight);
    return {
      start: Math.max(0, first - overscan),
      end: Math.min(rowCount, Math.max(first + 1, last + overscan)),
    };
  }, [scrollTop, viewportHeight, rowHeight, rowCount, overscan]);

  useEffect(() => {
    onRangeChange?.(start, end);
  }, [start, end, onRangeChange]);

  const rows: ReactNode[] = [];
  for (let index = start; index < end; index++) {
    rows.push(
      <div
        key={index}
        className="ui-vlist__row"
        style={{ top: index * rowHeight, height: rowHeight }}
        role="presentation"
      >
        {renderRow(index)}
      </div>,
    );
  }

  return (
    <div
      ref={viewportRef}
      className={`ui-vlist ${className ?? ""}`}
      onScroll={handleScroll}
      role="list"
      aria-label={ariaLabel}
      tabIndex={0}
    >
      <div className="ui-vlist__spacer" style={{ height: Math.max(0, rowCount) * rowHeight }}>
        {rows}
      </div>
    </div>
  );
}

export default VirtualList;