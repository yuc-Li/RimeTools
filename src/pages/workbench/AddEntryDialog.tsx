/**
 * 新增词条弹窗：文本 → `suggest_codes` 编码建议 → 多选入库（PRD FR-3.23、FR-3.22）。
 *
 * 关键产品约束：
 * 1. 建议只是**默认填充**，永远可编辑；建议接口失败也不能挡住手动填码（降级为手填）；
 * 2. `blocksLearning` 必须显眼：向 custom_phrase 写入「完整拼音/双拼码」会让该词不参与
 *    自学习造词，这里不但逐条打徽标，提交时还要用户再确认一次；
 * 3. 支持一次多编码入库（同一文本多条）；
 * 4. 无权重列的文件（xhup 用户码表两列格式）权重输入禁用并说明原因。
 */
import { useEffect, useRef, useState } from "react";
import { toAppError } from "../../ipc/client";
import type {
  AppError,
  CodeSuggestion,
  FileRole,
  ManagedFileRef,
  SchemeInfo,
  SuggestionWarning,
} from "../../ipc/types";
import { Badge, Spinner, type BadgeTone } from "../../components/Feedback";
import { Modal } from "../../components/Modal";
import { ipc } from "./api";
import type { DraftAdd } from "./draft";
import { useDebouncedValue } from "./useDebouncedValue";

const WARNING_META: Record<SuggestionWarning, { label: string; tone: BadgeTone; help: string }> = {
  blocksLearning: {
    label: "阻断自学习",
    tone: "danger",
    help: "完整拼音/双拼码写入 custom_phrase 后，该词将不参与自学习造词（FR-3.22）。建议改用简拼，或把完整拼音词放进 mydict / 方案 pin_cand_filter。",
  },
  exceedsMaxCodeLength: {
    label: "超出码长",
    tone: "warn",
    help: "编码长度超过方案的 max_code_length，Rime 可能无法正常匹配。",
  },
  unsupportedCharset: {
    label: "字符不受支持",
    tone: "warn",
    help: "编码含方案 speller/alphabet 之外的字符，保存校验会拒绝。",
  },
  polyphone: {
    label: "多音字",
    tone: "info",
    help: "该词含多音字，请确认读音后再选择编码。",
  },
};

const KIND_ORDER: Record<CodeSuggestion["kind"], number> = {
  shapeWord: 0,
  shapeChar: 0,
  fullPinyin: 1,
  doublePinyin: 2,
  abbrev: 3,
  english: 4,
  history: 5,
};

function suggestionId(s: CodeSuggestion): string {
  return `${s.kind}:${s.code}`;
}

export interface AddEntryDialogProps {
  scheme: SchemeInfo | null;
  file: ManagedFileRef | null;
  hasWeightColumn: boolean;
  role: FileRole | null;
  groups: string[];
  onClose: () => void;
  onAdd: (rows: Array<Omit<DraftAdd, "localId">>) => void;
}

