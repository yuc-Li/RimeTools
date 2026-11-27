/**
 * 导入 / 导出弹窗（M2）。
 *
 * 设计要点：
 * - 导入**先预览再写入**：解析 + 冲突比对在弹窗里展示，用户确认后才真正写文件；
 * - 导入复用标准保存流程，因此同样有写前备份、校验与外部修改保护；
 * - 导出可「保存到文件」或「复制内容」；浏览器 mock 模式下只能复制（无文件对话框）；
 * - 文件选择用原生 `<input type="file">`，Tauri 与浏览器都能用，不依赖额外插件。
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { Modal } from "../../components/Modal";
import { toAppError } from "../../ipc/client";
import type {
  ExportResult,
  FileId,
  ImportPlan,
  ImportPolicy,
  TransferFormat,
} from "../../ipc/types";
import { ipc, isMockMode } from "./api";

export interface TransferDialogProps {
  mode: "import" | "export";
  fileId: FileId;
  fileName: string;
  onClose: () => void;
  /** 导入成功后调用（工作台据此重新加载文件） */
  onImported: () => void;
}

const FORMAT_LABEL: Record<TransferFormat, string> = {
  txt: "纯文本（制表符分隔）",
  csv: "CSV（表格软件可编辑）",
  dictYaml: "Rime 词典 YAML",
};

const POLICY_LABEL: Record<ImportPolicy, string> = {
  skipExisting: "跳过已存在的词条",
  overwrite: "用导入的权重覆盖",
  keepHigherWeight: "仅当导入权重更高时覆盖",
};

export function TransferDialog(props: TransferDialogProps) {
  return props.mode === "import" ? <ImportPanel {...props} /> : <ExportPanel {...props} />;
}

// ───────────────────────── 导出 ─────────────────────────

function ExportPanel({ fileId, fileName, onClose }: TransferDialogProps) {
  const [format, setFormat] = useState<TransferFormat>("txt");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<ExportResult | null>(null);
  const [copied, setCopied] = useState(false);

  const exportedRef = useRef<ExportResult | null>(null);

  const doExport = useCallback(
    async (toFile: boolean) => {
      setBusy(true);
      setError(null);
      setCopied(false);
      try {
        let path: string | null = null;
        if (toFile) {
          path = await ipc.pickExportPath(`${fileName}.${format === "dictYaml" ? "dict.yaml" : format}`);
          if (!path) {
            setBusy(false);
            return; // 用户取消
          }
        }
        const result = await ipc.exportDict({ fileId, format, path });
        exportedRef.current = result;
        setDone(result);
        if (!path && result.content) {
          try {
            await navigator.clipboard.writeText(result.content);
            setCopied(true);
          } catch {
            // 剪贴板不可用时仍展示内容已生成
          }
        }
      } catch (e) {
        setError(toAppError(e).message);
      } finally {
        setBusy(false);
      }
    },
    [fileId, fileName, format],
  );

  return (
    <Modal
      open
      title="导出词库"
      width={560}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="ui-btn" onClick={onClose}>
            关闭
          </button>
          <button
            type="button"
            className="ui-btn"
            disabled={busy}
            onClick={() => void doExport(false)}
            title={isMockMode() ? "浏览器预览模式下不可用" : "生成内容并复制到剪贴板"}
          >
            复制内容
          </button>
          <button
            type="button"
            className="ui-btn ui-btn--primary"
            disabled={busy || isMockMode()}
            onClick={() => void doExport(true)}
            title={isMockMode() ? "浏览器预览模式下不可用，请用「复制内容」" : "选择保存位置"}
          >
            保存到文件…
          </button>
        </>
      }
    >
      <div className="wb-transfer">
        <label className="wb-transfer__row">
          <span className="wb-transfer__label">格式</span>
          <select
            className="lk-select"
            value={format}
            onChange={(e) => {
              setFormat(e.target.value as TransferFormat);
              setDone(null);
            }}
          >
            {(Object.keys(FORMAT_LABEL) as TransferFormat[]).map((f) => (
              <option key={f} value={f}>
                {FORMAT_LABEL[f]}
              </option>
            ))}
          </select>
        </label>

        <p className="wb-transfer__note">
          导出的是当前所见的全部词条（含已暂存但未保存的改动）。文件格式遵循 Rime 约定，
          可直接作为 `*.dict.yaml` 或自定义短语使用。
        </p>

        {error ? <div className="ui-alert">{error}</div> : null}

        {done ? (
          <div className="ui-alert ui-alert--compact">
            {done.path ? (
              <>已导出 {done.entryCount} 条到 <code>{done.path}</code></>
            ) : (
              <>
                已生成 {done.entryCount} 条内容（{Math.ceil(done.bytes / 1024)} KB）
                {copied ? "，已复制到剪贴板" : "，可手动复制"}
              </>
            )}
            {!done.path && done.content ? (
              <details className="wb-transfer__preview">
                <summary>预览前 50 行</summary>
                <pre>{done.content.split("\n").slice(0, 50).join("\n")}</pre>
              </details>
            ) : null}
          </div>
        ) : null}
      </div>
    </Modal>
  );
}

// ───────────────────────── 导入 ─────────────────────────

