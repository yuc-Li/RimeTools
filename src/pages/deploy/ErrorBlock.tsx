/**
 * 错误块：统一渲染后端规整后的 `AppError`（`docs/ipc.md` §1.1）。
 *
 * - `message` 直接展示；
 * - `hint` 作为下一步建议；
 * - `detail` 折叠在 `<details>` 里（技术详情不干扰普通用户）。
 */
import type { ReactNode } from "react";
import type { AppError } from "../../ipc/types";

interface Props {
  error: AppError;
  /** 覆盖标题；默认直接用 `error.message` */
  title?: string;
  onRetry?: () => void;
  retryLabel?: string;
  /** 额外说明（自动部署建议、失败排查清单等） */
  children?: ReactNode;
}

export function ErrorBlock({ error, title, onRetry, retryLabel = "重试", children }: Props) {
  return (
    <div className="dp-error" role="alert">
      <div className="dp-error__head">
        <strong>{title ?? error.message}</strong>
        <code className="dp-error__code">{error.code}</code>
      </div>
      {title ? <p className="dp-error__message">{error.message}</p> : null}
      {error.hint ? <p className="dp-error__hint">💡 {error.hint}</p> : null}
      {error.detail ? (
        <details className="dp-error__detail">
          <summary>技术详情</summary>
          <pre>{error.detail}</pre>
        </details>
      ) : null}
      {children}
      {onRetry ? (
        <div className="dp-error__actions">
          <button type="button" className="dp-btn dp-btn--small" onClick={onRetry}>
            {retryLabel}
          </button>
        </div>
      ) : null}
    </div>
  );
}