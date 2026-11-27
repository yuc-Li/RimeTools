/**
 * IPC 客户端：UI 访问 Rust 核心的唯一入口。
 *
 * 设计要点：
 * - 所有命令统一经 `call()`，自动把后端错误规整为 `AppError`，UI 层只处理一种错误形态。
 * - 在非 Tauri 环境（纯浏览器 `vite dev`）下降级为 mock 实现，便于独立开发 UI。
 */
import type { AppError } from "./types";

type InvokeFn = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

interface TauriWindow {
  __TAURI_INTERNALS__?: { invoke?: InvokeFn };
}

let invokeImpl: InvokeFn | null = null;

function detectInvoke(): InvokeFn | null {
  if (typeof window === "undefined") return null;
  const w = window as unknown as TauriWindow;
  const fn = w.__TAURI_INTERNALS__?.invoke;
  return typeof fn === "function" ? fn : null;
}

/** 是否运行在 Tauri 宿主内（决定能力探测的取数来源） */
export function isTauri(): boolean {
  return detectInvoke() !== null;
}

function getInvoke(): InvokeFn {
  if (!invokeImpl) {
    const fn = detectInvoke();
    if (!fn) {
      throw {
        code: "internal",
        message: "当前不在 Tauri 运行时内，后端命令不可用。",
        hint: "请使用 `npm run dev` 配合 Tauri 宿主启动，或启用 mock 模式。",
      } satisfies AppError;
    }
    invokeImpl = fn;
  }
  return invokeImpl;
}

function isAppError(e: unknown): e is AppError {
  return typeof e === "object" && e !== null && "code" in e && "message" in e;
}

// ───────────────────────── 设置变更通知 ─────────────────────────
//
// 设置页改完主题/自动部署等，壳层需要立刻感知（例如切换暗色）。
// 这里在 IPC 层集中分发，避免每个页面自己造通知机制、也避免壳层轮询。

type SettingsListener = () => void;
const settingsListeners = new Set<SettingsListener>();

/** 订阅「设置已变更」，返回取消订阅函数。 */
export function onSettingsChanged(fn: SettingsListener): () => void {
  settingsListeners.add(fn);
  return () => {
    settingsListeners.delete(fn);
  };
}

function notifySettingsChanged() {
  for (const l of settingsListeners) {
    try {
      l();
    } catch (e) {
      console.warn("设置变更监听器异常", e);
    }
  }
}

/** 调用后端命令；失败时抛出规整后的 AppError */
export async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    const result = await getInvoke()<T>(cmd, args);
    // 设置类命令成功后广播，壳层据此刷新主题等全局状态
    if (cmd === "settings_update" || cmd === "settings_reset" || cmd === "env_set_user_dir") {
      notifySettingsChanged();
    }
    return result;
  } catch (e) {
    if (isAppError(e)) throw e;
    const message = e instanceof Error ? e.message : String(e);
    throw {
      code: "internal",
      message: `调用 ${cmd} 失败：${message}`,
      detail: e instanceof Error ? e.stack : undefined,
    } satisfies AppError;
  }
}

/** 把任意异常转成可展示的 AppError，避免 UI 层到处 try/catch 类型判断 */
export function toAppError(e: unknown): AppError {
  if (isAppError(e)) return e;
  return {
    code: "internal",
    message: e instanceof Error ? e.message : String(e),
  };
}