/**
 * 通用反馈组件：加载中 / 错误 / 空状态 / 徽标。
 * 错误一律接收规整后的 `AppError`（由 `toAppError()` 转换），UI 层不再做类型判断。
 */
import type { ReactNode } from "react";
import type { AppError } from "../ipc/types";
import "./components.css";

export function Spinner({ label }: { label?: string }) {
  return (
    <span className="ui-spinner" role="status" aria-live="polite">
      <span className="ui-spinner__dot" aria-hidden="true" />
      {label ? <span>{label}</span> : null}
    </span>
  );
}

export interface ErrorAlertProps {
  error: AppError;
  /** 重试动作（可选） */
  onRetry?: () => void;
  retryLabel?: string;
  /** 额外的操作区（如「重新加载」） */
  extra?: ReactNode;
  compact?: boolean;
}

export function ErrorAlert({ error, onRetry, retryLabel = "重试", extra, compact }: ErrorAlertProps) {
  return (
    <div className={`ui-alert${compact ? " ui-alert--compact" : ""}`} role="alert">
      <div className="ui-alert__head">
        <strong>{error.message}</strong>
        <code className="ui-alert__code">{error.code}</code>
      </div>
      {error.hint ? <p className="ui-alert__hint">💡 {error.hint}</p> : null}
      {error.detail ? (
        <details className="ui-alert__detail">
          <summary>技术详情</summary>
          <pre>{error.detail}</pre>
        </details>
      ) : null}
      {onRetry || extra ? (
        <div className="ui-alert__actions">
          {onRetry ? (
            <button type="button" className="ui-btn" onClick={onRetry}>
              {retryLabel}
            </button>
          ) : null}
          {extra}
        </div>
      ) : null}
    </div>
  );
}

export interface EmptyStateProps {
  title: string;
  description?: string;
  action?: ReactNode;
}

export function EmptyState({ title, description, action }: EmptyStateProps) {
  return (
    <div className="ui-empty">
      <p className="ui-empty__title">{title}</p>
      {description ? <p className="ui-empty__desc">{description}</p> : null}
      {action ? <div className="ui-empty__action">{action}</div> : null}
    </div>
  );
}

export type BadgeTone = "info" | "accent" | "warn" | "danger" | "ok" | "muted";

export function Badge({
  tone = "muted",
  children,
  title,
}: {
  tone?: BadgeTone;
  children: ReactNode;
  title?: string;
}) {
  return (
    <span className={`ui-badge ui-badge--${tone}`} title={title}>
      {children}
    </span>
  );
}