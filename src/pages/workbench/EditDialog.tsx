/**
 * 编辑既有词条 / 修改暂存新增行的弹窗（PRD FR-3.7、FR-3.9；契约 §6.1 草稿模型）。
 *
 * 产品红线（FR-3.23 / T0.4 报告 C5）：**编辑文本时绝不自动改写已有编码**。
 * 本弹窗只把用户显式输入的值回传，不调用任何建议接口、不做编码推导。
 */
import { useState } from "react";
import { Badge } from "../../components/Feedback";
import { Modal } from "../../components/Modal";
import type { Entry, FileRole } from "../../ipc/types";
import type { DraftAdd } from "./draft";

export type EditTarget =
  | { kind: "entry"; entry: Entry }
  | { kind: "add"; add: DraftAdd };

export interface EditPatch {
  text: string;
  code: string;
  weight: number | null;
  group: string | null;
}

export interface EditDialogProps {
  target: EditTarget;
  hasWeightColumn: boolean;
  role: FileRole | null;
  groups: string[];
  onClose: () => void;
  onApply: (target: EditTarget, patch: EditPatch) => void;
}

export function EditDialog({
  target,
  hasWeightColumn,
  role,
  groups,
  onClose,
  onApply,
}: EditDialogProps) {
  const baseEntry = target.kind === "entry" ? target.entry : null;
  const baseAdd = target.kind === "add" ? target.add : null;

  const [text, setText] = useState(baseEntry?.key.text ?? baseAdd?.text ?? "");
  const [code, setCode] = useState(baseEntry?.key.code ?? baseAdd?.code ?? "");
  const [weight, setWeight] = useState(
    baseEntry?.weight != null ? String(baseEntry.weight) : baseAdd?.weight != null ? String(baseAdd.weight) : "",
  );
  const [group, setGroup] = useState(baseEntry?.group ?? baseAdd?.group ?? "");
  const [error, setError] = useState<string | null>(null);

  const title = baseEntry ? "编辑词条" : "编辑新增词条（未保存）";

  function submit() {
    const nextText = text.trim();
    const nextCode = code.trim();
    if (!nextText) {
      setError("文本不能为空");
      return;
    }
    if (!nextCode) {
      setError("编码不能为空");
      return;
    }
    let nextWeight: number | null = null;
    if (hasWeightColumn && weight.trim() !== "") {
      const parsed = Number(weight.trim());
      if (!Number.isInteger(parsed) || parsed < 0) {
        setError("权重必须是非负整数（留空表示使用默认值）");
        return;
      }
      nextWeight = parsed;
    }
    onApply(target, {
      text: nextText,
      code: nextCode,
      weight: hasWeightColumn ? nextWeight : null,
      group: group.trim() === "" ? null : group.trim(),
    });
  }

  return (
    <Modal
      open
      title={title}
      onClose={onClose}
      width={620}
      footer={
        <>
          <span className="wb-dialog-hint">修改先暂存，点「保存全部」才写入文件</span>
          <button type="button" className="ui-btn" onClick={onClose}>
            取消
          </button>
          <button type="button" className="ui-btn ui-btn--primary" onClick={submit}>
            应用到暂存
          </button>
        </>
      }
    >
      <form
        className="wb-form"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <label className="wb-field">
          <span className="wb-field__label">文本</span>
          <input
            className="wb-input"
            value={text}
            onChange={(event) => setText(event.target.value)}
            spellCheck={false}
            autoComplete="off"
          />
        </label>

        <label className="wb-field">
          <span className="wb-field__label">编码</span>
          <input
            className="wb-input wb-input--mono"
            value={code}
            onChange={(event) => setCode(event.target.value)}
            spellCheck={false}
            autoComplete="off"
          />
        </label>

        <p className="wb-note">
          🔒 工具不会自动改写编码：修改文本后编码保持原样；如需变更请在此手动编辑。
        </p>

        <label className="wb-field">
          <span className="wb-field__label">
            权重
            {hasWeightColumn ? null : <Badge tone="muted">本文件无权重列</Badge>}
          </span>
          <input
            className="wb-input wb-input--mono"
            value={weight}
            onChange={(event) => setWeight(event.target.value)}
            disabled={!hasWeightColumn}
            inputMode="numeric"
            placeholder={hasWeightColumn ? "留空 = 不写权重列" : "—"}
          />
          <span className="wb-field__hint">
            {hasWeightColumn
              ? role === "customPhrase"
                ? "custom_phrase 的权重表示「同编码内的排序」，数值越大越靠前。"
                : "权重是该文件内的排序依据。"
              : "该文件是两列格式（文本 + 编码），没有权重列，因此权重不可编辑。"}
          </span>
        </label>

        <label className="wb-field">
          <span className="wb-field__label">分组</span>
          <input
            className="wb-input"
            value={group}
            onChange={(event) => setGroup(event.target.value)}
            list="wb-groups"
            placeholder="留空 = 未分组（分组只是本工具的元数据，不写进 Rime 文件）"
          />
          <datalist id="wb-groups">
            {groups.map((name) => (
              <option key={name} value={name} />
            ))}
          </datalist>
        </label>

        {baseEntry ? (
          <p className="wb-note wb-note--muted">
            源文件第 {baseEntry.lineNo ?? "?"} 行 · 来源：
            {baseEntry.origin === "file" ? "文件" : baseEntry.origin === "userAdded" ? "新增" : "已编辑"}
          </p>
        ) : null}

        {error ? <p className="wb-form__error">{error}</p> : null}
      </form>
    </Modal>
  );
}

export default EditDialog;