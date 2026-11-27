//! 词库内存索引：打开、分页查询、分组、暂存编辑、校验与序列化。
//!
//! ## 为什么是内存索引（技术决策，勿改）
//!
//! 10 万条词条的内存过滤是毫秒级，远低于 PRD 的 200ms 预算；SQLite 留到真的需要
//! （>50 万条或需要持久化索引）时再上。因此本模块**不引入任何数据库依赖**。
//!
//! ## 数据模型
//!
//! - `base`：打开时（或上次 `mark_saved` 后）的文件模型，永不被 `stage` 改动；
//! - `working`：`base` + 当前暂存编辑。查询 / 校验 / 序列化都只看 `working`；
//! - 分组是**工具级元数据**，只存在于 `base_groups` / `working_groups`（与词条行
//!   顺序一一对齐），绝不写入 Rime 文件（FR-3.4）。跨进程持久化由上层负责；
//!   同一进程内保存后请调用 `mark_saved` 刷新（重新 `open` 会丢分组）。
//!
//! ## 暂存模型（FR-3.10）
//!
//! `stage(&SaveRequest)` 每次都从 `base` 重建 `working`，再按
//! `delete → update → add → reorder` 的顺序应用请求。因此：
//!
//! - **幂等**：同一份 `SaveRequest` 重复 stage 不会叠加出重复条目；
//! - **可放弃**：`discard_staged()` 丢弃全部暂存，回到磁盘基线；
//! - 每次请求必须是「相对磁盘基线的**完整** diff」，不支持增量累积
//!   （命令层 `dict_validate` / `dict_save` 都是这一语义）。
//!
//! 外部修改检测（`expected_fingerprint` 比对）属于写入安全协议，由 `rime-safety`
//! 在落盘前完成；`stage` 只做内存暂存，不比对指纹（见 `docs/ipc.md` §5.1）。
//!
//! ## 保真
//!
//! 序列化复用 M0 的行模型：未编辑的行原样输出，只有真正改动（文本 / 编码 / 权重）
//! 的行按列风格重排；仅修改分组不会改动任何字节。

use crate::types::{
    AppError, AppResult, ColumnKind, Entry, EntryKey, EntryOrigin, EntryPage, EntryQuery,
    ErrorCode, FileFormat, FileId, FileRole, Fingerprint, GroupCount, SaveRequest, SortField,
    SortSpec, ValidationIssue,
};
use crate::{detect_columns, fingerprint_bytes, validate_dict, EntryLine, Line, ParsedDict, WriteOptions};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// `GroupCount` 中「未分组」的保留名称（侧栏筛选用；不会写进 Rime 文件）。
pub const UNGROUPED: &str = "未分组";

/// 单文件词库的内存索引与暂存状态。
///
/// `Clone` 用于命令层的「在副本上 stage + validate」场景（`dict_validate`）。
#[derive(Debug, Clone)]
pub struct DictStore {
    /// 稳定标识，与 `rime_platform::make_file_id` 一致（`path.display()`）。
    file_id: FileId,
    path: PathBuf,
    format: FileFormat,
    role: FileRole,
    /// 由文件内容推断的列风格，打开后固定；序列化始终使用它
    /// （避免「两列 xhup 用户码表被写成三列」）。
    columns: Vec<ColumnKind>,
    fingerprint: Fingerprint,
    /// 磁盘基线（上一次成功保存后的内容）。
    base: ParsedDict,
    /// 基线 + 暂存编辑。
    working: ParsedDict,
    /// 与词条行顺序对齐的分组元数据（基线 / 工作副本）。
    base_groups: Vec<Option<String>>,
    working_groups: Vec<Option<String>>,
    /// 基线中已存在的 文本+编码，用于判定 `EntryOrigin`。
    base_keys: HashSet<(String, String)>,
    /// 本次暂存新增 / 修改的词条键（用于 `EntryOrigin`，`mark_saved` 后清空）。
    added_keys: HashSet<(String, String)>,
    edited_keys: HashSet<(String, String)>,
    has_staged: bool,
}