export function AddEntryDialog({
  scheme,
  file,
  hasWeightColumn,
  role,
  groups,
  onClose,
  onAdd,
}: AddEntryDialogProps) {
  const [text, setText] = useState("");
  const debouncedText = useDebouncedValue(text, 250);
  const [suggestions, setSuggestions] = useState<CodeSuggestion[]>([]);
  const [suggestState, setSuggestState] = useState<"idle" | "loading" | "ok" | "error">("idle");
  const [suggestError, setSuggestError] = useState<AppError | null>(null);
  const [selected, setSelected] = useState<Record<string, true>>({});
  const [manualCode, setManualCode] = useState("");
  const [weight, setWeight] = useState("");
  const [group, setGroup] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [confirmBlocker, setConfirmBlocker] = useState(false);
  const seqRef = useRef(0);

  // 文本变化 → 防抖后请求编码建议（`docs/ipc.md` §3.3 suggest_codes）
  useEffect(() => {
    const trimmed = debouncedText.trim();
    seqRef.current += 1;
    const seq = seqRef.current;
    setSelected({});
    setConfirmBlocker(false);

    if (!trimmed) {
      setSuggestions([]);
      setSuggestState("idle");
      setSuggestError(null);
      return;
    }

    setSuggestState("loading");
    setSuggestError(null);
    ipc
      .suggestCodes({ text: trimmed, schemaId: scheme?.schemaId, role: role ?? undefined })
      .then((list) => {
        if (seq !== seqRef.current) return;
        setSuggestions([...list].sort((a, b) => KIND_ORDER[a.kind] - KIND_ORDER[b.kind]));
        setSuggestState("ok");
      })
      .catch((e: unknown) => {
        if (seq !== seqRef.current) return;
        setSuggestions([]);
        setSuggestError(toAppError(e));
        setSuggestState("error");
      });
  }, [debouncedText, scheme?.schemaId, role]);

  const selectedList = suggestions.filter((s) => selected[suggestionId(s)]);
  const blocking = selectedList.filter((s) => s.warnings.includes("blocksLearning"));

  function toggle(s: CodeSuggestion) {
    const id = suggestionId(s);
    setSelected((prev) => {
      const next = { ...prev };
      if (next[id]) delete next[id];
      else next[id] = true;
      return next;
    });
    setConfirmBlocker(false);
  }

  function buildRows(): { rows: Array<Omit<DraftAdd, "localId">>; error: string | null } {
    const trimmed = text.trim();
    if (!trimmed) return { rows: [], error: "文本不能为空" };

    let parsedWeight: number | null = null;
    if (hasWeightColumn && weight.trim() !== "") {
      const parsed = Number(weight.trim());
      if (!Number.isInteger(parsed) || parsed < 0) {
        return { rows: [], error: "权重必须是非负整数（留空表示不写权重列）" };
      }
      parsedWeight = parsed;
    }

    const byCode = new Map<string, Omit<DraftAdd, "localId">>();
    for (const suggestion of selectedList) {
      byCode.set(suggestion.code, {
        text: trimmed,
        code: suggestion.code,
        weight: parsedWeight,
        group: group.trim() === "" ? null : group.trim(),
      });
    }
    for (const raw of manualCode.split(/[\s,，;；]+/)) {
      const code = raw.trim();
      if (!code) continue;
      byCode.set(code, {
        text: trimmed,
        code,
        weight: parsedWeight,
        group: group.trim() === "" ? null : group.trim(),
      });
    }

    if (byCode.size === 0) {
      return { rows: [], error: "请勾选编码建议，或手动填写至少一个编码" };
    }
    return { rows: [...byCode.values()], error: null };
  }

  const preview = buildRows();

  function submit() {
    if (preview.error) {
      setError(preview.error);
      return;
    }
    if (blocking.length > 0 && !confirmBlocker) {
      // 阻断自学习 → 需要用户再确认一次（FR-3.22：提示后用户确认仍可保存）
      setConfirmBlocker(true);
      setError(null);
      return;
    }
    onAdd(preview.rows);
  }

  return (
    <Modal
      open
      title="新增词条"
      onClose={onClose}
      width={760}
      footer={
        <>
          <span className="wb-dialog-hint">
            将添加 {preview.rows.length} 条到暂存（不会立即写文件）
          </span>
          <button type="button" className="ui-btn" onClick={onClose}>
            取消
          </button>
          <button
            type="button"
            className={`ui-btn ${confirmBlocker ? "ui-btn--danger" : "ui-btn--primary"}`}
            onClick={submit}
            disabled={preview.rows.length === 0}
          >
            {confirmBlocker
              ? `我了解会阻断自学习，仍然添加 ${preview.rows.length} 条`
              : `添加到暂存（${preview.rows.length} 条）`}
          </button>
        </>
      }
    >
      {role === "customPhrase" ? (
        <div className="wb-banner wb-banner--warn">
          <strong>注意：custom_phrase 的造词机制</strong>
          <p>
            向本文件写入<strong>完整拼音 / 双拼码</strong>的词，会占据最高权重但
            <strong>不参与自学习造词</strong>（文件头注释明示，FR-3.22）。
            建议只用简拼/首字母码；需要完整拼音词请放进 mydict（个人词典）或用方案里的
            <code>pin_cand_filter</code>。
          </p>
        </div>
      ) : null}

      <form
        className="wb-form"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <label className="wb-field">
          <span className="wb-field__label">
            文本
            <span className="wb-field__hint">输入后自动拉取编码建议（{scheme?.name ?? "当前方案"}）</span>
          </span>
          <input
            className="wb-input"
            value={text}
            onChange={(event) => {
              setText(event.target.value);
              setError(null);
            }}
            placeholder="例如：美团 / 报关单 / 编码建议"
            spellCheck={false}
            autoComplete="off"
          />
        </label>

        <section className="wb-suggest" aria-label="编码建议">
          <header className="wb-suggest__head">
            <span>编码建议</span>
            {suggestState === "loading" ? <Spinner label="正在生成…" /> : null}
            {suggestState === "ok" && suggestions.length > 0 ? (
              <span className="wb-field__hint">
                勾选需要的编码（可多选）；建议只是默认填充，仍可手动改
              </span>
            ) : null}
          </header>

          {suggestState === "idle" ? (
            <p className="wb-note wb-note--muted">先输入文本，再选择编码。</p>
          ) : null}

          {suggestState === "error" && suggestError ? (
            <div className="wb-inline-error">
              <span>编码建议获取失败：{suggestError.message}</span>
              <span className="wb-field__hint">不影响手动填写编码。</span>
            </div>
          ) : null}

          {suggestState === "ok" && suggestions.length === 0 ? (
            <p className="wb-note wb-note--muted">
              没有可用的编码建议（mock 只内置常用字读音；真实后端由 rime-suggest 生成）。
              请在下方手动填写编码——建议只是默认填充，永远可编辑。
            </p>
          ) : null}

          {suggestions.length > 0 ? (
            <ul className="wb-suggest__list">
              {suggestions.map((s) => {
                const id = suggestionId(s);
                const checked = selected[id] === true;
                return (
                  <li key={id}>
                    <label className={`wb-suggest__item${checked ? " is-checked" : ""}`}>
                      <input type="checkbox" checked={checked} onChange={() => toggle(s)} />
                      <code className="wb-suggest__code">{s.code}</code>
                      <Badge tone="muted">{s.label}</Badge>
                      {s.reading ? (
                        <span className="wb-suggest__reading">{s.reading}</span>
                      ) : null}
                      <span className="wb-suggest__confidence">
                        {s.confidence === "high" ? "高置信" : s.confidence === "medium" ? "中置信" : "低置信"}
                      </span>
                      <span className="wb-suggest__warnings">
                        {s.warnings.map((warning) => {
                          const meta = WARNING_META[warning];
                          return (
                            <Badge key={warning} tone={meta.tone} title={meta.help}>
                              {meta.label}
                            </Badge>
                          );
                        })}
                      </span>
                    </label>
                  </li>
                );
              })}
            </ul>
          ) : null}
        </section>

        <div className="wb-grid-3">
          <label className="wb-field">
            <span className="wb-field__label">手动编码</span>
            <input
              className="wb-input wb-input--mono"
              value={manualCode}
              onChange={(event) => {
                setManualCode(event.target.value);
                setError(null);
              }}
              placeholder="多码用空格分隔"
              spellCheck={false}
              autoComplete="off"
            />
          </label>

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
              placeholder={hasWeightColumn ? "可选，留空不写" : "—"}
            />
            <span className="wb-field__hint">
              {hasWeightColumn
                ? "custom_phrase 权重 = 同编码内的排序（越大越靠前）"
                : "该文件为两列格式，没有权重列，无法写入权重"}
            </span>
          </label>

          <label className="wb-field">
            <span className="wb-field__label">分组</span>
            <input
              className="wb-input"
              value={group}
              onChange={(event) => setGroup(event.target.value)}
              list="wb-add-groups"
              placeholder="可选"
            />
            <datalist id="wb-add-groups">
              {groups.map((name) => (
                <option key={name} value={name} />
              ))}
            </datalist>
            <span className="wb-field__hint">分组只是本工具元数据，不写进 Rime 文件</span>
          </label>
        </div>

        {confirmBlocker ? (
          <div className="wb-banner wb-banner--danger" role="alert">
            <strong>确认要添加会阻断自学习的编码？</strong>
            <p>
              已选 {blocking.length} 个完整拼音/双拼码：{blocking.map((s) => s.code).join("、")}。
              这些词写入 {file?.path.split("/").pop() ?? "custom_phrase"} 后不会参与自学习造词。
              确认后仍会添加，但建议优先选「简拼」或改用 mydict。
            </p>
          </div>
        ) : null}

        {error ? <p className="wb-form__error">{error}</p> : null}
        {!hasWeightColumn ? (
          <p className="wb-note wb-note--muted">
            说明：{file?.path.split("/").pop() ?? "该文件"} 是两列码表（文本 + 编码），
            权重输入已禁用。
          </p>
        ) : null}
      </form>
    </Modal>
  );
}

export default AddEntryDialog;