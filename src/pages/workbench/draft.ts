/**
 * 草稿模型（`docs/ipc.md` §6.1 / PRD FR-3.10）。
 *
 * 所有编辑先落在 UI 内存里，标记 dirty；「保存全部」时一次性构造成 `SaveRequest` 提交。
 * **不做乐观更新**：写成功后以后端返回的 fingerprint 与重新查询结果为准。
 *
 * 这里全部是纯函数，便于单测与推理；页面只负责 setState。
 */

import type {
  Entry,
  EntryKey,
  EntryUpdate,
  FileId,
  Fingerprint,
  NewEntry,
  SaveRequest,
  ValidationIssue,
} from "../../ipc/types";

export interface DraftAdd {
  /** 本地唯一 id（新增词条还没有服务端 key） */
  localId: string;
  text: string;
  code: string;
  weight: number | null;
  group: string | null;
}

/** 对既有词条的暂存修改（保留原始 key 用于后端定位） */
export interface DraftUpdate {
  key: EntryKey;
  base: Entry;
  text: string;
  code: string;
  weight: number | null;
  group: string | null;
}

export interface Draft {
  adds: DraftAdd[];
  updates: Record<string, DraftUpdate>;
  deletes: Record<string, true>;
}

export function emptyDraft(): Draft {
  return { adds: [], updates: {}, deletes: {} };
}

let localSeq = 0;

export function createDraftAdd(row: Omit<DraftAdd, "localId">): DraftAdd {
  localSeq += 1;
  return { ...row, localId: `new-${localSeq}` };
}

/** EntryKey 的稳定字符串 id（同一文件内 text+code 唯一） */
export function entryKeyId(key: EntryKey): string {
  return `${key.fileId}\u0000${key.text}\u0000${key.code}`;
}

export interface DraftSummary {
  adds: number;
  updates: number;
  deletes: number;
  total: number;
}

export function draftSummary(draft: Draft): DraftSummary {
  const adds = draft.adds.length;
  const updates = Object.keys(draft.updates).length;
  const deletes = Object.keys(draft.deletes).length;
  return { adds, updates, deletes, total: adds + updates + deletes };
}

export function isDraftDirty(draft: Draft): boolean {
  return draftSummary(draft).total > 0;
}

export function draftUpdateFor(draft: Draft, key: EntryKey): DraftUpdate | undefined {
  return draft.updates[entryKeyId(key)];
}

export function isDraftDeleted(draft: Draft, key: EntryKey): boolean {
  return draft.deletes[entryKeyId(key)] === true;
}

export function addDraftAdds(draft: Draft, rows: Array<Omit<DraftAdd, "localId">>): Draft {
  if (rows.length === 0) return draft;
  return { ...draft, adds: [...draft.adds, ...rows.map(createDraftAdd)] };
}

export function patchDraftAdd(
  draft: Draft,
  localId: string,
  patch: { text: string; code: string; weight: number | null; group: string | null },
): Draft {
  return {
    ...draft,
    adds: draft.adds.map((row) => (row.localId === localId ? { ...row, ...patch } : row)),
  };
}

export function removeDraftAdd(draft: Draft, localId: string): Draft {
  return { ...draft, adds: draft.adds.filter((row) => row.localId !== localId) };
}

/**
 * 暂存对既有词条的修改。
 * 注意（产品红线 FR-3.23）：这里只接受用户显式给出的字段，**绝不根据新文本反推/改写编码**。
 * 若修改后的值与原值完全一致，则撤销这条 edit（不留 dirty）。
 */
export function stageEntryUpdate(
  draft: Draft,
  entry: Entry,
  patch: { text: string; code: string; weight: number | null; group: string | null },
): Draft {
  const id = entryKeyId(entry.key);
  const updates = { ...draft.updates };
  const unchanged =
    patch.text === entry.key.text &&
    patch.code === entry.key.code &&
    patch.weight === entry.weight &&
    patch.group === entry.group;

  if (unchanged) {
    delete updates[id];
    return { ...draft, updates };
  }
  updates[id] = { key: entry.key, base: entry, ...patch };
  return { ...draft, updates };
}

/** 标记/取消删除既有词条（真正删除在保存时提交） */
export function toggleEntryDelete(draft: Draft, entry: Entry): Draft {
  const id = entryKeyId(entry.key);
  const deletes = { ...draft.deletes };
  if (deletes[id]) {
    delete deletes[id];
  } else {
    deletes[id] = true;
  }
  return { ...draft, deletes };
}

/**
 * 构造 `SaveRequest`（`docs/ipc.md` §2 / §5.1）。
 * - `expectedFingerprint` 必传：后端据此做外部修改检测；
 * - 无权重列的文件不传 weight（避免写入第三列）；
 * - 只有真正变化的字段才进 `update`。
 */
