/**
 * 当前部署状态卡片（PRD FR-5.1/5.5/5.6/5.8）。
 *
 * 数据来自 `src/ipc/events.ts` 的全局快照（`docs/ipc.md` §4 `deploy://state`），
 * 本组件不自己订阅、不发命令，只负责把 `DeployState` 呈现清楚：
 * - 阶段徽标 + 三阶段进度（准备 → 部署 → 校验产物）；
 * - 开始/结束时间、实时耗时；
 * - `success && !outputVerified` 显示成**警告**（FR-5.5，防假成功）；
 * - 失败时展示 `error.message/hint/detail` 与可操作建议。
 */
import type { ReactNode } from "react";
import { DEPLOY_PHASE_LABEL, describeDeployResult, isDeploying } from "../../ipc/events";
import type { DeployPhase, DeployState } from "../../ipc/types";
import { ErrorBlock } from "./ErrorBlock";
import {
  PHASE_TONE,
  classifyDeployResult,
  deployElapsedMs,
  formatDateTime,
  formatDuration,
  resultError,
  triggerLabel,
} from "./format";

const STEPS: Array<{ phase: DeployPhase; label: string }> = [
  { phase: "preparing", label: "准备" },
  { phase: "running", label: "部署" },
  { phase: "verifying", label: "校验产物" },
];

function stepClass(index: number, phase: DeployPhase): string {
  if (phase === "success") return "is-done";
  const activeIndex = STEPS.findIndex((step) => step.phase === phase);
  if (activeIndex === -1) return "";
  if (index < activeIndex) return "is-done";
  if (index === activeIndex) return "is-active";
  return "";
}

const FAILURE_ADVICE: ReactNode[] = [
  <>
    确认输入法正在运行：fcitx5 可执行 <code>pgrep -a fcitx5</code>；小狼毫检查 WeaselServer
    进程。未运行时先启动，再重试部署。
  </>,
  <>确认当前方案已在 schema_list 中启用，Rime 用户目录指向正确。</>,
  <>
    若错误提示「未检测到 build/ 产物变化」，重启 fcitx5 后再部署一次——命令退出码不等于部署生效
    （PRD FR-5.5）。
  </>,
  <>
    若当前环境不支持自动部署，请按页面上方「部署能力」里的手动部署方法操作；刚保存过词库时，
    可先到备份页恢复写前备份再重试（FR-5.7）。
  </>,
];

interface Props {
  state: DeployState;
  /** 由页面每秒刷新，用于部署中的实时耗时 */
  now: number;
  /** 已发出 `deploy_start` 但事件还没到达 */
  starting: boolean;
}

