/**
 * 备份页的 IPC 门面：把 `docs/ipc.md` §3.6 / §3.7 的命令收敛成强类型函数。
 *
 * 数据来源切换（与 `src/pages/workbench/api.ts` 保持一致）：
 * - Tauri 宿主内 → `call()`（`src/ipc/client.ts`）；
 * - 纯浏览器（`npm run dev`）→ `src/ipc/mock.ts`，不装 Rust 也能开发调试。
 *
 * 参数命名注意：Tauri v2 把 JS 的 camelCase 参数映射到 Rust 的 snake_case 形参；
 * 结构体参数名以后端签名为准（这里只有 `backup_restore(args: BackupRestoreArgs)`）。
 */
import { call, isTauri } from "../../ipc/client";
import { mockInvoke } from "../../ipc/mock";
import type { AppSettings, BackupRef, FileDiff, SaveResult } from "../../ipc/types";

/** `backup_restore` 出参（`docs/ipc.md` §3.6）：dry-run 时 `result` 为空 */
export interface RestoreOutcome {
  diff: FileDiff;
  result?: SaveResult | null;
}

/** 当前是否是纯前端 mock 模式（用于展示「模拟数据」标识） */
export function isMockMode(): boolean {
  return !isTauri();
}

async function backend<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    return mockInvoke<T>(cmd, args ?? {});
  }
  return call<T>(cmd, args);
}

export const ipc = {
  /**
   * `docs/ipc.md` §3.6 `backup_list`
   * 后端签名 `backup_list(file_id: Option<FileId>)`；传 null = 全部受管文件的备份。
   */
  listBackups: (): Promise<BackupRef[]> => backend<BackupRef[]>("backup_list", { fileId: null }),

  /**
   * `docs/ipc.md` §3.6 `backup_now`
   * 后端签名 `backup_now(file_ids: Option<Vec<FileId>>)`；不传 = 备份全部受管文件。
   */
  backupNow: (): Promise<BackupRef[]> => backend<BackupRef[]>("backup_now", { fileIds: null }),

  /**
   * `docs/ipc.md` §3.6 `backup_restore`
   *
   * **数据安全红线（PRD FR-6.5）**：调用方必须先 `dryRun: true` 拿到 `diff` 并让用户确认，
   * 才允许 `dryRun: false`；本函数不做任何「自动补一次真实恢复」的兜底。
   *
   * 参数形状：真实后端是单结构体参数 `backup_restore(args: { backupId, dryRun })`，
   * 所以 Tauri 侧要包一层 `{ args: {...} }`；mock 直接读顶层字段。两个都带上，
   * 让浏览器 mock 与真实宿主都能工作（多余字段会被 Tauri/mock 忽略）。
   */
  restore: (backupId: string, dryRun: boolean): Promise<RestoreOutcome> =>
    backend<RestoreOutcome>("backup_restore", {
      args: { backupId, dryRun },
      backupId,
      dryRun,
    }),

  /** `docs/ipc.md` §3.6 `backup_delete`（出参 `{ deleted }`） */
  deleteBackups: (backupIds: string[]): Promise<{ deleted: number }> =>
    backend<{ deleted: number }>("backup_delete", { backupIds }),

  /** `docs/ipc.md` §3.7 `settings_get` */
  getSettings: (): Promise<AppSettings> => backend<AppSettings>("settings_get"),

  /**
   * `docs/ipc.md` §3.7 `settings_update`（部分更新：只提交 `backup` 段）
   * 后端只覆盖显式给出的字段，未提供的保持原值。
   */
  updateBackupSettings: (backup: AppSettings["backup"]): Promise<AppSettings> =>
    backend<AppSettings>("settings_update", { patch: { backup } }),

  /**
   * `docs/ipc.md` §3.7 `settings_open_dir`（`which: "backup"`）。
   * 注意：M1 后端尚未实现该命令（见交付说明），失败时页面降级为「复制路径」。
   */
  openBackupDir: (): Promise<{ ok: boolean }> =>
    backend<{ ok: boolean }>("settings_open_dir", { which: "backup" }),
};

export default ipc;