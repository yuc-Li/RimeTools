/**
 * 部署页的 IPC 门面（契约 `docs/ipc.md` §3.1 / §3.5）。
 *
 * 数据来源切换与工作台一致：
 * - Tauri 宿主内 → `call()`（`src/ipc/client.ts`，错误已规整为 `AppError`）；
 * - 纯浏览器（`npm run dev`）→ `src/ipc/mock.ts`（约 1.2s 推进 preparing→running→verifying→success）。
 *
 * 注意：`deploy://state` 事件由 `src/ipc/events.ts` 全局订阅，本页只用 `useDeployState()`
 * 读快照，不重复订阅；这里只负责命令调用。
 */
import { call, isTauri } from "../../ipc/client";
import { mockInvoke } from "../../ipc/mock";
import type { DeployResult, DeployTrigger, FrontendInfo } from "../../ipc/types";

/** `env_detect` 出参（`docs/ipc.md` §3.1） */
export interface EnvDetectResult {
  frontends: FrontendInfo[];
  active: FrontendInfo | null;
}

/** `deploy_start` 出参（`docs/ipc.md` §3.5 / §5.2 并发合并） */
export interface DeployStartResult {
  accepted: boolean;
  /** true 表示已有部署在跑，本次请求被合并；本轮结束后最多补跑一次 */
  merged: boolean;
}

async function backend<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!isTauri()) return mockInvoke<T>(cmd, args);
  return call<T>(cmd, args);
}

export const deployApi = {
  /** `docs/ipc.md` §3.1 `env_detect` —— 取前端与 `deployCapability` */
  envDetect: (): Promise<EnvDetectResult> => backend<EnvDetectResult>("env_detect"),

  /** `docs/ipc.md` §3.5 `deploy_start` —— 手动部署入口（触发源固定 manual） */
  start: (trigger: DeployTrigger = "manual"): Promise<DeployStartResult> =>
    backend<DeployStartResult>("deploy_start", { trigger }),

  /** `docs/ipc.md` §3.5 `deploy_history` —— 最近结果（默认 10 条） */
  history: (limit = 10): Promise<DeployResult[]> =>
    backend<DeployResult[]>("deploy_history", { limit }),
};