function ImportPanel({ fileId, onClose, onImported }: TransferDialogProps) {
  const [policy, setPolicy] = useState<ImportPolicy>("skipExisting");
  const [path, setPath] = useState<string | null>(null);
  const [content, setContent] = useState<string | null>(null);
  const [fileName, setFileName] = useState<string | null>(null);
  const [plan, setPlan] = useState<ImportPlan | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const preview = useCallback(
    async (next: { path?: string | null; content?: string | null }, p: ImportPolicy) => {
      setBusy(true);
      setError(null);
      try {
        setPlan(
          await ipc.importPreview({
            fileId,
            path: next.path ?? null,
            content: next.content ?? null,
            policy: p,
          }),
        );
      } catch (e) {
        setPlan(null);
        setError(toAppError(e).message);
      } finally {
        setBusy(false);
      }
    },
    [fileId],
  );

  // 切换策略时重新比对（不重新读文件）
  useEffect(() => {
    if (!path && !content) return;
    void preview({ path, content }, policy);
    // 只依赖 policy：文件已在选择时载入
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [policy]);

  const onPick = useCallback(
    async (event: React.ChangeEvent<HTMLInputElement>) => {
      const file = event.target.files?.[0];
      if (!file) return;
      setFileName(file.name);
      setPath(null);
      const text = await file.text();
      setContent(text);
      await preview({ content: text }, policy);
    },
    [policy, preview],
  );

  const apply = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await ipc.importApply({ fileId, path, content, policy });
      onImported();
      onClose();
    } catch (e) {
      setError(toAppError(e).message);
    } finally {
      setBusy(false);
    }
  }, [content, fileId, onClose, onImported, path, policy]);

  const writeCount = plan ? plan.added + plan.updated : 0;

  return (
    <Modal
      open
      title="导入词库"
      width={620}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="ui-btn" onClick={onClose}>
            取消
          </button>
          <button
            type="button"
            className="ui-btn ui-btn--primary"
            disabled={busy || writeCount === 0}
            onClick={() => void apply()}
          >
            {writeCount > 0 ? `导入 ${writeCount} 条` : "导入"}
          </button>
        </>
      }
    >
      <div className="wb-transfer">
        <label className="wb-transfer__row">
          <span className="wb-transfer__label">选择文件</span>
          <input
            type="file"
            accept=".txt,.csv,.yaml,.yml"
            onChange={(e) => void onPick(e)}
            className="wb-transfer__file"
          />
        </label>

        <label className="wb-transfer__row">
          <span className="wb-transfer__label">冲突处理</span>
          <select
            className="lk-select"
            value={policy}
            onChange={(e) => setPolicy(e.target.value as ImportPolicy)}
          >
            {(Object.keys(POLICY_LABEL) as ImportPolicy[]).map((p) => (
              <option key={p} value={p}>
                {POLICY_LABEL[p]}
              </option>
            ))}
          </select>
        </label>

        <p className="wb-transfer__note">
          支持 txt（制表符分隔）、csv 与 Rime 词典 YAML。导入会先比对再写入：
          写入前自动备份，并校验编码字符集与长度。
        </p>

        {error ? <div className="ui-alert">{error}</div> : null}

        {plan ? (
          <>
            <div className="wb-transfer__stats">
              <Stat label="文件中" value={plan.total} />
              <Stat label="新增" value={plan.added} tone="ok" />
              <Stat label="更新" value={plan.updated} tone="warn" />
              <Stat label="跳过" value={plan.skipped} />
              <Stat label="未变化" value={plan.unchanged} />
              {plan.duplicates > 0 ? <Stat label="自身重复" value={plan.duplicates} /> : null}
            </div>

            {plan.weightIgnored ? (
              <p className="wb-transfer__warn">
                目标文件是两列格式（无权重列），导入的权重会被忽略。
              </p>
            ) : null}

            {plan.invalidCodes.length > 0 ? (
              <details className="wb-transfer__preview" open>
                <summary>编码不合法的 {plan.invalidCodes.length} 条（保存时会被拦下）</summary>
                <ul>
                  {plan.invalidCodes.slice(0, 10).map((i) => (
                    <li key={`${i.lineNo}-${i.raw}`}>
                      第 {i.lineNo} 行：{i.message} —— <code>{i.raw}</code>
                    </li>
                  ))}
                </ul>
              </details>
            ) : null}

            {plan.issues.length > 0 ? (
              <details className="wb-transfer__preview">
                <summary>无法解析的 {plan.skippedLines} 行</summary>
                <ul>
                  {plan.issues.slice(0, 10).map((i) => (
                    <li key={`${i.lineNo}-${i.raw}`}>
                      第 {i.lineNo} 行：{i.message} —— <code>{i.raw}</code>
                    </li>
                  ))}
                </ul>
              </details>
            ) : null}

            {plan.conflicts.length > 0 ? (
              <details className="wb-transfer__preview">
                <summary>冲突明细（共 {plan.conflicts.length} 条）</summary>
                <ul>
                  {plan.conflicts.slice(0, 20).map((c) => (
                    <li key={`${c.text}-${c.code}`}>
                      <code>{c.text}</code> / <code>{c.code}</code> · 现有权重{" "}
                      {c.existingWeight ?? "—"} → 导入 {c.incomingWeight ?? "—"} ·{" "}
                      {c.action === "updated"
                        ? "更新"
                        : c.action === "skipped"
                          ? "跳过"
                          : c.action === "added"
                            ? "新增"
                            : "不变"}
                    </li>
                  ))}
                </ul>
              </details>
            ) : null}
          </>
        ) : (
          <p className="wb-transfer__note">
            {busy ? "正在读取并比对…" : "选择文件后会先显示比对结果，不会立即写入。"}
            {fileName ? `（当前：${fileName}）` : ""}
          </p>
        )}
      </div>
    </Modal>
  );
}

function Stat({
  label,
  value,
  tone,
}: {
  label: string;
  value: number;
  tone?: "ok" | "warn";
}) {
  return (
    <span className={`wb-transfer__stat ${tone ? `wb-transfer__stat--${tone}` : ""}`}>
      <span className="wb-transfer__stat-label">{label}</span>
      <span className="wb-transfer__stat-value">{value}</span>
    </span>
  );
}

export default TransferDialog;