export function DeployStatusCard({ state, now, starting }: Props) {
  const deploying = isDeploying(state);
  /** 已发请求、事件尚未到达：此时不要沿用上一轮的起止时间与进度 */
  const submitting = starting && !deploying;
  const live = deploying || starting;
  const elapsed = deployElapsedMs(state, now);
  const result = state.lastResult;
  const verdict = classifyDeployResult(result);
  const showSteps = !submitting && state.phase !== "failed" && state.phase !== "cancelled";

  return (
    <section
      className="dp-card dp-status"
      aria-labelledby="dp-status-title"
      aria-busy={live || undefined}
    >
      <div className="dp-card__head">
        <h2 id="dp-status-title">当前状态</h2>
        <span className={`dp-badge dp-badge--${PHASE_TONE[state.phase]}`}>
          {DEPLOY_PHASE_LABEL[state.phase]}
        </span>
        {live ? <span className="dp-status__pulse" aria-hidden="true" /> : null}
      </div>

      {showSteps ? (
        <ol className="dp-steps" aria-label="部署阶段">
          {STEPS.map((step, index) => {
            const cls = stepClass(index, state.phase);
            return (
              <li
                key={step.phase}
                className={`dp-step ${cls}`.trim()}
                aria-current={cls === "is-active" ? "step" : undefined}
              >
                <span className="dp-step__dot" aria-hidden="true">
                  {cls === "is-done" ? "✓" : index + 1}
                </span>
                <span className="dp-step__label">{step.label}</span>
                {index < STEPS.length - 1 ? (
                  <span className="dp-step__line" aria-hidden="true" />
                ) : null}
              </li>
            );
          })}
        </ol>
      ) : null}

      <dl className="dp-kv">
        <dt>阶段</dt>
        <dd>
          {DEPLOY_PHASE_LABEL[state.phase]}
          {state.message && !live ? <span className="dp-muted"> · {state.message}</span> : null}
        </dd>
        <dt>开始时间</dt>
        <dd>{submitting ? "—" : formatDateTime(state.startedAt)}</dd>
        <dt>结束时间</dt>
        <dd>
          {submitting ? "正在提交…" : deploying ? "进行中…" : formatDateTime(state.finishedAt)}
        </dd>
        <dt>耗时</dt>
        <dd>
          {submitting
            ? "正在提交…"
            : deploying
              ? `进行中 · ${formatDuration(elapsed)}`
              : formatDuration(elapsed)}
        </dd>
        <dt>触发源</dt>
        <dd>{deploying || submitting ? "—" : result ? triggerLabel(result.trigger) : "—"}</dd>
      </dl>

      {live ? (
        <div className="dp-status__live" role="status" aria-live="polite">
          <span className="dp-spinner" aria-hidden="true" />
          <div>
            <p className="dp-status__live-title">
              {submitting ? "正在提交部署请求…" : `${DEPLOY_PHASE_LABEL[state.phase]}…`}
            </p>
            <p className="dp-status__live-desc">
              {submitting
                ? "等待后端接受请求；并发请求会被合并，不会排队多次。"
                : state.message ?? "部署进行中，阶段变化会自动刷新。"}
            </p>
            <p className="dp-status__live-desc">
              部署期间输入法可能短暂无响应或候选异常，通常数秒内恢复；无需关闭正在编辑的窗口。
            </p>
          </div>
        </div>
      ) : null}

      {/* 已取消：没有 DeployResult，只能读当前状态的 message */}
      {!live && state.phase === "cancelled" ? (
        <div className="dp-verdict dp-verdict--warn">
          <p className="dp-verdict__title">部署已取消</p>
          <p className="dp-verdict__detail">
            {state.message ?? "本轮部署被取消，改动尚未生效；如需生效请重新部署。"}
          </p>
        </div>
      ) : null}

      {/* 成功 / 警告：PRD FR-5.5 的关键区分 */}
      {!live && state.phase !== "cancelled" && result && result.success ? (
        <div className={`dp-verdict dp-verdict--${verdict.tone}`}>
          <p className="dp-verdict__title">
            {verdict.tone === "warn" ? "⚠ " : ""}
            {verdict.title}
          </p>
          <p className="dp-verdict__detail">
            {result.outputVerified ? describeDeployResult(result) : verdict.detail}
          </p>
          <p className="dp-verdict__meta">
            触发源：{triggerLabel(result.trigger)} · 耗时 {formatDuration(result.durationMs)}
          </p>
          {result.logPath ? (
            <p className="dp-verdict__log">
              日志：<code>{result.logPath}</code>
            </p>
          ) : null}
        </div>
      ) : null}

      {/* 失败：message / hint / detail（折叠）+ 可操作建议 */}
      {!live && state.phase !== "cancelled" && result && !result.success ? (
        <div className="dp-status__failure">
          <ErrorBlock error={resultError(result)} title="部署失败" />
          <ul className="dp-advice">
            {FAILURE_ADVICE.map((item, index) => (
              <li key={index}>{item}</li>
            ))}
          </ul>
          {result.logPath ? (
            <p className="dp-verdict__log">
              日志：<code>{result.logPath}</code>
            </p>
          ) : null}
        </div>
      ) : null}

      {!live && state.phase !== "cancelled" && !result ? (
        <p className="dp-verdict dp-verdict--muted">
          尚无部署结果。点击右上角「立即部署」把词库与配置改动推送到输入法。
        </p>
      ) : null}
    </section>
  );
}