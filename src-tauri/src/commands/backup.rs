//! 备份与历史命令（契约 `docs/ipc.md` §3.6 / PRD FR-6）。

use crate::state::AppState;
use rime_core::types::{
    AppError, BackupReason, BackupRef, ErrorCode, FileDiff, FileId, SaveResult,
};
use rime_safety::backup::{create_backup, list_backups, BackupOptions};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupRestoreArgs {
    pub backup_id: String,
    pub dry_run: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreOutcome {
    pub diff: FileDiff,
    /// `dry_run` 时为 None
    pub result: Option<SaveResult>,
}

/// 列出备份（可按来源文件过滤）。
#[tauri::command]
pub fn backup_list(
    file_id: Option<FileId>,
    state: State<'_, AppState>,
) -> Result<Vec<BackupRef>, AppError> {
    let dir = state.backup_dir();
    let sources: Vec<std::path::PathBuf> = match &file_id {
        Some(id) => state
            .managed_file(id)
            .map(|f| vec![f.path])
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "未注册的受管文件"))?,
        None => state.managed_files(),
    };

    // `list_backups` 以「某个源文件的备份」为单位，这里按受管文件逐个取再合并
    let mut all = Vec::new();
    for src in sources {
        match list_backups(&src, Some(&dir)) {
            Ok(mut list) => all.append(&mut list),
            Err(e) => return Err(to_app_error(e, "读取备份列表失败")),
        }
    }
    all.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    // 用原因索引还原「手动/写前自动」等语义（`list_backups` 从磁盘重建时只有默认原因）
    crate::backup_index::apply(&dir, &mut all);
    Ok(all)
}

/// 立即备份全部受管文件。
#[tauri::command]
pub fn backup_now(
    file_ids: Option<Vec<FileId>>,
    state: State<'_, AppState>,
) -> Result<Vec<BackupRef>, AppError> {
    let dir = state.backup_dir();
    let targets: Vec<_> = match file_ids {
        Some(ids) => ids
            .iter()
            .filter_map(|id| state.managed_file(id))
            .map(|f| f.path)
            .collect(),
        None => state.managed_files(),
    };

    if targets.is_empty() {
        return Err(AppError::new(ErrorCode::NotFound, "没有可备份的受管文件")
            .with_hint("请先在设置中确认环境与方案"));
    }

    let mut out = Vec::with_capacity(targets.len());
    for path in targets {
        let contents = std::fs::read(&path).map_err(|e| {
            AppError::new(
                ErrorCode::FileNotFound,
                format!("读取待备份文件失败：{}", path.display()),
            )
            .with_detail(e.to_string())
        })?;
        let opts = BackupOptions {
            dir: Some(dir.clone()),
            reason: BackupReason::Manual,
        };
        match create_backup(&path, &contents, &opts) {
            Ok(r) => out.push(r),
            Err(e) => return Err(to_app_error(e, "备份失败")),
        }
    }
    crate::backup_index::record(&dir, &out);
    Ok(out)
}

/// 恢复前先出差异（`dry_run: true`），确认后才真正恢复。
///
/// 数据安全：真正恢复前会**再备份当前版本**，保证恢复本身也可回滚。
#[tauri::command]
pub fn backup_restore(
    args: BackupRestoreArgs,
    state: State<'_, AppState>,
) -> Result<RestoreOutcome, AppError> {
    let dir = state.backup_dir();
    let mut backups = Vec::new();
    for src in state.managed_files() {
        if let Ok(mut list) = list_backups(&src, Some(&dir)) {
            backups.append(&mut list);
        }
    }
    let backup = backups
        .into_iter()
        .find(|b| b.id == args.backup_id)
        .ok_or_else(|| AppError::new(ErrorCode::NotFound, "备份不存在或已被清理"))?;

    let current = std::fs::read_to_string(&backup.source_path).map_err(|e| {
        AppError::new(
            ErrorCode::FileNotFound,
            format!("读取当前文件失败：{}", backup.source_path.display()),
        )
        .with_detail(e.to_string())
    })?;
    let restored = std::fs::read_to_string(&backup.path).map_err(|e| {
        AppError::new(ErrorCode::FileNotFound, "读取备份文件失败").with_detail(e.to_string())
    })?;

    let diff = diff_entries(&backup.source_path, &current, &restored);

    if args.dry_run {
        return Ok(RestoreOutcome { diff, result: None });
    }

    // 恢复前保护：先备份当前版本
    let protect = BackupOptions {
        dir: Some(dir.clone()),
        reason: BackupReason::BeforeRestore,
    };
    let protect_ref = create_backup(&backup.source_path, current.as_bytes(), &protect)
        .map_err(|e| to_app_error(e, "恢复前备份失败"))?;
    crate::backup_index::record(&dir, std::slice::from_ref(&protect_ref));

    // 走统一写入安全协议（指纹用当前值，避免把正在编辑的外部改动覆盖掉）
    let fingerprint = rime_core::fingerprint_file(&backup.source_path).map_err(|e| {
        AppError::new(ErrorCode::Internal, "计算当前指纹失败").with_detail(e.to_string())
    })?;
    let outcome = rime_safety::save_file(
        &backup.source_path,
        &fingerprint,
        restored.as_bytes(),
        &Default::default(),
    )
    .map_err(|e| to_app_error(e, "恢复写入失败"))?;

    // 同步内存中的 store，避免 UI 仍显示旧内容
    state.reload_store_after_external_write(&backup.source_path);

    Ok(RestoreOutcome {
        diff,
        result: Some(SaveResult {
            written: outcome.bytes_written as u32,
            backup: Some(outcome.backup),
            fingerprint: outcome.fingerprint,
            deploy: None,
        }),
    })
}

