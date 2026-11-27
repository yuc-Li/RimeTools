/**
 * 部署页展示辅助：触发源/前端文案、时间与耗时格式化、结果判定、手动部署指引。
 *
 * 结果判定严格遵循 PRD FR-5.5：
 * `success === true && outputVerified === false` 表示「命令执行成功，但 build/ 产物没有变化」，
 * 必须归为**警告**，绝不能显示成绿色成功（那是假成功，会误导用户以为改动已生效）。
 */
import { isDeploying } from "../../ipc/events";
import type {
  AppError,
  DeployPhase,
  DeployResult,
  DeployState,
  DeployTrigger,
  FrontendInfo,
  FrontendKind,
} from "../../ipc/types";

export type Tone = "muted" | "info" | "accent" | "ok" | "warn" | "danger";

// ───────────────────────── 文案 ─────────────────────────

export const TRIGGER_LABEL: Record<DeployTrigger, string> = {
  manual: "手动",
  afterSave: "保存后自动",
  scheduled: "定时",
};

export function triggerLabel(trigger: DeployTrigger): string {
  return TRIGGER_LABEL[trigger] ?? trigger;
}

export const FRONTEND_LABEL: Record<FrontendKind, string> = {
  weasel: "小狼毫",
  fcitx5Rime: "fcitx5-rime",
  ibusRime: "ibus-rime",
  unknown: "未知前端",
};

/** 阶段徽标配色（文案统一用 `DEPLOY_PHASE_LABEL`） */
export const PHASE_TONE: Record<DeployPhase, Tone> = {
  idle: "muted",
  preparing: "info",
  running: "accent",
  verifying: "info",
  success: "ok",
  failed: "danger",
  cancelled: "warn",
};

export function methodLabel(method: string): string {
  switch (method) {
    case "dbus-setconfig":
      return "D-Bus SetConfig（fcitx5）";
    case "rime-deployer":
      return "rime_deployer 预构建";
    case "weasel-deployer":
      return "WeaselDeployer";
    case "none":
      return "无自动部署通道";
    default:
      return method || "未知方式";
  }
}

export function frontendLabel(frontend: FrontendInfo): string {
  const kind = FRONTEND_LABEL[frontend.kind] ?? frontend.kind;
  const name = frontend.displayName || kind;
  return frontend.version ? `${name} · v${frontend.version}` : name;
}

// ───────────────────────── 结果判定 ─────────────────────────

export interface DeployVerdict {
  tone: "ok" | "warn" | "danger" | "muted";
  title: string;
  detail?: string;
  /** 命令成功但未验证到 build/ 产物变化（FR-5.5 重点） */
  unverified: boolean;
}

export function classifyDeployResult(result: DeployResult | null | undefined): DeployVerdict {
  if (!result) {
    return { tone: "muted", title: "尚无部署结果", unverified: false };
  }
  if (!result.success) {
    return {
      tone: "danger",
      title: "部署失败",
      detail: result.error?.message ?? "后端未返回失败原因，请查看部署日志",
      unverified: !result.outputVerified,
    };
  }
  if (!result.outputVerified) {
    return {
      tone: "warn",
      title: "命令执行成功，但未检测到 build/ 产物变化，改动可能未生效",
      detail:
        "这不能视为部署成功：命令退出成功只说明触发动作完成，无法证明输入法已加载新的 build/ 产物。" +
        "请确认 fcitx5 正在运行、当前方案已启用，或重启 fcitx5 后重新部署。",
      unverified: true,
    };
  }
  return { tone: "ok", title: "部署成功，build/ 产物已更新", unverified: false };
}

/** 失败结果兜底成可展示的 `AppError`（正常情况后端一定带 error） */
export function resultError(result: DeployResult): AppError {
  return (
    result.error ?? {
      code: "deployFailed",
      message: "部署失败，但后端未返回具体错误信息",
      hint: "请查看部署日志或稍后重试。",
    }
  );
}

export function isDeployTerminal(phase: DeployPhase): boolean {
  return phase === "success" || phase === "failed" || phase === "cancelled";
}

// ───────────────────────── 时间 / 耗时 ─────────────────────────

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

/** Unix 毫秒 → 本地 `YYYY-MM-DD HH:mm:ss`；空值显示「—」 */
export function formatDateTime(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms)) return "—";
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return "—";
  return (
    `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ` +
    `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`
  );
}

