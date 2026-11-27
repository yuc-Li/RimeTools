/**
 * 最近部署列表（`docs/ipc.md` §3.5 `deploy_history`，PRD FR-5.6）。
 *
 * 每条展示：触发源、完成时间（能推断时）、耗时、产物是否验证、失败原因。
 * 关键：`success && !outputVerified` 的历史条目同样标成「产物未验证」警告，
 * 不能因为命令成功就显示为绿色成功（FR-5.5）。
 */
import type { AppError, DeployResult, DeployState } from "../../ipc/types";
import { ErrorBlock } from "./ErrorBlock";
import {
  formatDateTime,
  formatDuration,
  inferHistoryFinishedAt,
  triggerLabel,
} from "./format";

interface Props {
  items: DeployResult[] | null;
  loading: boolean;
  error: AppError | null;
  state: DeployState;
  /** 本页会话内观察到的完成时刻（结果指纹 → Unix 毫秒） */
  observedTimes: Record<string, number>;
  onReload: () => void;
}

type HistoryTone = "ok" | "warn" | "danger";

function verdictOf(item: DeployResult): { tone: HistoryTone; label: string; reason?: string } {
  if (!item.success) {
    return {
      tone: "danger",
      label: "失败",
      reason: item.error?.message ?? "后端未返回失败原因，请查看日志",
    };
  }
  if (!item.outputVerified) {
    return {
      tone: "warn",
      label: "产物未验证",
      reason: "命令执行成功，但未检测到 build/ 产物变化，改动可能未生效",
    };
  }
  return { tone: "ok", label: "产物已验证" };
}

function SkeletonRows() {
  return (
    <div className="dp-skeleton-list" aria-hidden="true">
      <span className="dp-skeleton" />
      <span className="dp-skeleton" />
      <span className="dp-skeleton" />
    </div>
  );
}

export function DeployHistoryList({
  items,
  loading,
  error,
  state,
  observedTimes,
  onReload,
}: Props) {
  const hasUnknownTime =
    items?.some(
      (item, index) => inferHistoryFinishedAt(item, index, state, observedTimes) == null,
    ) ?? false;

  return (
    <section className="dp-card" aria-labelledby="dp-history-title" aria-busy={loading || undefined}>
      <div className="dp-card__head">
        <h2 id="dp-history-title">最近部署</h2>
        {items && items.length > 0 ? (
          <span className="dp-badge dp-badge--muted">{items.length} 条</span>
        ) : null}
        <button
          type="button"
          className="dp-btn dp-btn--small dp-card__action"
          onClick={onReload}
          disabled={loading}
        >
          {loading ? "刷新中…" : "刷新"}
        </button>
      </div>

      {error ? (
        <ErrorBlock error={error} title="加载部署历史失败" onRetry={onReload} retryLabel="重试" />
      ) : null}

      {!items && loading ? <SkeletonRows /> : null}

      {!items && !loading && !error ? (
        <p className="dp-empty">部署历史暂不可用，请稍后刷新。</p>
      ) : null}

      {items && items.length === 0 ? (
        <p className="dp-empty">暂无部署记录。点击右上角「立即部署」发起第一次部署。</p>
      ) : null}

      {items && items.length > 0 ? (
        <ol className="dp-history__list">
          {items.map((item, index) => {
            const verdict = verdictOf(item);
            const finishedAt = inferHistoryFinishedAt(item, index, state, observedTimes);
            return (
              <li
                key={`${index}-${item.trigger}-${item.success}-${item.durationMs}`}
                className={`dp-history__row dp-history__row--${verdict.tone}`}
              >
                <div className="dp-history__main">
                  <span className="dp-badge dp-badge--muted">{triggerLabel(item.trigger)}</span>
                  <span
                    className="dp-history__time"
                    title={
                      finishedAt == null
                        ? "deploy_history 的结果不含完成时刻，且没有日志时间戳可还原"
                        : undefined
                    }
                  >
                    {formatDateTime(finishedAt)}
                  </span>
                  <span className="dp-history__meta">耗时 {formatDuration(item.durationMs)}</span>
                  <span className={`dp-badge dp-badge--${verdict.tone} dp-history__verdict`}>
                    {verdict.label}
                  </span>
                  {item.logPath ? (
                    <span className="dp-history__log" title={item.logPath}>
                      日志
                    </span>
                  ) : null}
                </div>
                {verdict.reason ? (
                  <p
                    className={`dp-history__reason${
                      verdict.tone === "warn" ? " dp-history__reason--warn" : ""
                    }`}
                  >
                    {verdict.reason}
                  </p>
                ) : null}
              </li>
            );
          })}
        </ol>
      ) : null}

      {hasUnknownTime ? (
        <p className="dp-note dp-note--muted dp-history__footnote">
          历史接口只返回耗时、不含完成时刻：时间列优先从部署日志文件名
          （<code>deploy-&lt;unix_ms&gt;.log</code>）还原，最近一条可用当前状态的结束时间，其余显示
          「—」。
        </p>
      ) : null}
    </section>
  );
}