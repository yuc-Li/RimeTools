/** 通用二次确认弹窗（切换文件丢弃草稿、放弃修改、重新加载等不可逆操作前使用）。 */
import { Modal } from "../../components/Modal";

export interface ConfirmDialogProps {
  title: string;
  message: string;
  confirmLabel: string;
  cancelLabel?: string;
  danger?: boolean;
  onConfirm: () => void;
  onClose: () => void;
}

export function ConfirmDialog({
  title,
  message,
  confirmLabel,
  cancelLabel = "取消",
  danger,
  onConfirm,
  onClose,
}: ConfirmDialogProps) {
  return (
    <Modal
      open
      title={title}
      tone={danger ? "danger" : "default"}
      width={520}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="ui-btn" onClick={onClose}>
            {cancelLabel}
          </button>
          <button
            type="button"
            className={`ui-btn ${danger ? "ui-btn--danger" : "ui-btn--primary"}`}
            onClick={onConfirm}
          >
            {confirmLabel}
          </button>
        </>
      }
    >
      <p className="wb-confirm-text">{message}</p>
    </Modal>
  );
}

export default ConfirmDialog;