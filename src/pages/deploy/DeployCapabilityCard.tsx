/**
 * 部署能力提示（`docs/ipc.md` §3.1 `env_detect` → `FrontendInfo.deployCapability`）。
 *
 * 目标（PRD FR-5.4/5.8）：
 * - 自动部署可用时说明实际走的通道（D-Bus / rime_deployer / WeaselDeployer）；
 * - `supported === false` 时**明确告知需手动部署并给出可执行方法**，不能只把按钮变灰。
 */
import type { AppError, FrontendInfo } from "../../ipc/types";
import type { EnvDetectResult } from "./api";
import { ErrorBlock } from "./ErrorBlock";
import {
  frontendLabel,
  manualDeployGuide,
  methodLabel,
  type ManualDeployGuide,
} from "./format";

interface Props {
  env: EnvDetectResult | null;
  loading: boolean;
  error: AppError | null;
  onReload: () => void;
}

function ManualGuide({ guide }: { guide: ManualDeployGuide }) {
  return (
    <>
      <ol className="dp-guide">
        {guide.steps.map((step) => (
          <li key={step}>{step}</li>
        ))}
      </ol>
      {guide.command ? (
        <>
          <p className="dp-note dp-note--muted">rime_deployer 兜底命令（在终端执行）：</p>
          <pre className="dp-command">{guide.command}</pre>
        </>
      ) : null}
      <p className="dp-note dp-note--muted">
        手动部署完成后，可点「重新检测」确认环境，或直接观察输入法是否加载新词库。
      </p>
    </>
  );
}

export function DeployCapabilityCard({ env, loading, error, onReload }: Props) {
  const active: FrontendInfo | null = env?.active ?? null;
  const frontends = env?.frontends ?? [];
  // `active` 为空（多前端共存时后端不猜，FR-1.6）时，优先展示一个支持自动部署的前端。
  const frontend =
    active ??
    frontends.find((item) => item.deployCapability.supported) ??
    frontends[0] ??
    null;
  const capability = frontend?.deployCapability ?? null;
  const multiple = !active && frontends.length > 1;
  const unsupported = !!frontend && !frontend.deployCapability.supported;
  const guide = manualDeployGuide(frontend);
  const showEmpty = !loading && !error && !frontend;

  return (
    <section
      className={`dp-card dp-capability${unsupported ? " dp-capability--warn" : ""}`}
      aria-labelledby="dp-capability-title"
    >
      <div className="dp-card__head">
        <h2 id="dp-capability-title">部署能力</h2>
        {loading ? <span className="dp-badge dp-badge--muted">检测中…</span> : null}
        {!loading && error ? <span className="dp-badge dp-badge--warn">检测失败</span> : null}
        {capability ? (
          <span className={`dp-badge dp-badge--${capability.supported ? "ok" : "warn"}`}>
            {capability.supported ? "自动部署可用" : "需手动部署"}
          </span>
        ) : null}
        <button
          type="button"
          className="dp-btn dp-btn--small dp-card__action"
          onClick={onReload}
          disabled={loading}
        >
          重新检测
        </button>
      </div>

      {loading && !env ? (
        <p className="dp-note dp-note--muted">正在探测 Rime 前端与部署通道…</p>
      ) : null}

      {error ? (
        <ErrorBlock error={error} title="无法探测部署能力" onRetry={onReload} retryLabel="重新检测">
          <p className="dp-note dp-note--muted">
            仍可尝试「立即部署」；若失败，请先确认输入法正在运行。
          </p>
        </ErrorBlock>
      ) : null}

      {showEmpty ? (
        <>
          <p className="dp-note dp-note--warn">
            未检测到 Rime 前端（fcitx5-rime / 小狼毫 / ibus-rime）。
          </p>
          <p className="dp-note dp-note--muted">改动写入后需要手动让 Rime 重新加载：</p>
          <ManualGuide guide={guide} />
        </>
      ) : null}

      {!error && frontend && capability ? (
        <>
          {multiple ? (
            <p className="dp-note dp-note--warn">
              检测到 {frontends.length} 个 Rime 前端，未能唯一确定活动前端；以下按「
              {frontend.displayName}」展示（FR-1.6：不静默猜测）。可在设置页指定用户目录与前端。
            </p>
          ) : null}
          <dl className="dp-kv dp-kv--capability">
            <dt>前端</dt>
            <dd>
              {frontendLabel(frontend)}
              {!active ? <span className="dp-muted">（未确定活动前端）</span> : null}
            </dd>
            <dt>部署方式</dt>
            <dd>{methodLabel(capability.method)}</dd>
            <dt>Rime 用户目录</dt>
            <dd>
              <code>{frontend.userDir}</code>
            </dd>
            {frontend.sharedDir ? (
              <>
                <dt>共享数据目录</dt>
                <dd>
                  <code>{frontend.sharedDir}</code>
                </dd>
              </>
            ) : null}
            {frontend.librimeVersion ? (
              <>
                <dt>librime</dt>
                <dd>{frontend.librimeVersion}</dd>
              </>
            ) : null}
          </dl>
          {capability.note ? <p className="dp-note">{capability.note}</p> : null}
          {capability.supported ? (
            <p className="dp-note dp-note--muted">
              部署会请求输入法重新加载配置与 build/ 产物；期间输入可能短暂受影响（FR-5.8）。
            </p>
          ) : (
            <>
              <p className="dp-note dp-note--warn">
                当前环境没有可用的自动部署通道，已将「立即部署」禁用。请按以下方式手动部署：
              </p>
              <ManualGuide guide={guide} />
            </>
          )}
        </>
      ) : null}
    </section>
  );
}