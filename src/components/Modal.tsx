/**
 * 通用模态框。
 * - `Escape` 关闭（等价于调用方的取消语义）；
 * - 打开时聚焦第一个可交互元素；点击遮罩可关闭，但数据安全类确认（如外部修改）用
 *   `dismissOnBackdrop={false}` 强制用户显式选择。
 */
import { useEffect, useRef, type ReactNode } from "react";
import "./components.css";

export interface ModalProps {
  open: boolean;
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  /** 最大宽度（px），默认 560 */
  width?: number;
  /** 是否允许点击遮罩关闭，默认 true */
  dismissOnBackdrop?: boolean;
  /** 标题旁的严重程度标记 */
  tone?: "default" | "danger";
}

export function Modal({
  open,
  title,
  onClose,
  children,
  footer,
  width = 560,
  dismissOnBackdrop = true,
  tone = "default",
}: ModalProps) {
  const dialogRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        onClose();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  useEffect(() => {
    if (!open) return;
    const focusable = dialogRef.current?.querySelector<HTMLElement>(
      "input:not([disabled]), textarea:not([disabled]), select:not([disabled]), button:not([disabled])",
    );
    focusable?.focus();
  }, [open]);

  if (!open) return null;

  return (
    <div
      className="ui-modal__backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && dismissOnBackdrop) onClose();
      }}
    >
      <div
        ref={dialogRef}
        className={`ui-modal${tone === "danger" ? " ui-modal--danger" : ""}`}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        style={{ maxWidth: width }}
      >
        <header className="ui-modal__header">
          <h2>{title}</h2>
          <button type="button" className="ui-modal__close" onClick={onClose} aria-label="关闭">
            ×
          </button>
        </header>
        <div className="ui-modal__body">{children}</div>
        {footer ? <footer className="ui-modal__footer">{footer}</footer> : null}
      </div>
    </div>
  );
}

export default Modal;