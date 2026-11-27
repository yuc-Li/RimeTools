/**
 * 外部修改冲突确认（`docs/ipc.md` §5.1 / §6.2；数据安全红线）。
 *
 * 后端返回 `externalModified` 表示「磁盘文件在本次编辑期间被其他程序改过」。
 * 此时**绝不能自动重试覆盖**：必须由用户显式选择「重新加载」或「取消」。
 * 本弹窗不允许点遮罩关闭，避免误触导致状态不明。
 */
import type { AppError } from "../../ipc/types";
import { Badge } from "../../components/Feedback";
import { Modal } from "../../components/Modal";
import type { DraftSummary } from "./draft";

export interface ExternalModifiedDialogProps {
  error: AppError;
  pending: DraftSummary;
  fileLabel: string;
  onReload: () => void;
  onCancel: () => void;
}

export function ExternalModifiedDialog({
  error,
  pending,
  fileLabel,
  onReload,
  onCancel,
}: ExternalModifiedDialogProps) {
  return (
    <Modal
      open
      tone="danger"
      title="⚠ 文件已被外部修改，保存已取消"
      width={640}
      dismissOnBackdrop={false}
      onClose={onCancel}
      footer={
        <>
          <button type="button" className="ui-btn" onClick={onCancel}>
            取消（保留我的修改，不保存）
          </button>
          <button type="button" className="ui-btn ui-btn--primary" onClick={onReload}>
            重新加载（丢弃未保存修改）
          </button>
        </>
      }
    >
      <div className="wb-banner wb-banner--danger" role="alert">
        <strong>磁盘上的 {fileLabel} 已在本次编辑之外被改动。</strong>
        <p>
          为保证不丢失你或外部程序的改动，本次保存<strong>没有写入任何内容</strong>，
          工具也<strong>不会自动重试覆盖</strong>。请选择重新加载磁盘内容，或取消后先备份再处理。
        </p>
      </div>

      <section>
        <h3 className="wb-section-title">本次被拒绝的改动</h3>
        <ul className="wb-diff-list">
          <li>
            <Badge tone="accent">新增</Badge> {pending.adds} 条
          </li>
          <li>
            <Badge tone="warn">修改</Badge> {pending.updates} 条
          </li>
          <li>
            <Badge tone="danger">删除</Badge> {pending.deletes} 条
          </li>
        </ul>
        <p className="wb-note wb-note--muted">
          「重新加载」会丢弃上面这些未保存的修改，并以磁盘现状为准重新读取；「取消」会保留你的修改，
          但下次保存仍会再次被指纹检测拦下。
        </p>
      </section>

      <section>
        <h3 className="wb-section-title">指纹差异</h3>
        <p className="wb-note">
          工具打开文件时记录的指纹与当前磁盘不一致（mtime / size / sha256 任一变化即视为外部修改）。
        </p>
        {error.detail ? <pre className="wb-pre">{error.detail}</pre> : null}
        {error.hint ? <p className="wb-note">💡 {error.hint}</p> : null}
      </section>
    </Modal>
  );
}

export default ExternalModifiedDialog;