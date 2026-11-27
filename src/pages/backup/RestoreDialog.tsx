/**
 * 恢复确认对话框 —— **PRD FR-6.5 数据安全红线的落点**。
 *
 * 流程（`docs/ipc.md` §3.6 `backup_restore`）：
 * 1. 打开即调 `backup_restore({ backupId, dryRun: true })`，只读地拿到 `FileDiff`；
 * 2. 展示差异摘要（新增/删除/修改/未变）与明细（新增、删除各最多 20 条，可折叠）；
 * 3. 用户点「确认恢复」后才调 `dryRun: false` 真正写入。
 *
 * **禁止**在没有 dry-run 结果的情况下直接恢复：`doRestore()` 首先校验 `diff` 存在。
 * 另：真正恢复前后端会先把当前版本备份一份（reason = beforeRestore），恢复本身仍可回滚。
 */
import { useCallback, useEffect, useState } from "react";
import { toAppError } from "../../ipc/client";
import type { AppError, BackupRef, DiffLine, FileDiff, SaveResult } from "../../ipc/types";
import { ErrorAlert, Spinner } from "../../components/Feedback";
import { Modal } from "../../components/Modal";
import { ipc } from "./api";
import {
  baseName,
  diffChangeCount,
  elideMiddle,
  formatSize,
  formatTime,
  reasonLabel,
} from "./format";
import "./backup.css";

/** 每组明细最多展示条数（超出仅提示总数，明细可折叠） */
const DETAIL_LIMIT = 20;

type Phase = "dryRun" | "ready" | "error";

export interface RestoreDialogProps {
  backup: BackupRef;
  onClose: () => void;
  /** 真正恢复成功（dryRun:false 返回 result）后回调，由页面刷新列表并提示 */
  onRestored: (backup: BackupRef, result: SaveResult) => void;
}