impl DictStore {
    /// 从文件加载。格式类解析失败映射为 `AppError(FileFormatInvalid)`；
    /// 非 UTF-8 映射为 `CorruptEncoding`（契约 §1.1 的专用错误码）。
    ///
    /// 指纹基于**本次实际读到的字节**计算（读一次文件，避免「内容与指纹来自
    /// 两次读取」的错配；外部并发修改仍由写入前的指纹复核兜住）。
    pub fn open(path: &Path, format: FileFormat, role: FileRole) -> AppResult<Self> {
        let bytes = fs::read(path).map_err(|e| io_error(path, e))?;
        let metadata = fs::metadata(path).map_err(|e| io_error(path, e))?;
        let mtime = metadata
            .modified()
            .map_err(|_| {
                AppError::new(
                    ErrorCode::Internal,
                    format!("无法读取文件修改时间：{}", path.display()),
                )
            })?
            .duration_since(UNIX_EPOCH)
            .map_err(|_| {
                AppError::new(
                    ErrorCode::Internal,
                    format!("文件修改时间早于 UNIX 纪元，无法生成指纹：{}", path.display()),
                )
            })?;
        let fingerprint = Fingerprint {
            mtime_ms: mtime.as_millis() as i64,
            size: bytes.len() as u64,
            sha256: fingerprint_bytes(&bytes),
        };

        let content = String::from_utf8(bytes).map_err(|e| {
            AppError::new(
                ErrorCode::CorruptEncoding,
                format!("文件不是合法 UTF-8：{}", path.display()),
            )
            .with_detail(e.to_string())
            .with_hint("Rime 文件必须为 UTF-8（推荐无 BOM）")
        })?;

        let parsed = ParsedDict::parse(&content, format).map_err(|e| parse_error(path, e))?;
        let columns = detect_columns(format, &parsed.lines);
        let entry_count = parsed.entry_count();
        let base_keys = collect_entry_keys(&parsed);

        Ok(Self {
            file_id: path.display().to_string(),
            path: path.to_path_buf(),
            format,
            role,
            columns,
            fingerprint,
            base: parsed.clone(),
            working: parsed,
            base_groups: vec![None; entry_count],
            working_groups: vec![None; entry_count],
            base_keys,
            added_keys: HashSet::new(),
            edited_keys: HashSet::new(),
            has_staged: false,
        })
    }

    /// 文件路径（绝对或打开时传入的路径）。
    pub fn file_path(&self) -> &Path {
        &self.path
    }

    /// 稳定文件 id（与 `dirs` / 受管文件映射一致）。
    pub fn file_id(&self) -> &FileId {
        &self.file_id
    }

    /// 文件格式（打开时传入）。
    pub fn format(&self) -> FileFormat {
        self.format
    }

    /// 文件列风格（两列 / 三列），打开时推断并固定。
    pub fn columns(&self) -> &[ColumnKind] {
        &self.columns
    }

    /// 当前工作副本的词条总数（含暂存增删）。
    pub fn total(&self) -> usize {
        self.working.entry_count()
    }

