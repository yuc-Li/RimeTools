/**
 * 事件订阅与共享状态（契约 `docs/ipc.md` §4）。
 *
 * 约定（§6.4）：`deploy://state`、`dict://external-change` 必须**全局订阅**，
 * 不随页面卸载中断。因此这里用模块级单例承载订阅与最新状态，页面只读快照。
 */
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { call, isTauri } from "./client";
import { IPC_EVENTS } from "./types";
import type { DeployPhase, DeployResult, DeployState, Fingerprint } from "./types";

type Listener = () => void;

// ───────────────────────── 部署状态 ─────────────────────────

let deployState: DeployState = {
  phase: "idle",
  startedAt: null,
  finishedAt: null,
};

const deployListeners = new Set<Listener>();
let deployBootstrapped = false;
let unlistenDeploy: UnlistenFn | null = null;

function emitDeployChange() {
  for (const l of deployListeners) l();
}

/** 供 mock 层调用：模拟状态推送 */
export function __setMockDeployState(next: DeployState) {
  deployState = next;
  emitDeployChange();
}

async function bootstrapDeploy() {
  if (deployBootstrapped) return;
  deployBootstrapped = true;

  // 先取一次当前状态，避免页面打开时显示 idle 却其实在部署
  try {
    const initial = await call<DeployState>("deploy_status");
    deployState = initial;
    emitDeployChange();
  } catch {
    // 状态查询失败不阻塞 UI；后续事件仍会更新
  }

  if (!isTauri()) return;

  try {
    unlistenDeploy = await listen<DeployState>(IPC_EVENTS.deployState, (event) => {
      deployState = event.payload;
      emitDeployChange();
    });
  } catch (e) {
    console.warn("订阅 deploy://state 失败", e);
  }
}

/** 订阅部署状态变化，返回取消订阅函数。 */
export function subscribeDeploy(fn: Listener): () => void {
  deployListeners.add(fn);
  void bootstrapDeploy();
  return () => {
    deployListeners.delete(fn);
  };
}

export function getDeployState(): DeployState {
  return deployState;
}

/** 部署是否在进行中（用于禁用按钮、显示进度）。 */
export function isDeploying(state: DeployState = deployState): boolean {
  return state.phase === "preparing" || state.phase === "running" || state.phase === "verifying";
}

/** 阶段的中文文案（集中维护，避免各页面各写一套）。 */
export const DEPLOY_PHASE_LABEL: Record<DeployPhase, string> = {
  idle: "空闲",
  preparing: "准备中",
  running: "部署中",
  verifying: "校验产物",
  success: "成功",
  failed: "失败",
  cancelled: "已取消",
};

export function describeDeployResult(r: DeployResult): string {
  if (!r.success) return r.error?.message ?? "部署失败";
  const ms = `${(r.durationMs / 1000).toFixed(1)}s`;
  return r.outputVerified
    ? `成功（${ms}，已确认构建产物更新）`
    : `命令执行成功（${ms}），但未检测到构建产物变化`;
}

// ───────────────────────── 外部修改事件 ─────────────────────────

export interface ExternalChangePayload {
  fileId: string;
  fingerprint: Fingerprint;
}

const externalListeners = new Set<(p: ExternalChangePayload) => void>();
let externalBootstrapped = false;

async function bootstrapExternal() {
  if (externalBootstrapped || !isTauri()) return;
  externalBootstrapped = true;
  try {
    await listen<ExternalChangePayload>(IPC_EVENTS.externalChange, (event) => {
      for (const l of externalListeners) l(event.payload);
    });
  } catch (e) {
    console.warn("订阅 dict://external-change 失败", e);
  }
}

/** 订阅「文件被外部修改」，返回取消订阅函数。 */
export function subscribeExternalChange(fn: (p: ExternalChangePayload) => void): () => void {
  externalListeners.add(fn);
  void bootstrapExternal();
  return () => {
    externalListeners.delete(fn);
  };
}

/** 释放全局订阅（应用退出/热重载时调用）。 */
export function disposeEventSubscriptions() {
  unlistenDeploy?.();
  unlistenDeploy = null;
  deployBootstrapped = false;
  externalBootstrapped = false;
}
// ───────────────────────── React Hooks ─────────────────────────

import { useSyncExternalStore } from "react";

/** 订阅部署状态（全局单例，页面切换不丢订阅）。 */
export function useDeployState(): DeployState {
  return useSyncExternalStore(subscribeDeploy, getDeployState, getDeployState);
}
