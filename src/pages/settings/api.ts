/**
 * 设置页的 IPC 门面：把 `docs/ipc.md` §3.1（`env_*`）与 §3.7（`settings_*`）收敛成强类型函数。
 *
 * 数据来源切换与工作台一致：
 * - Tauri 宿主内 → `call()`（`src/ipc/client.ts`）；
 * - 纯浏览器（`npm run dev`）→ `src/ipc/mock.ts`，保证不装 Rust 也能预览。
 *
 * 参数命名：Tauri v2 把 JS 的 camelCase 参数映射到 Rust 的 snake_case 形参，因此：
 * - `env_set_user_dir(path)` → `{ path }`
 * - `env_list_schemes(user_dir: Option<String>)` → `{ userDir }`
 * - `settings_update(patch)` → `{ patch }`，`patch` 内部字段保持 camelCase
 *   （`autoDeployAfterSave` / `backup.keepCount` …）
 */

import { call, isTauri, toAppError } from "../../ipc/client";
import { mockInvoke } from "../../ipc/mock";
import type { AppError, AppSettings, FrontendInfo, SchemeInfo } from "../../ipc/types";

/**
 * `env_detect` 的返回结构。
 * 契约 §3.1 只写了 `{ frontends, active }`，`types.ts` 未单独定义该包装类型，故在此声明。
 */
export interface EnvDetectResult {
  frontends: FrontendInfo[];
  active: FrontendInfo | null;
}

/**
 * `env_set_user_dir` 的返回：
 * - 真实后端（当前实现）返回重新探测到的 `Vec<SchemeInfo>`；
 * - `docs/ipc.md` 写的是 `AppSettings`（契约与实现此处有出入）。
 * 两种都兼容，调用方拿到数组就刷新方案列表、拿到对象就刷新设置。
 */
export type UserDirResult = SchemeInfo[] | AppSettings;

async function backend<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    return mockInvoke<T>(cmd, args ?? {});
  }
  return call<T>(cmd, args);
}

export const settingsApi = {
  /** §3.1 `env_detect`：自动探测前端；多前端时 `active` 为空（不静默猜测） */
  detectEnv: (): Promise<EnvDetectResult> => backend<EnvDetectResult>("env_detect"),

  /** §3.1 `env_list_schemes`：方案 + 受管文件映射；`userDir: null` 表示用后端当前目录 */
  listSchemes: (userDir: string | null = null): Promise<SchemeInfo[]> =>
    backend<SchemeInfo[]>("env_list_schemes", { userDir }),

  /** §3.1 `env_set_user_dir`：手动指定用户目录（失败时返回 `AppError.hint`） */
  setUserDir: (path: string): Promise<UserDirResult> => setUserDirImpl(path),

  /** §3.7 `settings_get` */
  getSettings: (): Promise<AppSettings> => backend<AppSettings>("settings_get"),

  /**
   * §3.7 `settings_update`：**部分更新**，只传要改的字段。
   * 字段名 camelCase；`backup` 传完整对象（真实后端按字段合并，mock 整对象替换）。
   */
  updateSettings: (patch: Partial<AppSettings>): Promise<AppSettings> =>
    backend<AppSettings>("settings_update", { patch }),

  /**
   * §3.7 `settings_reset`：恢复默认。契约入参 `{ keep_dict_data }`；
   * 当前 Rust 实现无此形参（多余参数被 Tauri 忽略），这里仍按契约显式表达
   * 「保留词库数据」的意图，绝不删除词库/备份。
   */
  resetSettings: (): Promise<AppSettings> =>
    backend<AppSettings>("settings_reset", { keepDictData: true }),
};

async function setUserDirImpl(path: string): Promise<UserDirResult> {
  if (isTauri()) {
    return call<UserDirResult>("env_set_user_dir", { path });
  }

  try {
    return await mockInvoke<UserDirResult>("env_set_user_dir", { path });
  } catch (error) {
    const err = toAppError(error);
    if (!err.message.includes("mock 未实现")) throw err;
    // ── 浏览器预览替身 ─────────────────────────────────────────────
    // `src/ipc/mock.ts` 由工作台任务维护，当前尚未实现 `env_set_user_dir`。
    // 为了在纯浏览器里也能演示「成功 / 失败（含 hint）」两条路径，这里用
    // `env_detect` 给出的模拟目录做最小校验：只接受 mock 的用户目录本身，
    // 选共享目录或其它路径都按真实后端语义报 `userDirInvalid`。
    // 真实环境（Tauri）永远走上面的 `call("env_set_user_dir")`，不受影响。
    return mockSetUserDir(path);
  }
}

async function mockSetUserDir(path: string): Promise<SchemeInfo[]> {
  const detected = await mockInvoke<EnvDetectResult>("env_detect");
  const front = detected.active ?? detected.frontends[0] ?? null;
  const normalize = (value: string) => value.replace(/[\\/]+$/, "");

  if (front && normalize(front.userDir) === normalize(path)) {
    await mockInvoke<AppSettings>("settings_update", { patch: { userDir: path } });
    return mockInvoke<SchemeInfo[]>("env_list_schemes", { userDir: path });
  }

  if (front?.sharedDir && normalize(front.sharedDir) === normalize(path)) {
    throw {
      code: "userDirInvalid",
      message: `这是共享目录，不是 Rime 用户目录：${path}`,
      hint: "共享目录只读，方案与系统词典都在那里；请选择用户目录，例如 /home/你/.local/share/fcitx5/rime。",
    } satisfies AppError;
  }

  throw {
    code: "userDirInvalid",
    message: `目录不存在或未包含 Rime 方案：${path}`,
    hint: `浏览器 mock 只模拟了 ${front?.userDir ?? "一个用户目录"}；真实目录校验请用 ./scripts/dev.sh 在 Tauri 宿主中测试。`,
  } satisfies AppError;
}

/** 纯前端 mock 模式（用于页面上的「Mock 预览」标识） */
export function isMockMode(): boolean {
  return !isTauri();
}