    /// 打开文件时记录的指纹（写入前用于外部修改检测）。
    pub fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }

    /// 分页查询：关键词同时匹配 text 与 code（大小写不敏感），支持分组筛选与排序。
    ///
    /// 语义约定：
    /// - `file_id` 给出但不等于本词库时返回空页（多文件索引由上层路由）；
    /// - `total` 是「关键词 + 分组」过滤后的总数，与 `offset/limit` 无关；
    /// - `groups` 是**关键词过滤后、分组过滤前**的分组计数（侧栏在搜索时也能显示计数）；
    /// - `limit == 0` 返回空 `items`（但 `total` 仍是过滤后总数）；
    /// - 排序的平局以文件行号升序兜底，保证分页稳定。
    pub fn query(&self, q: &EntryQuery) -> EntryPage {
        if let Some(file_id) = &q.file_id {
            if file_id != &self.file_id {
                return EntryPage { items: Vec::new(), total: 0, groups: Vec::new() };
            }
        }

        let keyword = q
            .keyword
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase);
        // 大小写不敏感匹配（只折叠 ASCII，与 `to_ascii_lowercase` 语义一致，
        // 但不在每条词条上分配小写副本——10 万条时这是主要开销）

        let mut items: Vec<Entry> = Vec::new();
        let mut ordinal = 0usize;
        for (idx, line) in self.working.lines.iter().enumerate() {
            let entry = match line {
                Line::Entry(e) => e,
                Line::Raw(_) => continue,
            };
            let group = group_at(&self.working_groups, ordinal);
            ordinal += 1;

            if let Some(keyword) = &keyword {
                let hit = contains_ascii_case_insensitive(&entry.text, keyword)
                    || contains_ascii_case_insensitive(&entry.code, keyword);
                if !hit {
                    continue;
                }
            }

            items.push(Entry {
                key: EntryKey {
                    file_id: self.file_id.clone(),
                    text: entry.text.clone(),
                    code: entry.code.clone(),
                },
                weight: entry.weight,
                group,
                // 行号是「当前工作副本按序写出后的 1-based 行号」，与源文件一致
                // （新增行追加在数据区末尾，删除行会让后续行号前移）。
                line_no: Some((idx + 1) as u32),
                origin: self.origin_of(entry),
            });
        }

        let groups = counts_of(items.iter().map(|e| e.group.as_deref()));

        let group_filter: Option<Option<&str>> = match q.group.as_deref().map(str::trim) {
            None => None,
            // 空串与「未分组」都表示筛选未分组；`None` 才表示「全部分组」
            Some("") => Some(None),
            Some(g) if g == UNGROUPED => Some(None),
            Some(g) => Some(Some(g)),
        };
        let mut items: Vec<Entry> = match group_filter {
            None => items,
            Some(None) => items.into_iter().filter(|e| e.group.is_none()).collect(),
            Some(Some(g)) => items
                .into_iter()
                .filter(|e| e.group.as_deref() == Some(g))
                .collect(),
        };

        sort_entries(&mut items, &q.sort);

        let total = items.len() as u32;
        let items = items
            .into_iter()
            .skip(q.offset as usize)
            .take(q.limit as usize)
            .collect();

        EntryPage { items, total, groups }
    }

    /// 分组计数（含「未分组」）。
    ///
    /// 顺序固定：「未分组」在前（若有），其余按名称升序，便于侧栏稳定渲染。
    pub fn group_counts(&self) -> Vec<GroupCount> {
        let mut ordinal = 0usize;
        let groups = self.working.lines.iter().filter_map(|line| {
            if matches!(line, Line::Entry(_)) {
                let g = self
                    .working_groups
                    .get(ordinal)
                    .and_then(|g| g.as_deref());
                ordinal += 1;
                Some(g)
            } else {
                None
            }
        });
        counts_of(groups)
    }

    /// 暂存编辑：增 / 改 / 删 / 重排（不落盘）。
    ///
    /// 以磁盘基线为起点重建工作副本，**幂等**：同一请求重复调用结果一致。
    /// 失败时（如键不存在）不改动当前状态。
    ///
    /// 约定：
    /// - `EntryKey.file_id` 不参与匹配（词库本身是单文件；请求已按 `file_id` 路由）；
    /// - `update.new_text` / `update.new_code` 为 `None` 表示不改该字段；
    /// - `update.weight` 为 `None` 表示不改；无权重列的文件忽略权重修改
    ///   （xhup 用户码表两列，FR-3.11）；
    /// - `update.group` 为 `Some("")` 表示清除分组；
    /// - `reorder` 只需列出要前置的词条（未列出的词条按原顺序补在后面），
    ///   出现未知键时返回 `NotFound`；
    /// - `NewEntry.weight` 在无权重列的文件里被忽略（恒为 `None`）。
    pub fn stage(&mut self, req: &SaveRequest) -> AppResult<()> {
        if self.role == FileRole::ReadOnlyDict {
            return Err(AppError::new(
                ErrorCode::FileNotWritable,
                format!("系统词典只读，不允许编辑：{}", self.path.display()),
            )
            .with_hint("请把词条加入个人词库后再保存"));
        }

        // 从基线重建：这是「幂等」与「可放弃」的前提
        let mut working = self.base.clone();
        let mut groups = self.base_groups.clone();
        let mut added_keys: HashSet<(String, String)> = HashSet::new();
        let mut edited_keys: HashSet<(String, String)> = HashSet::new();
        let mut changed = false;

        // ── 1) 删除 ──
        // 单趟扫描完成：批量删除 1000 条时是 O(n) 而不是 O(n²)（PRD AC）。
        if !req.delete.is_empty() {
            let mut pending: HashSet<(String, String)> = req
                .delete
                .iter()
                .map(|k| (k.text.clone(), k.code.clone()))
                .collect();
            let mut kept_lines = Vec::with_capacity(working.lines.len());
            let mut kept_groups = Vec::with_capacity(groups.len());
            for line in std::mem::take(&mut working.lines) {
                match line {
                    Line::Entry(entry) => {
                        if !pending.is_empty() {
                            // 同一文本+编码只删第一条（与 find_entry 语义一致）
                            let key = (entry.text.clone(), entry.code.clone());
                            if pending.remove(&key) {
                                changed = true;
                                continue; // 该行分组一并丢弃，序号不推进
                            }
                        }
                        let ordinal = kept_groups.len();
                        kept_groups.push(groups.get(ordinal).cloned().flatten());
                        kept_lines.push(Line::Entry(entry));
                    }
                    raw => kept_lines.push(raw),
                }
            }
            if !pending.is_empty() {
                // 有键没找到：整批失败，store 状态保持原样
                let missing = req
                    .delete
                    .iter()
                    .find(|k| pending.contains(&(k.text.clone(), k.code.clone())))
                    .cloned()
                    .expect("pending 非空必然对应某个 delete 键");
                return Err(entry_not_found("删除", &missing));
            }
            working.lines = kept_lines;
            groups = kept_groups;
        }

        // ── 2) 修改 ──
        let has_weight_column = self.columns.contains(&ColumnKind::Weight);
        if !req.update.is_empty() {
            // 行下标 → 词条序号（修改不移动行，整批只建一次，避免逐个 O(n) 数词条）
            let mut ordinals: Vec<Option<usize>> = Vec::with_capacity(working.lines.len());
            let mut next = 0usize;
            for line in &working.lines {
                if matches!(line, Line::Entry(_)) {
                    ordinals.push(Some(next));
                    next += 1;
                } else {
                    ordinals.push(None);
                }
            }

            for upd in &req.update {
                let idx = working
                    .find_entry(&upd.key.text, &upd.key.code)
                    .ok_or_else(|| entry_not_found("修改", &upd.key))?;
                let ordinal = ordinals[idx].expect("find_entry 命中的一定是词条行");

                let mut final_text = upd.key.text.clone();
                let mut final_code = upd.key.code.clone();
                if let Line::Entry(entry) = &mut working.lines[idx] {
                    if let Some(text) = &upd.new_text {
                        if *text != entry.text {
                            entry.set_text(text.clone());
                            changed = true;
                        }
                        final_text = entry.text.clone();
                    }
                    if let Some(code) = &upd.new_code {
                        if *code != entry.code {
                            entry.set_code(code.clone());
                            changed = true;
                        }
                        final_code = entry.code.clone();
                    }
                    if let Some(weight) = upd.weight {
                        if has_weight_column {
                            if entry.weight != Some(weight) {
                                entry.set_weight(Some(weight));
                                changed = true;
                            }
                        } else {
                            tracing::debug!(
                                file = %self.path.display(),
                                text = %entry.text,
                                "无权重列的文件忽略权重修改（xhup 用户码表为两列）"
                            );
                        }
                    }
                }
                edited_keys.insert((final_text, final_code));

                if let Some(group) = &upd.group {
                    let new_group = if group.trim().is_empty() {
                        None
                    } else {
                        Some(group.clone())
                    };
                    if group_at(&groups, ordinal) != new_group {
                        groups[ordinal] = new_group;
                        changed = true;
                    }
                }
            }
        }

        // ── 3) 新增 ──
        if !req.add.is_empty() {
            // 追加在数据区最后一条词条之后，避免插到尾部注释/空行之后
            let append_at = working
                .lines
                .iter()
                .rposition(|l| matches!(l, Line::Entry(_)))
                .map(|i| i + 1)
                .unwrap_or(working.lines.len());
            // 追加位置之后的词条数恒为 0，因此新词条的序号从 groups.len() 递增
            let first_new_ordinal = groups.len();
            for (offset, new) in req.add.iter().enumerate() {
                let weight = if has_weight_column { new.weight } else { None };
                let line = EntryLine::new(new.text.clone(), new.code.clone(), weight);
                working.lines.insert(append_at + offset, Line::Entry(line));
                let group = new.group.clone().filter(|g| !g.trim().is_empty());
                groups.insert(first_new_ordinal + offset, group);
                added_keys.insert((new.text.clone(), new.code.clone()));
                changed = true;
            }
        }

        // ── 4) 重排 ──
        if let Some(order) = &req.reorder {
            let slots = entry_slots(&working.lines);
            let pairs: Vec<(EntryLine, Option<String>)> = slots
                .iter()
                .enumerate()
                .map(|(ordinal, &slot)| match &working.lines[slot] {
                    Line::Entry(entry) => (entry.clone(), group_at(&groups, ordinal)),
                    Line::Raw(_) => unreachable!("entry_slots 只返回词条行"),
                })
                .collect();

            // 键 → 待放置的下标队列（同一文本+编码可能重复出现，按原顺序取用）
            let mut pending: HashMap<(String, String), VecDeque<usize>> = HashMap::new();
            for (i, (entry, _)) in pairs.iter().enumerate() {
                pending
                    .entry((entry.text.clone(), entry.code.clone()))
                    .or_default()
                    .push_back(i);
            }
            let mut order_indices: Vec<usize> = Vec::with_capacity(pairs.len());
            for key in order {
                let queue = pending
                    .get_mut(&(key.text.clone(), key.code.clone()))
                    .ok_or_else(|| entry_not_found("重排", key))?;
                let Some(index) = queue.pop_front() else {
                    return Err(AppError::new(
                        ErrorCode::NotFound,
                        format!("重排列表里词条重复出现：{}/{}", key.text, key.code),
                    ));
                };
                order_indices.push(index);
            }
            // 未列出的词条保持原有相对顺序，补在后面
            let placed: HashSet<usize> = order_indices.iter().copied().collect();
            order_indices.extend((0..pairs.len()).filter(|i| !placed.contains(i)));

            if order_indices
                .iter()
                .enumerate()
                .any(|(position, &source)| position != source)
            {
                changed = true;
            }

            let reordered: Vec<(EntryLine, Option<String>)> = order_indices
                .iter()
                .map(|&i| pairs[i].clone())
                .collect();
            let reordered_groups: Vec<Option<String>> =
                reordered.iter().map(|(_, g)| g.clone()).collect();
            for (slot, (entry, _)) in slots.iter().zip(reordered) {
                working.lines[*slot] = Line::Entry(entry);
            }
            groups = reordered_groups;
        }

        // 全部成功后再替换状态，失败时保持原样
        self.working = working;
        self.working_groups = groups;
        self.added_keys = added_keys;
        self.edited_keys = edited_keys;
        self.has_staged = changed;
        Ok(())
    }

    /// 是否存在未保存的暂存编辑。
    pub fn has_staged(&self) -> bool {
        self.has_staged
    }

    /// 丢弃全部暂存编辑，回到磁盘基线。
    pub fn discard_staged(&mut self) {
        self.working = self.base.clone();
        self.working_groups = self.base_groups.clone();
        self.added_keys.clear();
        self.edited_keys.clear();
        self.has_staged = false;
    }

    /// 校验（列 / 字符集 / 码长 / 重复 / 权重 / 非法字符）。
    ///
    /// `alphabet` 为空串表示不限制字符集；`max_code_length` 为 `None` 表示不限制码长。
    /// 逐条返回 `line_no` 与字段，供 UI 定位（FR-3.11）。
    pub fn validate(&self, alphabet: &str, max_code_length: Option<u32>) -> Vec<ValidationIssue> {
        validate_dict(&self.working, &self.columns, alphabet, max_code_length)
    }

    /// 序列化当前工作副本；供 `rime-safety` 原子写入。
    ///
    /// 返回 `WriteOptions`，命令层可直接复用其中的列风格与换行信息。
    /// 若暂存态含「会写坏文件」的字段（空文本 / 空编码 / Tab / 换行），
    /// 这里直接拒绝序列化——正常流程应先由 `validate` 报错拦截。
    pub fn serialize(&self) -> AppResult<(String, WriteOptions)> {
        for (idx, line) in self.working.lines.iter().enumerate() {
            let Line::Entry(entry) = line else { continue };
            let line_no = idx + 1;
            if entry.text.is_empty() || entry.code.is_empty() {
                return Err(AppError::new(
                    ErrorCode::ValidationFailed,
                    format!("第 {line_no} 行文本或编码为空，拒绝序列化"),
                )
                .with_hint("请先修正后再保存"));
            }
            if contains_layout_breakers(&entry.text) || contains_layout_breakers(&entry.code) {
                return Err(AppError::new(
                    ErrorCode::FileFormatInvalid,
                    format!("第 {line_no} 行文本或编码含 Tab/换行，拒绝序列化"),
                )
                .with_hint("请先修正后再保存，否则会破坏 Rime 文件的列结构"));
            }
        }

        let options = WriteOptions::new(self.columns.clone(), self.working.newline);
        let content = self.working.serialize(&options).map_err(|e| match e {
            crate::SerializeError::InvalidColumns => AppError::new(
                ErrorCode::FileFormatInvalid,
                "写入列配置非法：必须包含文本与编码列",
            ),
            crate::SerializeError::EmptyField => AppError::new(
                ErrorCode::ValidationFailed,
                "词条缺少必要字段（文本或编码为空）",
            ),
        })?;
        Ok((content, options))
    }

    /// 写入成功后刷新内部状态与指纹。
    ///
    /// 会把暂存行固化为「原始行」（清除 dirty），使后续序列化与磁盘字节保持一致，
    /// 再把工作副本提升为新的基线。
    ///
    /// **命令层应优先用它而不是重新 `open`**：分组是内存中的工具元数据，
    /// 重新 `open` 会丢掉 `group`；本方法会保留当前分组并把指纹换成写入后的新值。
    /// 前提是调用方已经把 `serialize()` 的输出原样写盘（`rime-safety` 正是如此）。
    pub fn mark_saved(&mut self, fingerprint: Fingerprint) {
        let columns = self.columns.clone();
        for line in &mut self.working.lines {
            if let Line::Entry(entry) = line {
                if entry.is_dirty() {
                    if let Err(error) = entry.commit(&columns) {
                        tracing::warn!(
                            file = %self.path.display(),
                            %error,
                            "保存后固化词条行失败，保留 dirty 标记"
                        );
                    }
                }
            }
        }
        self.base = self.working.clone();
        self.base_groups = self.working_groups.clone();
        self.base_keys = collect_entry_keys(&self.base);
        self.added_keys.clear();
        self.edited_keys.clear();
        self.has_staged = false;
        self.fingerprint = fingerprint;
    }

    /// 判定词条来源：未编辑 → `File`；新增 → `UserAdded`；其余编辑 → `UserEdited`。
    fn origin_of(&self, entry: &EntryLine) -> EntryOrigin {
        if !entry.is_dirty() {
            return EntryOrigin::File;
        }
        let key = (entry.text.clone(), entry.code.clone());
        if self.added_keys.contains(&key) {
            EntryOrigin::UserAdded
        } else if self.edited_keys.contains(&key) || self.base_keys.contains(&key) {
            EntryOrigin::UserEdited
        } else {
            EntryOrigin::UserAdded
        }
    }
}

