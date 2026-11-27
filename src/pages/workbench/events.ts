/**
 * 工作台事件总线（`docs/ipc.md` §4 / §6.4）。
 *
 * 契约要求 `dict://external-change`、`deploy://state` **全局订阅、不随页面卸载**。
 * 真正的 Tauri 事件监听需要 `@tauri-apps/api/event` 的 `listen()`；当前工程还没装该依赖，
 * 所以这里提供一个**模块级**总线：App 壳层接入 `listen()` 后，把 payload 转发给
 * `emitExternalChange()` / `emitDeployState()` 即可，工作台页面只订阅本模块。
 */
import type { DeployState, ExternalChangeEvent } from "../../ipc/types";

type Listener<T> = (payload: T) => void;

const externalListeners = new Set<Listener<ExternalChangeEvent>>();
const deployListeners = new Set<Listener<DeployState>>();

/** 订阅「检测到外部修改」。返回取消订阅函数。 */
export function onExternalChange(listener: Listener<ExternalChangeEvent>): () => void {
  externalListeners.add(listener);
  return () => externalListeners.delete(listener);
}

/** 由事件桥（Tauri `listen("dict://external-change")` 或 mock 调试按钮）调用 */
export function emitExternalChange(payload: ExternalChangeEvent): void {
  for (const listener of externalListeners) listener(payload);
}

/** 订阅部署状态（`deploy://state`） */
export function onDeployState(listener: Listener<DeployState>): () => void {
  deployListeners.add(listener);
  return () => deployListeners.delete(listener);
}

/** 由事件桥（Tauri `listen("deploy://state")`）调用 */
export function emitDeployState(payload: DeployState): void {
  for (const listener of deployListeners) listener(payload);
}