export function formatDuration(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms) || ms < 0) return "—";
  if (ms < 1000) return `${Math.round(ms)} 毫秒`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)} 秒`;
  const minutes = Math.floor(seconds / 60);
  const rest = Math.round(seconds % 60);
  return rest > 0 ? `${minutes} 分 ${rest} 秒` : `${minutes} 分`;
}

/**
 * 当前轮耗时：部署中按 `now - startedAt` 实时走；结束后优先用后端返回的 `durationMs`。
 */
export function deployElapsedMs(state: DeployState, now: number): number | null {
  if (isDeploying(state) && state.startedAt != null) {
    return Math.max(0, now - state.startedAt);
  }
  if (state.lastResult) return state.lastResult.durationMs;
  if (state.startedAt != null && state.finishedAt != null) {
    return Math.max(0, state.finishedAt - state.startedAt);
  }
  return null;
}

function sameResult(a: DeployResult | undefined, b: DeployResult): boolean {
  return (
    !!a &&
    a.trigger === b.trigger &&
    a.success === b.success &&
    a.durationMs === b.durationMs &&
    a.outputVerified === b.outputVerified
  );
}

/** 结果指纹：用于把本页观察到的完成时刻关联到无时间戳的历史条目 */
export function resultKey(result: DeployResult): string {
  return [
    result.trigger,
    result.success,
    result.durationMs,
    result.outputVerified,
    result.logPath ?? "",
  ].join("|");
}

/**
 * 推断历史条目的完成时刻。
 *
 * 契约里的 `DeployResult`（`docs/ipc.md` §2）**不含完成时间**，只有 `durationMs`：
 * - Rust 侧部署日志文件名是 `deploy-<unix_ms>.log`，可从 `logPath` 反推真实时刻；
 * - 本页在会话内观察到的终态时刻（`observedTimes`）按结果指纹回填；
 * - 最新一条若与 `DeployState.lastResult` 一致，可用当前状态的 `finishedAt`；
 * - 其余只能显示「—」（契约缺口，已反馈 lead）。
 */
export function inferHistoryFinishedAt(
  item: DeployResult,
  index: number,
  state: DeployState,
  observedTimes?: Record<string, number>,
): number | null {
  const match = /deploy-(\d{10,})\.log$/.exec(item.logPath ?? "");
  if (match) {
    const ms = Number(match[1]);
    if (Number.isFinite(ms)) return ms;
  }
  const observed = observedTimes?.[resultKey(item)];
  if (observed != null && Number.isFinite(observed)) return observed;
  if (index === 0 && state.finishedAt != null && sameResult(state.lastResult, item)) {
    return state.finishedAt;
  }
  return null;
}

// ───────────────────────── 手动部署指引（降级路径） ─────────────────────────

export interface ManualDeployGuide {
  steps: string[];
  /** 可选兜底命令（rime_deployer 预构建） */
  command?: string;
}

/**
 * 根据前端类型给出手动部署方法（FR-5.4 兜底路径）。
 * 自动部署不可用时，用户至少要有一条可执行的替代路径。
 */
export function manualDeployGuide(frontend: FrontendInfo | null): ManualDeployGuide {
  const kind: FrontendKind = frontend?.kind ?? "unknown";
  const userDir = frontend?.userDir ?? "<Rime 用户目录>";
  const sharedDir = frontend?.sharedDir ?? "/usr/share/rime-data";
  const command = `rime_deployer --build "${userDir}" "${sharedDir}" "${userDir}/build"`;

  switch (kind) {
    case "weasel":
      return {
        steps: [
          "右键任务栏的小狼毫图标 →「重新部署」。",
          "或运行安装目录下的 WeaselDeployer.exe /deploy；无需重启 WeaselServer。",
        ],
      };
    case "fcitx5Rime":
      return {
        steps: [
          "在 fcitx5 托盘图标上右键 →「重新部署」（部分版本没有该菜单，见下一条）。",
          "重启 fcitx5：终端执行 fcitx5 -r，或注销会话后重新登录。",
          "如仍不生效，先执行下面的 rime_deployer 预构建命令，再重启 fcitx5。",
        ],
        command,
      };
    case "ibusRime":
      return {
        steps: [
          "重启 ibus：终端执行 ibus restart，再切换一次 Rime 输入法触发重载。",
          "如仍不生效，先执行下面的 rime_deployer 预构建命令，再重启 ibus。",
        ],
        command,
      };
    default:
      return {
        steps: [
          "在所用输入法的托盘/设置中执行「重新部署」。",
          "或重启输入法宿主进程（fcitx5 / ibus / 小狼毫）。",
          "若前端只提供 rime_deployer，可先执行下面的预构建命令，再重启前端。",
        ],
        command,
      };
  }
}