// ───────────────────────── 内部工具 ─────────────────────────

fn collect_entry_keys(dict: &ParsedDict) -> HashSet<(String, String)> {
    dict.entries()
        .map(|(_, e)| (e.text.clone(), e.code.clone()))
        .collect()
}

/// 词条行在 `lines` 中的下标（按出现顺序）。
fn entry_slots(lines: &[Line]) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| matches!(line, Line::Entry(_)).then_some(idx))
        .collect()
}

fn group_at(groups: &[Option<String>], ordinal: usize) -> Option<String> {
    groups.get(ordinal).cloned().flatten()
}

fn sort_entries(items: &mut [Entry], spec: &SortSpec) {
    items.sort_by(|a, b| {
        let primary = match spec.field {
            SortField::Text => a.key.text.cmp(&b.key.text),
            SortField::Code => a.key.code.cmp(&b.key.code),
            // `None`（无权重列）按 0 参与排序，与 Rime「省略权重 = 默认」语义一致
            SortField::Weight => a.weight.unwrap_or(0).cmp(&b.weight.unwrap_or(0)),
            SortField::LineNo => a
                .line_no
                .unwrap_or(u32::MAX)
                .cmp(&b.line_no.unwrap_or(u32::MAX)),
        };
        let primary = if spec.desc { primary.reverse() } else { primary };
        // 平局用行号升序兜底：保证分页稳定（同一查询不会重复/漏项）
        primary.then_with(|| {
            a.line_no
                .unwrap_or(u32::MAX)
                .cmp(&b.line_no.unwrap_or(u32::MAX))
        })
    });
}

