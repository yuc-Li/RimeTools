/**
 * 词库工作台（M1 / PRD FR-3.6–3.11、3.22–3.24）。
 *
 * MVP 主路径：**浏览 → 搜索/筛选/排序 → 编辑（暂存 dirty）→ 保存全部（原子写 + 写前备份）**。
 *
 * IPC 对应关系（`docs/ipc.md`）：
 * - §3.1 `env_list_schemes`：取方案与受管文件（经 `api.ts`）
 * - §3.2 `dict_open`：打开词库并记录指纹（保存必传）
 * - §3.2 `dict_query`：分页/搜索/排序（`useEntryWindow` 按可视区间取数）
 * - §3.2 `dict_validate` / `dict_save`：保存前校验 → 原子写；含外部修改检测
 * - §3.3 `suggest_codes`：新增词条的编码建议（`AddEntryDialog`）
 * - §6：草稿模型、指纹必传、不做乐观更新、事件全局订阅（`events.ts`）
 *
 * 产品约束：
 * - 编码建议只是默认填充，永远可编辑；编辑既有词条**绝不自动改写编码**；
 * - 向 custom_phrase 写完整拼音/双拼码会阻断自学习造词，必须显眼提示（FR-3.22）；
 * - 保存遇到 `externalModified` 必须弹差异确认让用户选「重新加载 / 取消」，**绝不自动重试覆盖**。
 *
 * App.tsx 由 lead 接线：本组件为默认导出，无必填 props。
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { toAppError } from "../../ipc/client";
import type {
  AppError,
  Entry,
  FileId,
  FileRole,
  Fingerprint,
  ManagedFileRef,
  SaveResult,
  SchemeInfo,
  SortField,
  SortSpec,
  ValidationIssue,
} from "../../ipc/types";
import { mockFillStress, mockSimulateExternalChange } from "../../ipc/mock";
import { Badge, EmptyState, ErrorAlert, Spinner } from "../../components/Feedback";
import { AddEntryDialog } from "./AddEntryDialog";
import { ConfirmDialog } from "./ConfirmDialog";
import { EditDialog, type EditPatch, type EditTarget } from "./EditDialog";
import { EntryTable } from "./EntryTable";
import { ExternalModifiedDialog } from "./ExternalModifiedDialog";
import { ipc, isMockMode } from "./api";
import {
  addDraftAdds,
  buildSaveRequest,
  draftSummary,
  emptyDraft,
  localPrecheck,
  patchDraftAdd,
  removeDraftAdd,
  stageEntryUpdate,
  toggleEntryDelete,
  type Draft,
  type DraftAdd,
} from "./draft";
import { emitExternalChange, onExternalChange } from "./events";
import { useDebouncedValue } from "./useDebouncedValue";
import { useEntryWindow } from "./useEntryWindow";
import "./workbench.css";

const DEFAULT_SORT: SortSpec = { field: "lineNo", desc: false };

type OpenState = "idle" | "opening" | "ready" | "error";

interface ConfirmRequest {
  title: string;
  message: string;
  confirmLabel: string;
  danger?: boolean;
  onConfirm: () => void;
}

function baseName(path: string): string {
  const parts = path.split(/[/\\]/);
  return parts[parts.length - 1] || path;
}

function roleLabel(role: FileRole): string {
  switch (role) {
    case "customPhrase":
      return "自定义短语";
    case "userDict":
      return "用户码表";
    case "userTopDict":
      return "置顶码表";
    case "personalDict":
      return "个人词典";
    case "readOnlyDict":
      return "系统词典（只读）";
    default:
      return role;
  }
}

function fileLabel(file: ManagedFileRef): string {
  return `${roleLabel(file.role)} · ${baseName(file.path)}`;
}

function fingerprintShort(fingerprint: Fingerprint): string {
  return `${fingerprint.sha256.slice(0, 8)}… (${fingerprint.size}B)`;
}

export default function Workbench() {
  // ── 环境（方案 / 受管文件） ──
  const [schemes, setSchemes] = useState<SchemeInfo[] | null>(null);
  const [bootState, setBootState] = useState<"loading" | "ready" | "error">("loading");
  const [bootError, setBootError] = useState<AppError | null>(null);
  const [schemeId, setSchemeId] = useState<string | null>(null);
  const [fileId, setFileId] = useState<FileId | null>(null);

  // ── 打开的词典（total + 指纹） ──
  const [openState, setOpenState] = useState<OpenState>("idle");
  const [openError, setOpenError] = useState<AppError | null>(null);
  const [fileMeta, setFileMeta] = useState<{ total: number; fingerprint: Fingerprint } | null>(null);
  const [openNonce, setOpenNonce] = useState(0);

  // ── 查询条件 ──
  const [keywordInput, setKeywordInput] = useState("");
  const keyword = useDebouncedValue(keywordInput, 200);
  const [group, setGroup] = useState<string | null>(null);
  const [sort, setSort] = useState<SortSpec>(DEFAULT_SORT);
  const [revision, setRevision] = useState(0);

  // ── 草稿与保存 ──
  const [draft, setDraft] = useState<Draft>(emptyDraft);
  const [deployAfter, setDeployAfter] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<AppError | null>(null);
  const [issues, setIssues] = useState<ValidationIssue[]>([]);
  const [lastSaved, setLastSaved] = useState<SaveResult | null>(null);
  const [conflict, setConflict] = useState<AppError | null>(null);
  const [externalNotice, setExternalNotice] = useState<Fingerprint | null>(null);

  // ── 弹窗与提示 ──
  const [editTarget, setEditTarget] = useState<EditTarget | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const mockMode = isMockMode();

  const currentScheme = useMemo(
    () => schemes?.find((scheme) => scheme.schemaId === schemeId) ?? null,
    [schemes, schemeId],
  );
  const managedFiles = currentScheme?.managedFiles ?? [];
  const currentFile = useMemo(
    () => schemes?.flatMap((scheme) => scheme.managedFiles).find((file) => file.id === fileId) ?? null,
    [schemes, fileId],
  );
  const currentRole: FileRole | null = currentFile?.role ?? null;
  const hasWeightColumn = currentFile?.columns.includes("weight") ?? false;
  const readOnly = currentRole === "readOnlyDict";
  const summary = draftSummary(draft);
  const dirtyCount = summary.total;

  // 只有 dict_open 完成后才允许 dict_query（真实后端要求先打开建索引）
  const queryFileId = openState === "ready" && fileMeta ? fileId : null;
  const entryWindow = useEntryWindow({ fileId: queryFileId, keyword, group, sort, revision });

  const groupNames = useMemo(
    () => entryWindow.groups.map((item) => item.name).filter((name) => name !== ""),
    [entryWindow.groups],
  );

  // ── 方案加载 ──
  const loadSchemes = useCallback(async () => {
    setBootState("loading");
    setBootError(null);
    try {
      const list = await ipc.listSchemes();
      setSchemes(list);
      setBootState("ready");
      const preferred =
        list.find((scheme) => scheme.isCurrent) ??
        list.find((scheme) => scheme.isActive) ??
        list[0] ??
        null;
      if (!preferred) {
        setSchemeId(null);
        setFileId(null);
        return;
      }
      setSchemeId(preferred.schemaId);
      const file =
        preferred.managedFiles.find((item) => item.role !== "readOnlyDict") ??
        preferred.managedFiles[0] ??
        null;
      setFileId(file?.id ?? null);
      setOpenState(file ? "opening" : "idle");
    } catch (error) {
      setBootError(toAppError(error));
      setBootState("error");
    }
  }, []);

  useEffect(() => {
    void loadSchemes();
  }, [loadSchemes]);

  // ── 打开词库（含重新加载：openNonce 变化） ──
  useEffect(() => {
    if (!fileId) {
      setFileMeta(null);
      setOpenState("idle");
      return;
    }
    let cancelled = false;
    setOpenState("opening");
    setOpenError(null);
    setFileMeta(null);
    ipc
      .openDict(fileId)
      .then((result) => {
        if (cancelled) return;
        setFileMeta({ total: result.total, fingerprint: result.fingerprint });
        setOpenState("ready");
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        setOpenError(toAppError(error));
        setOpenState("error");
      });
    return () => {
      cancelled = true;
    };
  }, [fileId, openNonce]);

  // ── 外部修改事件（§6.4：全局订阅） ──
  useEffect(
    () =>
      onExternalChange((event) => {
        if (event.fileId !== fileId) return;
        setExternalNotice(event.fingerprint);
      }),
    [fileId],
  );

  // ── 普通提示 5s 后自动消失 ──
  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(null), 5000);
    return () => clearTimeout(timer);
  }, [notice]);

  // ── 选择切换（切换文件/方案会丢弃草稿，必须先确认） ──
  const applySelection = useCallback((nextSchemeId: string | null, nextFileId: FileId | null) => {
    setSchemeId(nextSchemeId);
    setFileId(nextFileId);
    setKeywordInput("");
    setGroup(null);
    setSort(DEFAULT_SORT);
    setDraft(emptyDraft());
    setLastSaved(null);
    setSaveError(null);
    setIssues([]);
    setExternalNotice(null);
    setOpenError(null);
    setFileMeta(null);
    setOpenState(nextFileId ? "opening" : "idle");
  }, []);

  const guardedSelection = useCallback(
    (nextSchemeId: string | null, nextFileId: FileId | null) => {
      if (nextFileId === fileId) {
        applySelection(nextSchemeId, nextFileId);
        return;
      }
      if (dirtyCount > 0) {
        setConfirm({
          title: "切换文件？",
          message: `当前文件有 ${dirtyCount} 项未保存的修改，切换文件会丢弃这些修改（不会写入磁盘）。`,
          confirmLabel: "放弃修改并切换",
          danger: true,
          onConfirm: () => {
            setConfirm(null);
            applySelection(nextSchemeId, nextFileId);
          },
        });
        return;
      }
      applySelection(nextSchemeId, nextFileId);
    },
    [applySelection, dirtyCount, fileId],
  );

  const doReload = useCallback(() => {
    setDraft(emptyDraft());
    setLastSaved(null);
    setSaveError(null);
    setIssues([]);
    setExternalNotice(null);
    setFileMeta(null);
    setOpenState("opening");
    setOpenNonce((n) => n + 1);
    setRevision((n) => n + 1);
  }, []);

  const requestReload = useCallback(() => {
    if (dirtyCount > 0) {
      setConfirm({
        title: "重新加载文件？",
        message: `重新加载会丢弃 ${dirtyCount} 项未保存的修改，并以磁盘当前内容为准。`,
        confirmLabel: "放弃修改并重新加载",
        danger: true,
        onConfirm: () => {
          setConfirm(null);
          doReload();
        },
      });
      return;
    }
    doReload();
  }, [dirtyCount, doReload]);

  // ── 保存全部（§5.1 时序：validate → save；指纹必传） ──
  const saveAll = useCallback(async () => {
    if (!fileId || !fileMeta || dirtyCount === 0 || saving || readOnly) return;

    setSaveError(null);
    setLastSaved(null);

    // 1) 本地快速预检（只是提前反馈，最终以后端 dict_validate 为准）
    const pre = localPrecheck(draft, {
      alphabet: currentScheme?.alphabet,
      maxCodeLength: currentScheme?.maxCodeLength ?? null,
      hasWeightColumn,
    });
    if (pre.length > 0) {
      setIssues(pre);
      setSaveError({
        code: "validationFailed",
        message: `本地校验发现 ${pre.length} 项问题，已阻止保存（未写入文件）`,
        hint: "修正下方列出的问题后再保存。",
      });
      return;
    }
    setIssues([]);

    const request = buildSaveRequest({
      fileId,
      fingerprint: fileMeta.fingerprint,
      draft,
      deployAfter,
      hasWeightColumn,
    });

    setSaving(true);
    try {
      // 2) 后端保存前校验（会逐条给出行号）
      const validation = await ipc.validate(request);
      if (!validation.ok) {
        setIssues(validation.errors);
        setSaveError({
          code: "validationFailed",
          message: `保存校验未通过（${validation.errors.length} 项问题），未写入文件`,
          hint: "修正后可再次点「保存全部」。",
        });
        return;
      }

      // 3) 原子写 + 写前备份（指纹不一致时后端抛 externalModified）
      const result = await ipc.save(request);
      setLastSaved(result);
      setFileMeta((prev) => ({
        total: prev?.total ?? entryWindow.total,
        fingerprint: result.fingerprint,
      }));
      setDraft(emptyDraft());
      // 4) 不做乐观更新：重新从后端查询（§6.3）
      setRevision((n) => n + 1);
    } catch (error) {
      const appError = toAppError(error);
      if (appError.code === "externalModified") {
        // 数据安全红线：绝不自动重试覆盖，必须让用户选择
        setConflict(appError);
      } else {
        setSaveError(appError);
      }
    } finally {
      setSaving(false);
    }
  }, [
    currentScheme?.alphabet,
    currentScheme?.maxCodeLength,
    deployAfter,
    dirtyCount,
    draft,
    entryWindow.total,
    fileId,
    fileMeta,
    hasWeightColumn,
    readOnly,
    saving,
  ]);

  // Ctrl/Cmd+S 触发保存
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        void saveAll();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [saveAll]);

  // ── 草稿操作 ──
  const applyEdit = (target: EditTarget, patch: EditPatch) => {
    if (target.kind === "entry") {
      setDraft((prev) => stageEntryUpdate(prev, target.entry, patch));
    } else {
      setDraft((prev) => patchDraftAdd(prev, target.add.localId, patch));
    }
    setEditTarget(null);
  };

  const handleAdd = (rows: Array<Omit<DraftAdd, "localId">>) => {
    setDraft((prev) => addDraftAdds(prev, rows));
    setAddOpen(false);
    setNotice(`已添加 ${rows.length} 条到暂存（点「保存全部」才写入文件）`);
  };

  const handleDeleteEntry = (entry: Entry) => {
    setDraft((prev) => toggleEntryDelete(prev, entry));
  };

  const handleRemoveAdd = (localId: string) => {
    setDraft((prev) => removeDraftAdd(prev, localId));
  };

  const discardDraft = () => {
    if (dirtyCount === 0) return;
    setConfirm({
      title: "放弃未保存的修改？",
      message: `将丢弃 ${dirtyCount} 项暂存改动（新增 ${summary.adds} · 修改 ${summary.updates} · 删除 ${summary.deletes}），此操作不可撤销。`,
      confirmLabel: "放弃修改",
      danger: true,
      onConfirm: () => {
        setConfirm(null);
        setDraft(emptyDraft());
        setIssues([]);
        setSaveError(null);
      },
    });
  };

  const toggleSort = (field: SortField) => {
    setSort((prev) =>
      prev.field === field
        ? { field, desc: !prev.desc }
        : { field, desc: field === "weight" },
    );
  };

  const clearFilters = () => {
    setKeywordInput("");
    setGroup(null);
    setSort(DEFAULT_SORT);
  };

  // ── mock 调试动作（仅纯浏览器模式显示） ──
  const loadStress = () => {
    if (!currentFile) return;
    try {
      mockFillStress(currentFile.id, 100_000);
      doReload();
      setNotice("已灌入 10 万行压测数据（仅 mock 模式）");
    } catch (error) {
      setSaveError(toAppError(error));
    }
  };

  const simulateExternalChange = () => {
    if (!currentFile) return;
    try {
      const event = mockSimulateExternalChange(currentFile.id);
      emitExternalChange(event);
      setNotice("已模拟外部修改：现在保存会被后端以 externalModified 拒绝");
    } catch (error) {
      setSaveError(toAppError(error));
    }
  };

  const resetToken = [
    queryFileId ?? "",
    keyword,
    group ?? "\u0000all",
    sort.field,
    sort.desc ? "1" : "0",
    revision,
    openNonce,
  ].join("|");

  const hasFilters = keyword.trim() !== "" || group !== null || sort.field !== "lineNo";
  const groupOptions = entryWindow.groups;

  // ───────────────────────── 渲染 ─────────────────────────

  return (
    <div className="workbench">
      <header className="wb-header">
        <div>
          <h1>词库工作台</h1>
          <p className="wb-header__sub">
            浏览 / 搜索 / 编辑词条。所有改动先暂存（脏标记），点「保存全部」才做写前备份 + 原子写入。
          </p>
        </div>
        <div className="wb-header__badges">
          {mockMode ? <Badge tone="info">mock 数据（无 Tauri 后端）</Badge> : <Badge tone="ok">Tauri 后端</Badge>}
          {currentScheme ? <Badge tone="muted">{currentScheme.name}</Badge> : null}
          {currentRole ? <Badge tone="muted">{roleLabel(currentRole)}</Badge> : null}
        </div>
      </header>

      {bootError ? (
        <ErrorAlert error={bootError} onRetry={() => void loadSchemes()} retryLabel="重新探测环境" />
      ) : null}

      {bootState === "loading" && !schemes ? (
        <div className="wb-center">
          <Spinner label="正在读取方案与受管文件…" />
        </div>
      ) : null}

      {bootState === "ready" && schemes && schemes.length === 0 ? (
        <EmptyState
          title="没有检测到可用的 Rime 方案"
          description="请确认 Rime 用户目录正确，且目录下存在 *.schema.yaml。"
          action={
            <button type="button" className="ui-btn" onClick={() => void loadSchemes()}>
              重新探测
            </button>
          }
        />
      ) : null}

      {schemes && schemes.length > 0 ? (
        <>
          <section className="wb-toolbar" aria-label="筛选与操作">
            <label className="wb-field-inline">
              <span>方案</span>
              <select
                className="wb-select"
                aria-label="方案"
                value={schemeId ?? ""}
                onChange={(event) => {
                  const next = schemes.find((scheme) => scheme.schemaId === event.target.value);
                  if (!next) return;
                  const nextFile =
                    next.managedFiles.find((item) => item.role !== "readOnlyDict") ??
                    next.managedFiles[0] ??
                    null;
                  guardedSelection(next.schemaId, nextFile?.id ?? null);
                }}
              >
                {schemes.map((scheme) => (
                  <option key={scheme.schemaId} value={scheme.schemaId}>
                    {scheme.name}
                    {scheme.isCurrent ? "（当前使用）" : ""}
                  </option>
                ))}
              </select>
            </label>

            <label className="wb-field-inline">
              <span>文件</span>
              <select
                className="wb-select wb-select--wide"
                aria-label="文件"
                value={fileId ?? ""}
                onChange={(event) => guardedSelection(schemeId, event.target.value || null)}
                disabled={managedFiles.length === 0}
              >
                {managedFiles.length === 0 ? <option value="">（无受管文件）</option> : null}
                {managedFiles.map((file) => (
                  <option key={file.id} value={file.id}>
                    {fileLabel(file)}
                  </option>
                ))}
              </select>
            </label>

            <div className="wb-search">
              <input
                className="wb-input"
                type="search"
                value={keywordInput}
                onChange={(event) => setKeywordInput(event.target.value)}
                placeholder="搜索文本或编码（200ms 防抖）"
                aria-label="搜索文本或编码"
                spellCheck={false}
              />
              {keywordInput ? (
                <button
                  type="button"
                  className="wb-search__clear"
                  onClick={() => setKeywordInput("")}
                  aria-label="清空搜索"
                >
                  ×
                </button>
              ) : null}
            </div>

            <label className="wb-field-inline">
              <span>分组</span>
              <select
                className="wb-select"
                aria-label="分组"
                value={group === null ? "\u0000all" : group}
                onChange={(event) =>
                  setGroup(event.target.value === "\u0000all" ? null : event.target.value)
                }
              >
                <option value={"\u0000all"}>全部</option>
                {groupOptions.map((item) => (
                  <option key={item.name === "" ? "\u0000ungrouped" : item.name} value={item.name}>
                    {item.name === "" ? "未分组" : item.name}（{item.count}）
                  </option>
                ))}
              </select>
            </label>

            <div className="wb-toolbar__actions">
              <button
                type="button"
                className="ui-btn ui-btn--primary"
                onClick={() => setAddOpen(true)}
                disabled={!currentFile || readOnly || openState !== "ready"}
              >
                新增词条
              </button>
              <button
                type="button"
                className="ui-btn"
                onClick={requestReload}
                disabled={!fileId || saving}
                title="以磁盘内容为准重新打开（会丢弃未保存修改）"
              >
                重新加载
              </button>
            </div>
          </section>

          {currentFile ? (
            <div className="wb-filebar">
              <code className="wb-filebar__path" title={currentFile.path}>
                {currentFile.path}
              </code>
              <span className="wb-filebar__sep">·</span>
              <span>
                列：
                {currentFile.columns
                  .map((column) => (column === "text" ? "文本" : column === "code" ? "编码" : "权重"))
                  .join(" / ")}
                {hasWeightColumn ? "" : "（两列无权重）"}
              </span>
              <span className="wb-filebar__sep">·</span>
              <span>共 {fileMeta?.total ?? "…"} 行</span>
              {fileMeta ? (
                <>
                  <span className="wb-filebar__sep">·</span>
                  <span className="mono" title={`mtime=${fileMeta.fingerprint.mtimeMs}`}>
                    指纹 {fingerprintShort(fileMeta.fingerprint)}
                  </span>
                </>
              ) : null}
              {hasWeightColumn ? (
                <>
                  <span className="wb-filebar__sep">·</span>
                  <span className="wb-muted">权重 = 同编码内排序（越大越靠前）</span>
                </>
              ) : (
                <>
                  <span className="wb-filebar__sep">·</span>
                  <span className="wb-muted" title="xhup 用户码表为两列格式，没有权重列">
                    无权重列：两列格式，权重不可编辑
                  </span>
                </>
              )}
              {entryWindow.loading ? <Spinner label="加载中" /> : null}
            </div>
          ) : null}

          {readOnly ? (
            <div className="wb-banner wb-banner--warn">
              <strong>该文件是系统词典（只读）</strong>
              <p>RimeTools 不会写入系统词典；请选择用户码表 / 自定义短语等个人词条文件。</p>
            </div>
          ) : null}

          {openError ? (
            <ErrorAlert error={openError} onRetry={requestReload} retryLabel="重新加载" />
          ) : null}

          {externalNotice ? (
            <div className="wb-banner wb-banner--warn" role="status">
              <strong>检测到文件被外部程序修改</strong>
              <p>
                磁盘指纹已变化（{fingerprintShort(externalNotice)}）。此时保存会被后端拒绝
                （externalModified），工具不会自动覆盖。请重新加载确认外部改动后再编辑。
              </p>
              <div className="wb-banner__actions">
                <button type="button" className="ui-btn" onClick={requestReload}>
                  重新加载
                </button>
                <button
                  type="button"
                  className="ui-btn ui-btn--ghost"
                  onClick={() => setExternalNotice(null)}
                >
                  稍后处理
                </button>
              </div>
            </div>
          ) : null}

          {notice ? (
            <div className="wb-banner wb-banner--ok" role="status">
              {notice}
              <button
                type="button"
                className="ui-btn ui-btn--sm ui-btn--ghost"
                onClick={() => setNotice(null)}
              >
                关闭
              </button>
            </div>
          ) : null}

          {dirtyCount > 0 ? (
            <div className="wb-dirtybar">
              <strong className="wb-dirtybar__count" aria-live="polite">
                {dirtyCount} 项未保存
              </strong>
              <span className="wb-muted">
                新增 {summary.adds} · 修改 {summary.updates} · 删除 {summary.deletes}
              </span>
              <label className="wb-checkbox">
                <input
                  type="checkbox"
                  checked={deployAfter}
                  onChange={(event) => setDeployAfter(event.target.checked)}
                />
                保存后部署
              </label>
              <span className="wb-spacer" />
              <button type="button" className="ui-btn" onClick={discardDraft} disabled={saving}>
                放弃修改
              </button>
              <button
                type="button"
                className="ui-btn ui-btn--primary"
                onClick={() => void saveAll()}
                disabled={saving || readOnly}
              >
                {saving ? "保存中…" : "保存全部"}
              </button>
            </div>
          ) : null}

          {lastSaved ? (
            <div className="wb-banner wb-banner--ok" role="status">
              <strong>保存成功，已用后端返回的指纹刷新本地状态</strong>
              <p>
                {lastSaved.backup ? (
                  <>
                    写前备份：<span className="mono">{baseName(lastSaved.backup.path)}</span>
                    {" · "}
                  </>
                ) : null}
                {lastSaved.deploy
                  ? `部署：${lastSaved.deploy.success ? "成功" : "失败"}（${lastSaved.deploy.durationMs}ms，${
                      lastSaved.deploy.outputVerified ? "产物已变化" : "产物未变化"
                    }）`
                  : "未触发部署"}
              </p>
              <button
                type="button"
                className="ui-btn ui-btn--sm ui-btn--ghost"
                onClick={() => setLastSaved(null)}
              >
                知道了
              </button>
            </div>
          ) : null}

          {saveError ? (
            <ErrorAlert
              error={saveError}
              onRetry={() => void saveAll()}
              retryLabel="重新保存"
              extra={
                <button
                  type="button"
                  className="ui-btn ui-btn--sm"
                  onClick={() => {
                    setSaveError(null);
                    setIssues([]);
                  }}
                >
                  关闭
                </button>
              }
            />
          ) : null}

          {issues.length > 0 ? <ValidationIssues issues={issues} /> : null}

          <EntryTable
            adds={draft.adds}
            draft={draft}
            total={entryWindow.total}
            entryAt={entryWindow.entryAt}
            hasWeightColumn={hasWeightColumn}
            loading={entryWindow.loading}
            error={entryWindow.error}
            sort={sort}
            hasFilters={hasFilters}
            onToggleSort={toggleSort}
            onRangeChange={entryWindow.ensureRange}
            onEditEntry={(entry) => setEditTarget({ kind: "entry", entry })}
            onEditAdd={(add) => setEditTarget({ kind: "add", add })}
            onDeleteEntry={handleDeleteEntry}
            onRemoveAdd={handleRemoveAdd}
            onClearFilters={clearFilters}
            onReload={requestReload}
            resetToken={resetToken}
            readOnly={readOnly}
          />

          <footer className="wb-footer">
            <span>
              过滤后 {entryWindow.total} 条
              {keyword.trim() ? ` · 关键词「${keyword.trim()}」` : ""}
              {group !== null ? ` · 分组「${group === "" ? "未分组" : group}」` : ""}
            </span>
            <span>
              排序：
              {sort.field === "text"
                ? "文本"
                : sort.field === "code"
                  ? "编码"
                  : sort.field === "weight"
                    ? "权重"
                    : "源文件行号"}
              {sort.desc ? " 降序" : " 升序"}
            </span>
            <span className="wb-spacer" />
            <span className="wb-muted">双击行编辑 · Ctrl/⌘+S 保存</span>
          </footer>

          {mockMode && currentFile ? (
            <div className="wb-devbar">
              <span className="wb-devbar__label">mock 调试</span>
              <button type="button" className="ui-btn ui-btn--sm" onClick={loadStress} disabled={saving}>
                灌入 10 万行（验证虚拟滚动）
              </button>
              <button type="button" className="ui-btn ui-btn--sm" onClick={simulateExternalChange} disabled={saving}>
                模拟外部修改
              </button>
              <span className="wb-muted">
                用于验证分页 + 虚拟滚动、externalModified 冲突流程；真实环境请用 Tauri 宿主。
              </span>
            </div>
          ) : null}
        </>
      ) : null}

      {addOpen && currentFile ? (
        <AddEntryDialog
          scheme={currentScheme}
          file={currentFile}
          hasWeightColumn={hasWeightColumn}
          role={currentRole}
          groups={groupNames}
          onClose={() => setAddOpen(false)}
          onAdd={handleAdd}
        />
      ) : null}

      {editTarget ? (
        <EditDialog
          key={editTarget.kind === "entry" ? `e:${editTarget.entry.key.text}:${editTarget.entry.key.code}` : `a:${editTarget.add.localId}`}
          target={editTarget}
          hasWeightColumn={hasWeightColumn}
          role={currentRole}
          groups={groupNames}
          onClose={() => setEditTarget(null)}
          onApply={applyEdit}
        />
      ) : null}

      {conflict ? (
        <ExternalModifiedDialog
          error={conflict}
          pending={summary}
          fileLabel={currentFile ? baseName(currentFile.path) : "文件"}
          onReload={() => {
            setConflict(null);
            doReload();
          }}
          onCancel={() => setConflict(null)}
        />
      ) : null}

      {confirm ? (
        <ConfirmDialog
          title={confirm.title}
          message={confirm.message}
          confirmLabel={confirm.confirmLabel}
          danger={confirm.danger}
          onConfirm={confirm.onConfirm}
          onClose={() => setConfirm(null)}
        />
      ) : null}
    </div>
  );
}

/** 校验问题列表（保存前校验 / 本地预检共用） */
function ValidationIssues({ issues }: { issues: ValidationIssue[] }) {
  return (
    <div className="wb-issues" role="alert">
      <h3 className="wb-section-title">校验问题（{issues.length}）</h3>
      <ul>
        {issues.slice(0, 50).map((issue, index) => (
          <li key={`${issue.code}-${index}`}>
            <code className="wb-issues__code">{issue.code}</code>
            <span>{issue.message}</span>
            {issue.lineNo != null ? <span className="wb-muted">第 {issue.lineNo} 行</span> : null}
            {issue.text ? (
              <span className="mono">
                {issue.text}
                {issue.entryCode ? ` / ${issue.entryCode}` : ""}
              </span>
            ) : null}
          </li>
        ))}
      </ul>
      {issues.length > 50 ? <p className="wb-muted">仅显示前 50 条。</p> : null}
    </div>
  );
}