export function RestoreDialog({ backup, onClose, onRestored }: RestoreDialogProps) {
  const [phase, setPhase] = useState<Phase>("dryRun");
  const [diff, setDiff] = useState<FileDiff | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [restoring, setRestoring] = useState(false);

  // 步骤 1：只读差异（dry-run）。这一步失败时不允许继续恢复。
  const runDryRun = useCallback(async () => {
    setPhase("dryRun");
    setError(null);
    setDiff(null);
    try {
      const outcome = await ipc.restore(backup.id, true);
      setDiff(outcome.diff);
      setPhase("ready");
    } catch (e) {
      setError(toAppError(e));
      setPhase("error");
    }
  }, [backup.id]);

  useEffect(() => {
    void runDryRun();
  }, [runDryRun]);

  const changed = diff ? diffChangeCount(diff.summary) : 0;
  const canRestore = phase === "ready" && diff !== null && changed > 0 && !restoring;

  // 步骤 3：用户确认后真正恢复。红线：没有 dry-run 结果就不发 dryRun:false。
  const doRestore = async () => {
    if (!diff || phase !== "ready" || restoring) return;
    setRestoring(true);
    setError(null);
    try {
      const outcome = await ipc.restore(backup.id, false);
      const result = outcome.result;
      if (!result) {
        // 后端仍按 dry-run 处理（理论上不会发生）：不做任何「补写」，只报错让用户重试。
        setError({
          code: "internal",
          message: "后端没有返回恢复结果（可能仍按 dry-run 处理），文件未被改动",
          hint: "请重试；若持续出现，请把这条提示和日志一起反馈。",
        });
        setRestoring(false);
        return;
      }
      onRestored(backup, result); // 父组件会关闭本对话框并刷新列表
    } catch (e) {
      setError(toAppError(e));
      setRestoring(false);
    }
  };

  return (
    <Modal
      open
      title="从备份恢复"
      width={760}
      tone="danger"
      dismissOnBackdrop={false}
      onClose={() => {
        if (!restoring) onClose();
      }}
      footer={
        <>
          <span className="bk-dialog-hint">
            {phase === "ready" && diff
              ? "请先核对上方差异；确认后才会写入文件（写入前会自动备份当前版本）。"
              : "先读取差异，确认后才允许恢复。"}
          </span>
          <button type="button" className="ui-btn" onClick={onClose} disabled={restoring}>
            取消
          </button>
          <button
            type="button"
            className="ui-btn ui-btn--danger"
            onClick={() => void doRestore()}
            disabled={!canRestore}
            title={changed === 0 ? "所选备份与当前文件内容一致，无需恢复" : undefined}
          >
            {restoring ? "恢复中…" : changed > 0 ? `确认恢复（${changed} 条变更）` : "确认恢复"}
          </button>
        </>
      }
    >
      {/* 备份元信息：来源文件可能很长，展示尾部并保留完整路径 tooltip */}
      <dl className="bk-meta">
        <dt>来源文件</dt>
        <dd>
          <code className="bk-meta__path" title={backup.sourcePath}>
            {baseName(backup.sourcePath)}
          </code>
          <span className="bk-muted bk-meta__full" title={backup.sourcePath}>
            {backup.sourcePath}
          </span>
        </dd>
        <dt>备份文件</dt>
        <dd>
          <code title={backup.path}>{elideMiddle(baseName(backup.path), 52, 0.62)}</code>
        </dd>
        <dt>备份时间</dt>
        <dd>
          {formatTime(backup.createdAt)} <span className="bk-muted">· {reasonLabel(backup.reason)}</span>
          <span className="bk-muted"> · {formatSize(backup.size)}</span>
        </dd>
      </dl>

      {phase === "dryRun" ? (
        <div className="bk-center">
          <Spinner label="正在比较当前文件与所选备份（dry-run，只读）…" />
        </div>
      ) : null}

      {phase === "error" && error ? (
        <ErrorAlert
          error={error}
          onRetry={() => void runDryRun()}
          retryLabel="重新比较"
          extra={
            <button type="button" className="ui-btn ui-btn--sm" onClick={onClose}>
              取消恢复
            </button>
          }
        />
      ) : null}

      {restoring ? (
        <div className="bk-banner bk-banner--warn" role="status">
          <Spinner label="正在写入备份内容（恢复前会自动备份当前版本）…" />
        </div>
      ) : null}

      {/* 步骤 2：差异摘要 + 明细 */}
      {phase === "ready" && diff ? (
        <>
          <div className="bk-chips" role="group" aria-label="差异摘要">
            <span className="bk-chip bk-chip--added">新增 {diff.summary.added}</span>
            <span className="bk-chip bk-chip--removed">删除 {diff.summary.removed}</span>
            <span className="bk-chip bk-chip--modified">修改 {diff.summary.modified}</span>
            <span className="bk-chip bk-chip--unchanged">未变 {diff.summary.unchanged}</span>
            <span className="bk-chips__total">
              合计变更 <strong>{changed}</strong> 条
            </span>
          </div>

          {changed === 0 ? (
            <p className="bk-note">
              所选备份与当前文件内容一致，<strong>没有需要恢复的变更</strong>，因此不能执行恢复。
            </p>
          ) : null}

          <div className="bk-diff">
            <DiffGroup
              kind="added"
              title="新增（恢复后会出现的词条）"
              count={diff.summary.added}
              lines={diff.added}
            />
            <DiffGroup
              kind="removed"
              title="删除（恢复后会消失的词条）"
              count={diff.summary.removed}
              lines={diff.removed}
            />
            {diff.summary.modified > 0 ? (
              <p className="bk-note bk-muted">
                修改 {diff.summary.modified} 条：按 IPC 契约 <code>FileDiff</code> 只携带新增/删除明细，
                修改仅计数（通常为权重变化），不列出行内容。
              </p>
            ) : null}
          </div>

          <div className="bk-banner bk-banner--warn">
            <strong>恢复会覆盖当前文件</strong>
            <p>
              后端会在写入前先把当前版本备份一份（原因「恢复前」），所以恢复本身仍可回滚；
              但恢复后需要部署，输入法候选才会更新。
            </p>
          </div>
        </>
      ) : null}

      {phase === "ready" && error ? (
        <ErrorAlert
          error={error}
          onRetry={() => void doRestore()}
          retryLabel="重试恢复"
          extra={
            <button type="button" className="ui-btn ui-btn--sm" onClick={() => setError(null)}>
              关闭
            </button>
          }
        />
      ) : null}
    </Modal>
  );
}

/** 单组差异明细（默认折叠；最多展示 20 条，其余只提示总数） */
function DiffGroup({
  kind,
  title,
  count,
  lines,
}: {
  kind: "added" | "removed";
  title: string;
  count: number;
  lines: DiffLine[];
}) {
  if (count === 0) return null;
  const shown = lines.slice(0, DETAIL_LIMIT);
  return (
    <details className={`bk-diff__group bk-diff__group--${kind}`}>
      <summary>
        {title}：<strong>{count}</strong> 条
        {lines.length > DETAIL_LIMIT ? `（明细显示前 ${DETAIL_LIMIT} 条）` : ""}
      </summary>
      {shown.length === 0 ? (
        <p className="bk-note bk-muted">
          后端未返回该组明细（<code>FileDiff</code> 中对应数组为空）。
        </p>
      ) : (
        <ul className="bk-diff__list">
          {shown.map((line, index) => (
            <li key={`${line.text}\u0000${line.code}\u0000${index}`} className="bk-diff__line">
              <code>{line.text}</code>
              <span className="bk-diff__sep">/</span>
              <code>{line.code}</code>
              {line.weight != null ? <span className="bk-muted">权重 {line.weight}</span> : null}
            </li>
          ))}
        </ul>
      )}
      {lines.length > DETAIL_LIMIT ? (
        <p className="bk-note bk-muted">另有 {lines.length - DETAIL_LIMIT} 条未显示。</p>
      ) : null}
    </details>
  );
}

export default RestoreDialog;