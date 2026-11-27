/**
 * 备份与恢复页（M1 / PRD FR-6.1–6.6）。
 *
 * IPC 对应（`docs/ipc.md`）：
 * - §3.6 `backup_list`    —— 备份列表（按 `createdAt` 倒序展示）
 * - §3.6 `backup_now`     —— 立即备份全部受管文件
 * - §3.6 `backup_restore` —— **必须先 `dryRun: true` 出差异，用户确认后才 `dryRun: false`**
 *                            （数据安全红线 PRD FR-6.5，见 `RestoreDialog.tsx`）
 * - §3.6 `backup_delete`  —— 多选删除（二次确认）
 * - §3.7 `settings_get` / `settings_update` —— 读 / 改 `backup.keepCount`
 * - §3.7 `settings_open_dir` —— 打开备份目录（M1 后端未实现，失败时降级为复制路径）
 *
 * 产品约束：
 * - 恢复**不允许**跳过 dry-run；恢复前后端会自动备份当前版本（reason=beforeRestore），可再回滚；
 * - 备份文件名形如 `custom_phrase_double.txt_20260126_153045.bak`，来源路径可能很长，
 *   列表只取路径尾部、过长省略中间（见 `format.ts` 的 `elideMiddle`）；
 * - 「保留份数」超出后的清理发生在**写入词库**之后（后端 `prune_keep_count`），立即备份不会触发。
 *
 * App.tsx 由 lead 接线：本组件为默认导出，无必填 props。
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { toAppError } from "../../ipc/client";
import type { AppError, AppSettings, BackupRef, SaveResult } from "../../ipc/types";
import { Badge, EmptyState, ErrorAlert, Spinner } from "../../components/Feedback";
import { BackupConfirmDialog } from "./BackupConfirmDialog";
import { RestoreDialog } from "./RestoreDialog";
import { ipc, isMockMode } from "./api";
import {
  baseName,
  elideMiddle,
  formatSize,
  formatTime,
  parseKeepCount,
  reasonLabel,
  reasonTone,
  relativeTime,
  resolveBackupDir,
} from "./format";
import "./backup.css";

type LoadState = "loading" | "ready" | "error";

type BannerTone = "ok" | "warn" | "danger";

interface Banner {
  tone: BannerTone;
  title: string;
  detail?: string;
  /** 恢复后自动备份的「当前版本」文件名，提示用户还能回滚 */
  protectName?: string | null;
  /** 常驻提示（恢复结果）不自动消失 */
  sticky?: boolean;
}