/// 删除指定备份。
#[tauri::command]
pub fn backup_delete(
    backup_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let dir = state.backup_dir();
    let mut backups = Vec::new();
    for src in state.managed_files() {
        if let Ok(mut list) = list_backups(&src, Some(&dir)) {
            backups.append(&mut list);
        }
    }
    let mut deleted = 0u32;
    for b in backups {
        if backup_ids.contains(&b.id) && std::fs::remove_file(&b.path).is_ok() {
            deleted += 1;
        }
    }
    Ok(serde_json::json!({ "deleted": deleted }))
}

/// 修改历史（M2 完整实现；此处返回空列表以保持契约可用）。
#[tauri::command]
pub fn history_list(
    _file_id: FileId,
    _limit: Option<usize>,
) -> Result<Vec<serde_json::Value>, AppError> {
    Ok(Vec::new())
}

// ───────────────────────── 辅助 ─────────────────────────

/// 词条级差异：以「文本\t编码」为键比较两个文件内容。
fn diff_entries(path: &std::path::Path, current: &str, restored: &str) -> FileDiff {
    use std::collections::BTreeMap;

    fn parse(text: &str) -> BTreeMap<(String, String), Option<u32>> {
        let mut map = BTreeMap::new();
        for line in text.lines() {
            if line.trim().is_empty() || line.starts_with('#') || line.trim() == "..." {
                continue;
            }
            let mut it = line.split('\t');
            if let (Some(t), Some(c)) = (it.next(), it.next()) {
                let weight = it.next().and_then(|w| w.trim().parse::<u32>().ok());
                map.insert((t.to_owned(), c.to_owned()), weight);
            }
        }
        map
    }

    let cur = parse(current);
    let res = parse(restored);

    let mut added = Vec::new();
    let mut modified = 0u32;
    let mut unchanged = 0u32;
    for (k, w) in &res {
        match cur.get(k) {
            None => added.push(rime_core::types::DiffLine {
                text: k.0.clone(),
                code: k.1.clone(),
                weight: *w,
            }),
            Some(cur_w) => {
                if cur_w != w {
                    modified += 1;
                } else {
                    unchanged += 1;
                }
            }
        }
    }
    let mut removed = Vec::new();
    for (k, w) in &cur {
        if !res.contains_key(k) {
            removed.push(rime_core::types::DiffLine {
                text: k.0.clone(),
                code: k.1.clone(),
                weight: *w,
            });
        }
    }

    FileDiff {
        source_path: path.to_path_buf(),
        summary: rime_core::types::DiffSummary {
            added: added.len() as u32,
            removed: removed.len() as u32,
            modified,
            unchanged,
        },
        added,
        removed,
    }
}

fn to_app_error(e: rime_core::types::AppError, ctx: &str) -> AppError {
    AppError {
        code: e.code,
        message: format!("{ctx}：{}", e.message),
        detail: e.detail,
        hint: e.hint,
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    const CUR: &str = "\
# Rime table
# coding: utf-8
...

一个\tig
美团\tmwtr\t100
骑行\tqixk
";

    const BACKUP: &str = "\
# Rime table
# coding: utf-8
...

一个\tig
美团\tmwtr\t50
新增\txbci
";

    #[test]
    fn diff_reports_added_removed_and_summary() {
        let d = diff_entries(std::path::Path::new("/tmp/x.txt"), CUR, BACKUP);

        // 备份里多出「新增」，少了「骑行」
        assert_eq!(d.added.len(), 1, "应检出 1 条新增");
        assert_eq!(d.added[0].text, "新增");
        assert_eq!(d.added[0].code, "xbci");

        assert_eq!(d.removed.len(), 1, "应检出 1 条删除");
        assert_eq!(d.removed[0].text, "骑行");

        // 美团权重 100 → 50 算修改；一个/一个未变
        assert_eq!(d.summary.modified, 1, "应检出 1 条修改");
        assert_eq!(d.summary.unchanged, 1, "应检出 1 条未变");
        assert_eq!(d.summary.added, 1);
        assert_eq!(d.summary.removed, 1);
    }

    #[test]
    fn diff_ignores_comments_blank_lines_and_terminator() {
        // 两侧只差注释/空行时不应产生任何词条差异
        let a = "# 注释A\n\n...\n\n一个\tig\n";
        let b = "# 注释B\n\n...\n\n一个\tig\n\n";
        let d = diff_entries(std::path::Path::new("/tmp/x.txt"), a, b);
        assert_eq!(d.added.len(), 0);
        assert_eq!(d.removed.len(), 0);
        assert_eq!(d.summary.modified, 0);
        assert_eq!(d.summary.unchanged, 1);
    }

    #[test]
    fn diff_identical_files_has_zero_changes() {
        let d = diff_entries(std::path::Path::new("/tmp/x.txt"), CUR, CUR);
        assert!(d.added.is_empty());
        assert!(d.removed.is_empty());
        assert_eq!(d.summary.modified, 0);
        assert_eq!(d.summary.unchanged, 3); // 一个、美团、骑行
    }
}