/// 分组计数：「未分组」在前，其余按名称升序。
fn counts_of<'a>(groups: impl Iterator<Item = Option<&'a str>>) -> Vec<GroupCount> {
    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
    for group in groups {
        *counts.entry(group.unwrap_or(UNGROUPED)).or_insert(0) += 1;
    }
    let mut out = Vec::with_capacity(counts.len());
    if let Some(count) = counts.remove(UNGROUPED) {
        out.push(GroupCount { name: UNGROUPED.to_owned(), count });
    }
    for (name, count) in counts {
        out.push(GroupCount { name: name.to_owned(), count });
    }
    out
}

/// 大小写不敏感的子串匹配（只折叠 ASCII 大小写，中文等非 ASCII 字节原样比较）。
///
/// `needle_lower` 必须是已做 ASCII 小写的模式；不分配内存，避免逐词条生成小写副本。
fn contains_ascii_case_insensitive(haystack: &str, needle_lower: &str) -> bool {
    let needle = needle_lower.as_bytes();
    if needle.is_empty() {
        return true;
    }
    let haystack = haystack.as_bytes();
    if haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|window| {
        window
            .iter()
            .zip(needle)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

fn contains_layout_breakers(s: &str) -> bool {
    s.contains('\t') || s.contains('\n') || s.contains('\r')
}

fn entry_not_found(action: &str, key: &EntryKey) -> AppError {
    AppError::new(
        ErrorCode::NotFound,
        format!("{action}失败：词条不存在 {}/{}", key.text, key.code),
    )
    .with_hint("词库可能已被外部修改，请重新打开")
}

fn io_error(path: &Path, source: std::io::Error) -> AppError {
    let code = match source.kind() {
        std::io::ErrorKind::NotFound => ErrorCode::FileNotFound,
        std::io::ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
        _ => ErrorCode::Internal,
    };
    AppError::new(code, format!("读取失败：{}：{source}", path.display()))
}

fn parse_error(path: &Path, source: crate::ParseError) -> AppError {
    let (code, hint) = match source {
        crate::ParseError::InvalidUtf8(_) => (
            ErrorCode::CorruptEncoding,
            Some("Rime 文件必须为 UTF-8（推荐无 BOM）"),
        ),
        crate::ParseError::MissingYamlTerminator => (
            ErrorCode::FileFormatInvalid,
            Some("词典 YAML 头必须以单独一行的 `...` 结束；请检查文件是否完整"),
        ),
        crate::ParseError::TooLarge { .. } => (
            ErrorCode::FileFormatInvalid,
            Some("文件超出词库大小上限，暂不支持打开"),
        ),
    };
    let mut error = AppError::new(
        code,
        format!("解析失败：{}：{source}", path.display()),
    )
    .with_detail(source.to_string());
    if let Some(hint) = hint {
        error = error.with_hint(hint);
    }
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EntryOrigin, FileFormat};

    fn write_temp(content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("custom_phrase.txt");
        fs::write(&path, content).unwrap();
        (dir, path)
    }

    fn request(store: &DictStore) -> SaveRequest {
        SaveRequest {
            file_id: store.file_id().clone(),
            expected_fingerprint: store.fingerprint().clone(),
            add: Vec::new(),
            update: Vec::new(),
            delete: Vec::new(),
            reorder: None,
            deploy_after: false,
        }
    }

    #[test]
    fn open_maps_missing_file_to_file_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let err = DictStore::open(
            &dir.path().join("nope.txt"),
            FileFormat::CustomPhrase,
            FileRole::CustomPhrase,
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::FileNotFound);
    }

    #[test]
    fn open_maps_broken_yaml_to_file_format_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.dict.yaml");
        fs::write(&path, "name: x\n没有分隔符\n").unwrap();
        let err = DictStore::open(&path, FileFormat::RimeDictYaml, FileRole::UserDict).unwrap_err();
        assert_eq!(err.code, ErrorCode::FileFormatInvalid);
    }

    #[test]
    fn stage_is_idempotent_and_byte_stable_for_group_only_edits() {
        let (_dir, path) = write_temp("一个\tig\n美团\tmwtr\t100\n");
        let mut store =
            DictStore::open(&path, FileFormat::CustomPhrase, FileRole::CustomPhrase).unwrap();
        let original = fs::read_to_string(&path).unwrap();

        let mut req = request(&store);
        req.update.push(crate::types::EntryUpdate {
            key: EntryKey {
                file_id: store.file_id().clone(),
                text: "美团".to_owned(),
                code: "mwtr".to_owned(),
            },
            new_text: None,
            new_code: None,
            weight: None,
            group: Some("收藏".to_owned()),
        });

        store.stage(&req).unwrap();
        store.stage(&req).unwrap(); // 幂等：不叠加
        assert!(store.has_staged());

        let (out, _) = store.serialize().unwrap();
        assert_eq!(out, original, "仅改分组不得改动任何字节");

        let page = store.query(&EntryQuery::default());
        let entry = page.items.iter().find(|e| e.key.text == "美团").unwrap();
        assert_eq!(entry.group.as_deref(), Some("收藏"));
        assert_eq!(entry.origin, EntryOrigin::File, "仅元数据变化不改变来源");
    }

    #[test]
    fn read_only_role_cannot_stage() {
        let (_dir, path) = write_temp("一个\tig\n");
        let mut store =
            DictStore::open(&path, FileFormat::CustomPhrase, FileRole::ReadOnlyDict).unwrap();
        let err = store.stage(&request(&store)).unwrap_err();
        assert_eq!(err.code, ErrorCode::FileNotWritable);
    }

    #[test]
    fn failure_in_stage_leaves_state_untouched() {
        let (_dir, path) = write_temp("一个\tig\n");
        let mut store =
            DictStore::open(&path, FileFormat::CustomPhrase, FileRole::CustomPhrase).unwrap();
        let before = store.serialize().unwrap().0;
        let mut req = request(&store);
        req.delete.push(EntryKey {
            file_id: store.file_id().clone(),
            text: "不存在".to_owned(),
            code: "xxx".to_owned(),
        });
        let err = store.stage(&req).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert!(!store.has_staged());
        assert_eq!(store.serialize().unwrap().0, before);
    }
}