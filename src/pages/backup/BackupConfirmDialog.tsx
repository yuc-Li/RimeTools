/**
 * 删除备份的二次确认（PRD FR-6.3 / 任务要求「多选 + 二次确认，显示将删除几份」）。
 *
 * 删除只作用于 `*.bak` 文件本身，**不改动**任何词库文件；但删除不可撤销，
 * 所以这里禁止点遮罩关闭，必须显式选「取消 / 删除」。
 */
import type { BackupRef } from "../../ipc/types";
import { Modal } from "../../components/Modal";
import { baseName, elideMiddle, formatSize, formatTime, reasonLabel } from "./format";
import "./backup.css";

export interface BackupConfirmDialogProps {
  backups: BackupRef[];
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}

const NAME_LIMIT = 5;

export function BackupConfirmDialog({
  backups,
  busy,
  onCancel,
  onConfirm,
}: BackupConfirmDialogProps) {
  const count = backups.length;
  const shown = backups.slice(0, NAME_LIMIT);
  const totalSize = backups.reduce((sum, item) => sum + (Number.isFinite(item.size) ? item.size : 0), 0);

  return (
    <Modal
      open
      title="删除备份？"
      width={540}
      tone="danger"
      dismissOnBackdrop={false}
      onClose={() => {
        if (!busy) onCancel();
      }}
      footer={
        <>
          <span className="bk-dialog-hint">此操作不可撤销，但不会改动当前词库文件。</span>
          <button type="button" className="ui-btn" onClick={onCancel} disabled={busy}>
            取消
          </button>
          <button
            type="button"
            className="ui-btn ui-btn--danger"
            onClick={onConfirm}
            disabled={busy}
          >
            {busy ? "删除中…" : `删除 ${count} 份备份`}
          </button>
        </>
      }
    >
      <p className="bk-confirm-text">
        将永久删除选中的 <strong>{count} 份</strong>备份文件
        {totalSize > 0 ? <>（合计 {formatSize(totalSize)}）</> : null}：
      </p>
      <ul className="bk-confirm-list">
        {shown.map((backup) => (
          <li key={backup.id}>
            <code title={backup.path}>{elideMiddle(baseName(backup.path), 46)}</code>
            <span className="bk-muted">
              {formatTime(backup.createdAt)} · {reasonLabel(backup.reason)}
            </span>
          </li>
        ))}
      </ul>
      {count > NAME_LIMIT ? <p className="bk-note bk-muted">另有 {count - NAME_LIMIT} 份未列出。</p> : null}
      <p className="bk-note bk-muted">
        删除后无法再从这些备份恢复；当前词库文件与「保留份数」设置都不受影响。
      </p>
    </Modal>
  );
}

export default BackupConfirmDialog;