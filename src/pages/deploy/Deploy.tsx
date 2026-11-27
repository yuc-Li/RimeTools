/**
 * 部署页（M1 / PRD FR-5.1–5.8；契约 `docs/ipc.md` §3.5、§4）。
 *
 * 数据来源：
 * - `useDeployState()`（`src/ipc/events.ts`）：全局单例订阅 `deploy://state`，页面切换不丢；
 * - `deploy_start({ trigger: "manual" })`：手动部署；返回 `{accepted:false, merged:true}` 表示
 *   并发请求被合并（不是失败，§5.2）；
 * - `deploy_history({ limit: 10 })`：最近结果；终态变化后自动静默刷新；
 * - `env_detect`：取 `deployCapability`，不支持自动部署时给手动部署指引。
 *
 * 产品红线：
 * - `success === true && outputVerified === false` 必须显示为**警告**（命令成功但 build/ 没变），
 *   不能一律绿色成功（FR-5.5，防止假成功误导用户）；
 * - 部署期间禁用按钮、展示阶段进度，但不阻塞 UI（FR-5.6）。
 *
 * App.tsx 接线：本组件是默认导出、无必填 props、自带页面标题与滚动容器，
 * 直接放进外壳的内容区（`.app__body`）即可。
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { toAppError } from "../../ipc/client";
import { DEPLOY_PHASE_LABEL, isDeploying, useDeployState } from "../../ipc/events";
import type { AppError, DeployResult } from "../../ipc/types";
import { DeployCapabilityCard } from "./DeployCapabilityCard";
import { DeployHistoryList } from "./DeployHistoryList";
import { DeployStatusCard } from "./DeployStatusCard";
import { ErrorBlock } from "./ErrorBlock";
import { deployApi, type EnvDetectResult } from "./api";
import { isDeployTerminal, resultKey } from "./format";
import "./deploy.css";

const HISTORY_LIMIT = 10;

interface Notice {
  tone: "info" | "success";
  title: string;
  text: string;
}

export default function Deploy() {
  // 全局部署状态（不随页面卸载中断）
  const deployState = useDeployState();
  const { phase, finishedAt } = deployState;
  const deploying = isDeploying(deployState);

  const [starting, setStarting] = useState(false);
  const [startError, setStartError] = useState<AppError | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);

  const [env, setEnv] = useState<EnvDetectResult | null>(null);
  const [envLoading, setEnvLoading] = useState(true);
  const [envError, setEnvError] = useState<AppError | null>(null);

  const [history, setHistory] = useState<DeployResult[] | null>(null);
  const [historyLoading, setHistoryLoading] = useState(true);
  const [historyError, setHistoryError] = useState<AppError | null>(null);
  // `DeployResult` 契约没有完成时刻，会话内观察到的终态时间按结果指纹回填给历史列表
  const [observedTimes, setObservedTimes] = useState<Record<string, number>>({});

  // 部署中的实时耗时（每秒 tick，非部署态不跑定时器）
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (!deploying) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [deploying]);

  /** `docs/ipc.md` §3.1 `env_detect` */
  const reloadEnv = useCallback(async () => {
    setEnvLoading(true);
    setEnvError(null);
    try {
      setEnv(await deployApi.envDetect());
    } catch (e) {
      setEnv(null);
      setEnvError(toAppError(e));
    } finally {
      setEnvLoading(false);
    }
  }, []);

  /**
   * `docs/ipc.md` §3.5 `deploy_history`
   * @param silent 终态自动刷新时不显示骨架屏，避免列表闪烁
   */
  const reloadHistory = useCallback(async (silent = false) => {
    if (!silent) setHistoryLoading(true);
    setHistoryError(null);
    try {
      setHistory(await deployApi.history(HISTORY_LIMIT));
    } catch (e) {
      setHistoryError(toAppError(e));
    } finally {
      if (!silent) setHistoryLoading(false);
    }
  }, []);

  useEffect(() => {
    void reloadEnv();
    void reloadHistory();
  }, [reloadEnv, reloadHistory]);

  // 每轮部署进入终态后刷新历史（`deploy://state` 只带最近结果，历史需单独查询）
  useEffect(() => {
    if (!isDeployTerminal(phase)) return;
    void reloadHistory(true);
  }, [phase, finishedAt, reloadHistory]);

  // 记录本轮完成时刻：历史接口不含时间戳，用于给列表回填（见 format.inferHistoryFinishedAt）
  const lastResult = deployState.lastResult;
  useEffect(() => {
    if (!isDeployTerminal(phase) || finishedAt == null || !lastResult) return;
    const key = resultKey(lastResult);
    setObservedTimes((prev) =>
      prev[key] === finishedAt ? prev : { ...prev, [key]: finishedAt },
    );
  }, [phase, finishedAt, lastResult]);

  /** `docs/ipc.md` §3.5 `deploy_start` */
  const handleDeploy = useCallback(async () => {
    if (starting || deploying) return;
    setStarting(true);
    setStartError(null);
    setNotice(null);
    try {
      const result = await deployApi.start("manual");
      if (result.merged) {
        // §5.2 并发合并：当前已有部署在跑，本轮结束后会自动补跑一次。不是失败！
        setNotice({
          tone: "info",
          title: "请求已合并（不是失败）",
          text: "已有部署在进行，本次请求已合并，结束后会自动补跑一次。",
        });
      } else if (result.accepted) {
        setNotice({
          tone: "info",
          title: "已发起部署",
          text: "正在触发 Rime 重新部署，阶段与结果会自动刷新。",
        });
      } else {
        setNotice({
          tone: "info",
          title: "请求未执行",
          text: "后端未接受本次部署请求（既未启动也未合并），请稍后重试。",
        });
      }
    } catch (e) {
      setStartError(toAppError(e));
    } finally {
      setStarting(false);
    }
  }, [deploying, starting]);

  // 能力已知不可用时禁用自动部署，避免用户反复触发一个必失败的命令
  const capabilityUnsupported = useMemo(() => {
    if (!env) return false;
    if (env.active) return !env.active.deployCapability.supported;
    if (env.frontends.length === 0) return true;
    return env.frontends.every((frontend) => !frontend.deployCapability.supported);
  }, [env]);

  const deployDisabled = starting || deploying || capabilityUnsupported;
  const deployLabel = starting
    ? "正在发起…"
    : deploying
      ? `部署中 · ${DEPLOY_PHASE_LABEL[phase]}`
      : "立即部署";
  const deployTitle = deploying
    ? "部署进行中，请等待本轮结束"
    : starting
      ? "正在提交部署请求"
      : capabilityUnsupported
        ? "当前环境不支持自动部署，请按「部署能力」中的手动方法操作"
        : "触发 Rime 重新部署";

  return (
    <div className="deploy">
      <header className="dp-header">
        <div className="dp-header__text">
          <h1>部署</h1>
          <p className="dp-header__sub">
            把词库与配置改动推送到输入法，并以 build/ 产物变化确认改动真的生效（不以命令退出码判定成功）。
          </p>
        </div>
        <div className="dp-header__actions">
          <button
            type="button"
            className="dp-btn dp-btn--primary"
            onClick={() => void handleDeploy()}
            disabled={deployDisabled}
            title={deployTitle}
          >
            {deployLabel}
          </button>
        </div>
      </header>

      {notice ? (
        <div className={`dp-banner dp-banner--${notice.tone}`} role="status">
          <div>
            <strong>{notice.title}</strong>
            <p>{notice.text}</p>
          </div>
          <button
            type="button"
            className="dp-banner__dismiss"
            aria-label="关闭提示"
            onClick={() => setNotice(null)}
          >
            ×
          </button>
        </div>
      ) : null}

      {startError ? (
        <ErrorBlock
          error={startError}
          title="发起部署失败"
          onRetry={() => void handleDeploy()}
          retryLabel="重试部署"
        />
      ) : null}

      <DeployStatusCard state={deployState} now={now} starting={starting} />

      <DeployCapabilityCard
        env={env}
        loading={envLoading}
        error={envError}
        onReload={() => void reloadEnv()}
      />

      <DeployHistoryList
        items={history}
        loading={historyLoading}
        error={historyError}
        state={deployState}
        observedTimes={observedTimes}
        onReload={() => void reloadHistory()}
      />

      <p className="dp-footer-note">
        提示：部署期间输入法可能短暂无响应或候选异常，通常数秒内恢复（FR-5.8）；并发部署请求会自动合并，
        同一时刻只跑一轮（FR-5.6）。
      </p>
    </div>
  );
}