export function buildSaveRequest(params: {
  fileId: FileId;
  fingerprint: Fingerprint;
  draft: Draft;
  deployAfter: boolean;
  hasWeightColumn: boolean;
}): SaveRequest {
  const { fileId, fingerprint, draft, deployAfter, hasWeightColumn } = params;

  const add: NewEntry[] = draft.adds.map((row) => ({
    text: row.text,
    code: row.code,
    weight: hasWeightColumn ? row.weight : undefined,
    group: row.group ?? undefined,
  }));

  const update: EntryUpdate[] = Object.values(draft.updates).map((row) => ({
    key: row.key,
    newText: row.text !== row.base.key.text ? row.text : undefined,
    newCode: row.code !== row.base.key.code ? row.code : undefined,
    weight: hasWeightColumn && row.weight !== row.base.weight ? row.weight : undefined,
    group: row.group !== row.base.group ? row.group : undefined,
  }));

  // 删除项的 key 直接由 id 还原（id 由 fileId + text + code 组成）
  const remove: EntryKey[] = Object.keys(draft.deletes).map(splitKeyId);

  return {
    fileId,
    expectedFingerprint: fingerprint,
    add,
    update,
    delete: remove,
    reorder: null,
    deployAfter,
  };
}

/** `entryKeyId` 的逆运算（fileId 可能含 \u0000？实际路径不会，安全） */
function splitKeyId(id: string): EntryKey {
  const parts = id.split("\u0000");
  return { fileId: parts[0] ?? "", text: parts[1] ?? "", code: parts[2] ?? "" };
}

/**
 * 本地快速预检（不替代后端 `dict_validate`，只为立刻给出反馈）。
 * 覆盖：空文本/空编码、权重非法、暂存集合内重复、字符集与码长。
 */
export function localPrecheck(
  draft: Draft,
  options: { alphabet?: string; maxCodeLength?: number | null; hasWeightColumn: boolean },
): ValidationIssue[] {
  const issues: ValidationIssue[] = [];
  const alphabet = options.alphabet ? new Set(options.alphabet.split("")) : null;
  const seen = new Map<string, true>();

  const rows: Array<{ text: string; code: string; weight: number | null; lineNo: number | null }> = [
    ...draft.adds.map((a) => ({ text: a.text, code: a.code, weight: a.weight, lineNo: null })),
    ...Object.values(draft.updates).map((u) => ({
      text: u.text,
      code: u.code,
      weight: u.weight,
      lineNo: u.base.lineNo,
    })),
  ];

  for (const row of rows) {
    const text = row.text.trim();
    const code = row.code.trim();
    if (!text) {
      issues.push({ code: "validationFailed", message: "文本不能为空" });
      continue;
    }
    if (!code) {
      issues.push({ code: "validationFailed", message: `「${text}」的编码不能为空`, text });
      continue;
    }
    if (/[\t\n\r]/.test(text) || /[\t\n\r]/.test(code)) {
      issues.push({
        code: "fileFormatInvalid",
        message: "文本或编码包含 Tab / 换行",
        lineNo: row.lineNo ?? undefined,
        text,
        entryCode: code,
      });
    }
    if (alphabet) {
      const bad = [...new Set([...code].filter((ch) => !alphabet.has(ch)))];
      if (bad.length > 0) {
        issues.push({
          code: "codeCharsetInvalid",
          message: `编码含方案字母表之外的字符：${bad.join(" ")}`,
          lineNo: row.lineNo ?? undefined,
          text,
          entryCode: code,
        });
      }
    }
    if (options.maxCodeLength != null && code.length > options.maxCodeLength) {
      issues.push({
        code: "codeTooLong",
        message: `编码长度 ${code.length} 超过方案上限 ${options.maxCodeLength}`,
        lineNo: row.lineNo ?? undefined,
        text,
        entryCode: code,
      });
    }
    if (options.hasWeightColumn && row.weight !== null) {
      if (!Number.isInteger(row.weight) || row.weight < 0) {
        issues.push({
          code: "validationFailed",
          message: `权重必须是非负整数（当前：${row.weight}）`,
          lineNo: row.lineNo ?? undefined,
          text,
          entryCode: code,
        });
      }
    }
    const dup = `${text}\u0000${code}`;
    if (seen.has(dup)) {
      issues.push({
        code: "duplicateEntry",
        message: `重复词条：${text} / ${code}`,
        lineNo: row.lineNo ?? undefined,
        text,
        entryCode: code,
      });
    } else {
      seen.set(dup, true);
    }
  }
  return issues;
}