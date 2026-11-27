/**
 * 备份页的纯展示工具函数（无 React、无 IPC，便于单测）。
 *
 * 备份命名形如 `custom_phrase_double.txt_20260126_153045.bak`（PRD FR-6.1，兼容参考工具），
 * 来源文件路径可能很长（`~/.local/share/fcitx5/rime/...`），列表里只展示路径尾部；
 * 名字过长时省略**中间**、保留头尾，保证能同时看到文件名与时间戳后缀。
 */
import type { AppSettings, BackupReason, DiffSummary } from "../../ipc/types";

/** 契约 `BackupReason` → 中文（PRD FR-6 / `docs/ipc.md` §2） */
export const REASON_LABEL: Record<BackupReason, string> = {
  beforeWrite: "写前自动",
  manual: "手动",
  scheduled: "定期",
  beforeRestore: "恢复前",
  beforeDeploy: "部署前",
};

export function reasonLabel(reason: BackupReason): string {
  return REASON_LABEL[reason] ?? reason;
}

/** 备份原因对应的徽标色调（仅影响观感，不承载语义） */
export function reasonTone(reason: BackupReason): "muted" | "accent" | "info" | "warn" {
  switch (reason) {
    case "manual":
      return "accent";
    case "scheduled":
      return "info";
    case "beforeRestore":
      return "warn";
    case "beforeDeploy":
      return "info";
    case "beforeWrite":
    default:
      return "muted";
  }
}

/** 取路径尾部（兼容 POSIX `/` 与 Windows `\`） */
export function baseName(path: string): string {
  const parts = path.split(/[/\\]/).filter((part) => part !== "");
  return parts.length > 0 ? parts[parts.length - 1] : path;
}

/** 取路径的目录部分（用于展示备份目录） */
export function dirName(path: string): string {
  const index = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return index > 0 ? path.slice(0, index) : "";
}

/**
 * 省略中间：`custom_phrase_double.txt_20260126_153045.bak` → `custom_phrase_do…153045.bak`
 *
 * `tailShare` 控制尾部保留比例（默认 0.5）。备份文件名建议给 0.6–0.65：
 * 尾部的时间戳与 `.bak` 后缀比前缀更有辨识度，省略时优先保留。
 */
export function elideMiddle(text: string, max = 32, tailShare = 0.5): string {
  if (max < 8 || text.length <= max) return text;
  const share = Math.min(0.8, Math.max(0.2, tailShare));
  const tail = Math.round((max - 1) * share);
  const head = max - 1 - tail;
  return `${text.slice(0, head)}…${text.slice(text.length - tail)}`;
}

/** 字节数 → 人类可读的 KB / MB（任务要求按 KB/MB 展示；0 字节显示 `0 KB`） */
export function formatSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  const kb = bytes / 1024;
  if (kb < 1024) {
    if (kb === 0) return "0 KB";
    // 小于 10KB 保留一位小数（1.2 KB），更大的取整（256 KB），避免表格里数字跳动
    return `${kb < 10 ? kb.toFixed(1) : Math.round(kb)} KB`;
  }
  const mb = kb / 1024;
  return `${mb < 10 ? mb.toFixed(2) : mb.toFixed(1)} MB`;
}

/** Unix 毫秒 → 本地时间 `YYYY-MM-DD HH:mm:ss`（用户时区的本地时间） */
export function formatTime(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "—";
  const date = new Date(ms);
  const pad = (value: number) => String(value).padStart(2, "0");
  return (
    `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ` +
    `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`
  );
}

/** 相对时间；超过 30 天返回空串（此时绝对时间已足够，不必再说「N 天前」） */
export function relativeTime(ms: number, now: number = Date.now()): string {
  if (!Number.isFinite(ms) || ms <= 0) return "";
  const diff = Math.max(0, now - ms);
  if (diff < 60_000) return "刚刚";
  const minutes = Math.floor(diff / 60_000);
  if (minutes < 60) return `${minutes} 分钟前`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} 小时前`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days} 天前`;
  return "";
}

/** 差异总变更数（新增 + 删除 + 修改）；未变不算变更 */
export function diffChangeCount(summary: DiffSummary): number {
  return summary.added + summary.removed + summary.modified;
}

/**
 * 解析「保留份数」输入：必须是 1–9999 的整数（对应 `backup.keepCount: u32`）。
 * 非法返回 null，由调用方给出内联提示。
 */
export function parseKeepCount(raw: string): number | null {
  const trimmed = raw.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const value = Number(trimmed);
  if (!Number.isInteger(value) || value < 1 || value > 9999) return null;
  return value;
}

/**
 * 解析备份目录：优先 `backup.dir`；未显式配置时后端用「用户目录/backups」
 * （`src-tauri/src/commands/backup.rs` 的 `backup_dir()`），这里按同一规则展示，
 * 用户目录未知时返回 null（由后端自动探测，前端不猜）。
 */
export function resolveBackupDir(settings: AppSettings | null): string | null {
  if (!settings) return null;
  if (settings.backup?.dir) return settings.backup.dir;
  if (!settings.userDir) return null;
  return `${settings.userDir.replace(/[/\\]+$/, "")}/backups`;
}