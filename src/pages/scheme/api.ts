/**
 * 「输入法配置」页的 IPC 门面（`docs/ipc.md` §3.10）。
 *
 * 数据来源与其它页一致：Tauri 宿主内走 `call()`，纯浏览器走 mock。
 */

import { call, isTauri } from "../../ipc/client";
import { mockInvoke } from "../../ipc/mock";
import type {
  MountToggle,
  OptionUpdate,
  SchemeConfigSnapshot,
  SchemeConfigUpdateResult,
  SchemeInfo,
} from "../../ipc/types";

async function backend<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    return mockInvoke<T>(cmd, args ?? {});
  }
  return call<T>(cmd, args);
}

export const schemeApi = {
  /** 方案列表（`env_list_schemes` 同时会把受管文件注册进后端状态） */
  listSchemes: (): Promise<SchemeInfo[]> =>
    backend<SchemeInfo[]>("env_list_schemes", { userDir: null }),

  /** 读取指定方案的配置快照；`schemaId` 为空时用后端当前激活方案 */
  getConfig: (schemaId: string | null = null): Promise<SchemeConfigSnapshot> =>
    backend<SchemeConfigSnapshot>("scheme_config_get", { schemaId }),

  /** 更新配置：选项写 custom.yaml 的 patch，挂载改 import_tables */
  updateConfig: (request: {
    schemaId: string;
    autoSelect?: boolean;
    maxCodeLength?: number;
    mounts?: MountToggle[];
    options?: OptionUpdate[];
    deployAfter?: boolean;
  }): Promise<SchemeConfigUpdateResult> =>
    backend<SchemeConfigUpdateResult>("scheme_config_update", { request }),
};