export default function Backup() {
  // ── 备份列表 ──
  const [backups, setBackups] = useState<BackupRef[] | null>(null);
  const [loadState, setLoadState] = useState<LoadState>("loading");
  const [loadError, setLoadError] = useState<AppError | null>(null);

  // ── 操作态 ──
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [backingUp, setBackingUp] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [actionError, setActionError] = useState<AppError | null>(null);
  const [banner, setBanner] = useState<Banner | null>(null);
  const [restoreTarget, setRestoreTarget] = useState<BackupRef | null>(null);
  const [deleteTargets, setDeleteTargets] = useState<BackupRef[] | null>(null);

  // ── 备份策略（settings.backup） ──
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [settingsError, setSettingsError] = useState<AppError | null>(null);
  const [keepDraft, setKeepDraft] = useState("");
  const [keepError, setKeepError] = useState<string | null>(null);
  const [savingKeep, setSavingKeep] = useState(false);
  const [dirNotice, setDirNotice] = useState<{ tone: "ok" | "warn"; text: string } | null>(null);

  const mockMode = isMockMode();

  // ── 读取备份列表（§3.6 backup_list），统一倒序 ──
  const loadBackups = useCallback(async (silent = false) => {
    if (!silent) {
      setLoadState("loading");
      setLoadError(null);
    }
    try {
      const list = await ipc.listBackups();
      const sorted = [...list].sort((a, b) => b.createdAt - a.createdAt);
      setBackups(sorted);
      setLoadState("ready");
      // 列表变了（删除/清理）后丢弃失效的选中项
      setSelected((prev) => {
        const alive = new Set(sorted.map((item) => item.id));
        const next = new Set([...prev].filter((id) => alive.has(id)));
        return next.size === prev.size ? prev : next;
      });
    } catch (error) {
      setLoadError(toAppError(error));
      setLoadState("error");
    }
  }, []);

  // ── 读取设置（§3.7 settings_get），拿 backup.keepCount / dir ──
  const loadSettings = useCallback(async () => {
    setSettingsError(null);
    try {
      const value = await ipc.getSettings();
      setSettings(value);
      setKeepDraft(String(value.backup.keepCount));
    } catch (error) {
      setSettingsError(toAppError(error));
    }
  }, []);

  useEffect(() => {
    void loadBackups();
    void loadSettings();
  }, [loadBackups, loadSettings]);

  // 普通提示 6 秒后自动消失；恢复结果常驻，等用户确认
  useEffect(() => {
    if (!banner || banner.sticky) return;
    const timer = setTimeout(() => setBanner(null), 6000);
    return () => clearTimeout(timer);
  }, [banner]);

  const selectedBackups = useMemo(
    () => (backups ?? []).filter((item) => selected.has(item.id)),
    [backups, selected],
  );
  const allSelected =
    (backups?.length ?? 0) > 0 && selectedBackups.length === (backups?.length ?? 0);

  const dirPath = useMemo(() => resolveBackupDir(settings), [settings]);
  const keepDirty = settings ? keepDraft.trim() !== String(settings.backup.keepCount) : false;

  // ── 立即备份（§3.6 backup_now，不做乐观插入：成功后重新拉列表） ──
  const backupNow = async () => {
    if (backingUp || deleting) return;
    setBackingUp(true);
    setActionError(null);
    setBanner(null);
    try {
      const created = await ipc.backupNow();
      await loadBackups(true);
      const names = [...new Set(created.map((item) => baseName(item.sourcePath)))];
      setBanner({
        tone: created.length > 0 ? "ok" : "warn",
        title:
          created.length > 0
            ? `已新建 ${created.length} 份备份`
            : "后端没有可备份的受管文件（请先在设置页确认环境与受管文件）",
        detail:
          created.length > 0
            ? `来源：${names.slice(0, 4).join("、")}${names.length > 4 ? ` 等 ${names.length} 个文件` : ""}`
            : undefined,
      });
    } catch (error) {
      setActionError(toAppError(error));
    } finally {
      setBackingUp(false);
    }
  };

  // ── 删除备份（§3.6 backup_delete）：先弹二次确认，确认后才真删 ──
  const confirmDelete = async () => {
    if (!deleteTargets || deleteTargets.length === 0 || deleting) return;
    setDeleting(true);
    setActionError(null);
    setBanner(null);
    const count = deleteTargets.length;
    try {
      const result = await ipc.deleteBackups(deleteTargets.map((item) => item.id));
      await loadBackups(true);
      setDeleteTargets(null);
      setSelected(new Set());
      const missed = count - result.deleted;
      setBanner({
        tone: missed > 0 ? "warn" : "ok",
        title: `已删除 ${result.deleted} 份备份`,
        detail:
          missed > 0
            ? `有 ${missed} 份未删除（可能已被自动清理或文件已不存在）。`
            : undefined,
      });
    } catch (error) {
      setDeleteTargets(null);
      setActionError(toAppError(error));
    } finally {
      setDeleting(false);
    }
  };

  // ── 恢复成功回调（真正恢复走完 dry-run → 确认 → dryRun:false） ──
  const handleRestored = useCallback(
    (backup: BackupRef, result: SaveResult) => {
      setRestoreTarget(null);
      setActionError(null);
      const deploy = result.deploy;
      const protectName = result.backup ? baseName(result.backup.path) : null;
      const source = baseName(backup.sourcePath);

      if (!deploy) {
        setBanner({
          tone: "ok",
          title: "已恢复，尚未部署",
          detail: `${source} 已回滚到 ${formatTime(backup.createdAt)} 的备份；需到「部署」页部署后，输入法候选才会更新。`,
          protectName,
          sticky: true,
        });
      } else if (deploy.success && deploy.outputVerified) {
        setBanner({
          tone: "ok",
          title: "已恢复并重新部署",
          detail: `${source} 已回滚并部署（耗时 ${deploy.durationMs}ms，已检测到 build 产物变化）。`,
          protectName,
          sticky: true,
        });
      } else if (deploy.success) {
        setBanner({
          tone: "warn",
          title: "已恢复并触发部署，但未检测到产物变化",
          detail:
            "命令执行成功，但 build/ 产物未变化，改动可能尚未生效。请到「部署」页确认或重试部署。",
          protectName,
          sticky: true,
        });
      } else {
        setBanner({
          tone: "warn",
          title: "已恢复，但部署失败",
          detail: `${deploy.error?.message ?? "（未返回错误详情）"}${
            deploy.error?.hint ? ` ${deploy.error.hint}` : ""
          } 文件已恢复，可到「部署」页重试。`,
          protectName,
          sticky: true,
        });
      }
      void loadBackups(true);
    },
    [loadBackups],
  );

  // ── 保存保留份数（§3.7 settings_update 写 backup.keepCount） ──
  const saveKeepCount = async () => {
    if (!settings || savingKeep || !keepDirty) return;
    const value = parseKeepCount(keepDraft);
    if (value === null) {
      setKeepError("保留份数需为 1–9999 的整数");
      return;
    }
    setKeepError(null);
    setSavingKeep(true);
    try {
      const next = await ipc.updateBackupSettings({ ...settings.backup, keepCount: value });
      setSettings(next);
      setKeepDraft(String(next.backup.keepCount));
      setBanner({
        tone: "ok",
        title: `保留份数已更新为 ${next.backup.keepCount} 份`,
        detail: "超出份数的旧备份会在下次写入词库后自动清理。",
      });
    } catch (error) {
      setKeepError(toAppError(error).message);
    } finally {
      setSavingKeep(false);
    }
  };

  // ── 打开备份目录（§3.7 settings_open_dir）；后端未实现时降级为复制路径 ──
  const openDir = async () => {
    if (!dirPath) return;
    setDirNotice(null);
    try {
      await ipc.openBackupDir();
      setDirNotice({ tone: "ok", text: `已请求文件管理器打开：${dirPath}` });
    } catch (error) {
      const err = toAppError(error);
      setDirNotice({
        tone: "warn",
        text: `当前后端未提供「打开目录」命令，请用上方「复制路径」在文件管理器里打开：${err.message}`,
      });
    }
  };

  const copyDirPath = async () => {
    if (!dirPath) return;
    try {
      await navigator.clipboard.writeText(dirPath);
      setDirNotice({ tone: "ok", text: "已复制备份目录路径" });
    } catch {
      setDirNotice({ tone: "warn", text: "浏览器不允许写剪贴板，请手动选择路径复制。" });
    }
  };

  const toggleSelect = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const toggleAll = () => {
    if (!backups || backups.length === 0) return;
    setSelected(allSelected ? new Set() : new Set(backups.map((item) => item.id)));
  };

  // ───────────────────────── 渲染 ─────────────────────────

  return (
    <div className="backup">
      <header className="bk-header">
        <div>
          <h1>备份与恢复</h1>
          <p className="bk-header__sub">
            写入前会自动备份。恢复必须先看差异（dry-run）并确认，才会覆盖文件；恢复前还会再备份当前版本。
          </p>
        </div>
        <div className="bk-header__badges">
          {mockMode ? (
            <Badge tone="info">mock 数据（无 Tauri 后端）</Badge>
          ) : (
            <Badge tone="ok">Tauri 后端</Badge>
          )}
          <Badge tone="muted">共 {backups?.length ?? "…"} 份备份</Badge>
        </div>
      </header>

      {banner ? (
        <div className={`bk-banner bk-banner--${banner.tone}`} role="status">
          <div className="bk-banner__head">
            <strong>{banner.title}</strong>
            <button
              type="button"
              className="ui-btn ui-btn--sm ui-btn--ghost"
              onClick={() => setBanner(null)}
            >
              关闭
            </button>
          </div>
          {banner.detail ? <p>{banner.detail}</p> : null}
          {banner.protectName ? (
            <p className="bk-banner__protect">
              恢复前的当前版本已自动备份：<code title={banner.protectName}>{banner.protectName}</code>
            </p>
          ) : null}
        </div>
      ) : null}

      {actionError ? (
        <ErrorAlert
          error={actionError}
          onRetry={() => setActionError(null)}
          retryLabel="关闭提示"
          compact
        />
      ) : null}

      {/* ── 备份策略：保留份数 + 备份目录 ── */}
      <section className="bk-card" aria-label="备份策略">
        <div className="bk-card__head">
          <h2>备份策略</h2>
          {settings ? (
            <span className="bk-badges">
              {settings.backup.enabled ? (
                <Badge tone="ok">自动备份已开启</Badge>
              ) : (
                <Badge tone="warn">自动备份已关闭（设置页开启）</Badge>
              )}
              <Badge tone={settings.backup.scheduledEnabled ? "accent" : "muted"}>
                定期备份：{settings.backup.scheduledEnabled ? "已开启" : "未开启（M2 提供周期配置）"}
              </Badge>
            </span>
          ) : null}
        </div>

        {settingsError ? (
          <ErrorAlert
            error={settingsError}
            onRetry={() => void loadSettings()}
            retryLabel="重新读取设置"
            compact
          />
        ) : null}

        <div className="bk-strategy">
          <label className="bk-field">
            <span className="bk-field__label">保留份数</span>
            <div className="bk-field__row">
              <input
                className="bk-input bk-input--num"
                type="number"
                min={1}
                max={9999}
                step={1}
                value={keepDraft}
                disabled={!settings || savingKeep}
                onChange={(event) => {
                  setKeepDraft(event.target.value);
                  setKeepError(null);
                }}
                aria-label="保留份数"
              />
              <button
                type="button"
                className="ui-btn ui-btn--primary"
                onClick={() => void saveKeepCount()}
                disabled={!settings || savingKeep || !keepDirty}
              >
                {savingKeep ? "保存中…" : "保存"}
              </button>
            </div>
          </label>
          <p className="bk-note bk-muted">
            超出份数的旧备份会在<strong>下次写入词库</strong>后自动清理（「立即备份」不会触发清理）。
            {keepError ? <span className="bk-inline-error">{keepError}</span> : null}
          </p>
        </div>

        <div className="bk-dir">
          <span className="bk-field__label">备份目录</span>
          <code className="bk-dir__path" title={dirPath ?? undefined}>
            {dirPath ?? "（用户目录未知：后端会自动探测「用户目录/backups」）"}
          </code>
          <div className="bk-dir__actions">
            <button
              type="button"
              className="ui-btn ui-btn--sm"
              onClick={() => void openDir()}
              disabled={!dirPath}
              title="调用 settings_open_dir 打开备份目录"
            >
              打开备份目录
            </button>
            <button
              type="button"
              className="ui-btn ui-btn--sm"
              onClick={() => void copyDirPath()}
              disabled={!dirPath}
            >
              复制路径
            </button>
          </div>
          {dirNotice ? (
            <p className={`bk-note ${dirNotice.tone === "warn" ? "bk-note--warn" : "bk-note--ok"}`}>
              {dirNotice.text}
            </p>
          ) : null}
        </div>
      </section>

      {/* ── 备份列表 ── */}
      <section className="bk-card bk-list" aria-label="备份列表">
        <div className="bk-list__head">
          <h2>
            备份列表 <span className="bk-muted">（按时间倒序）</span>
          </h2>
          <div className="bk-list__actions">
            <button
              type="button"
              className="ui-btn ui-btn--primary"
              onClick={() => void backupNow()}
              disabled={backingUp || deleting}
            >
              {backingUp ? "备份中…" : "立即备份"}
            </button>
            <button
              type="button"
              className="ui-btn ui-btn--danger"
              onClick={() => setDeleteTargets(selectedBackups)}
              disabled={selectedBackups.length === 0 || deleting || backingUp}
            >
              删除选中{selectedBackups.length > 0 ? `（${selectedBackups.length}）` : ""}
            </button>
            <button
              type="button"
              className="ui-btn ui-btn--ghost"
              onClick={() => void loadBackups()}
              disabled={backingUp || deleting}
            >
              刷新
            </button>
          </div>
        </div>

        {loadState === "loading" && !backups ? (
          <div className="bk-center">
            <Spinner label="正在读取备份列表…" />
          </div>
        ) : null}

        {loadError ? (
          <ErrorAlert
            error={loadError}
            onRetry={() => void loadBackups()}
            retryLabel="重新读取"
            compact
          />
        ) : null}

        {loadState === "ready" && backups && backups.length === 0 ? (
          <EmptyState
            title="还没有任何备份"
            description="保存词库前会自动生成写前备份；也可以点「立即备份」手动备份当前受管文件。"
            action={
              <button
                type="button"
                className="ui-btn ui-btn--primary"
                onClick={() => void backupNow()}
                disabled={backingUp}
              >
                立即备份
              </button>
            }
          />
        ) : null}

        {backups && backups.length > 0 ? (
          <div className="bk-table" role="table" aria-label="备份列表">
            <div className="bk-row bk-row--head" role="row">
              <span className="bk-check" role="columnheader">
                <input
                  type="checkbox"
                  checked={allSelected}
                  onChange={toggleAll}
                  aria-label={allSelected ? "取消全选" : "全选备份"}
                />
              </span>
              <span role="columnheader">时间</span>
              <span role="columnheader">来源文件 / 备份文件</span>
              <span role="columnheader">大小</span>
              <span role="columnheader">原因</span>
              <span className="bk-cell--actions" role="columnheader">
                操作
              </span>
            </div>

            {backups.map((backup) => {
              const isSelected = selected.has(backup.id);
              const relative = relativeTime(backup.createdAt);
              return (
                <div
                  key={backup.id}
                  className={`bk-row${isSelected ? " is-selected" : ""}`}
                  role="row"
                >
                  <span className="bk-check" role="cell">
                    <input
                      type="checkbox"
                      checked={isSelected}
                      onChange={() => toggleSelect(backup.id)}
                      aria-label={`选择备份 ${baseName(backup.path)}`}
                    />
                  </span>
                  <span className="bk-cell bk-cell--time" role="cell">
                    <span
                      className="bk-time"
                      title={
                        backup.createdAt > 0 ? new Date(backup.createdAt).toISOString() : undefined
                      }
                    >
                      {formatTime(backup.createdAt)}
                    </span>
                    {relative ? <span className="bk-muted bk-sub">{relative}</span> : null}
                  </span>
                  <span className="bk-cell" role="cell">
                    <span className="bk-name" title={backup.sourcePath}>
                      {elideMiddle(baseName(backup.sourcePath), 30)}
                    </span>
                    <span className="bk-muted bk-sub" title={backup.path}>
                      备份：{elideMiddle(baseName(backup.path), 42, 0.62)}
                    </span>
                  </span>
                  <span className="bk-cell mono" role="cell">
                    {formatSize(backup.size)}
                  </span>
                  <span className="bk-cell" role="cell">
                    <Badge tone={reasonTone(backup.reason)} title={backup.reason}>
                      {reasonLabel(backup.reason)}
                    </Badge>
                  </span>
                  <span className="bk-cell bk-cell--actions" role="cell">
                    <button
                      type="button"
                      className="ui-btn ui-btn--sm"
                      onClick={() => setRestoreTarget(backup)}
                      disabled={deleting || backingUp}
                      title="先查看差异（dry-run），确认后才恢复"
                    >
                      恢复…
                    </button>
                  </span>
                </div>
              );
            })}
          </div>
        ) : null}

        <p className="bk-footer-note bk-muted">
          恢复流程：读取差异（dry-run）→ 你确认 → 后端先备份当前版本 → 写入备份内容。
          未确认前不会改动任何文件。
        </p>
      </section>

      {/* ── 恢复对话框（内部强制先 dry-run） ── */}
      {restoreTarget ? (
        <RestoreDialog
          key={restoreTarget.id}
          backup={restoreTarget}
          onClose={() => setRestoreTarget(null)}
          onRestored={handleRestored}
        />
      ) : null}

      {/* ── 删除二次确认 ── */}
      {deleteTargets ? (
        <BackupConfirmDialog
          backups={deleteTargets}
          busy={deleting}
          onCancel={() => setDeleteTargets(null)}
          onConfirm={() => void confirmDelete()}
        />
      ) : null}
    </div>
  );
}