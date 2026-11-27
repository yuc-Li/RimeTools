//! 备份原因索引。
//!
//! 为什么需要它：`.bak` 文件名里只有时间戳，没有「为什么备份」。
//! `rime_safety::list_backups` 从磁盘重建列表时只能给出默认原因，
//! 重启后「手动备份」会显示成「写前自动备份」，误导用户判断恢复风险。
//!
//! 做法：在备份目录维护一个 `index.json`（路径 → 原因 + 时间）。
//! 索引只是**辅助信息**：缺失/损坏时退化为磁盘列表，绝不影响备份本身可用性。

use rime_core::types::{BackupReason, BackupRef};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const INDEX_FILE: &str = "index.json";

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Index {
    #[serde(default)]
    entries: HashMap<String, Entry>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Entry {
    reason: BackupReason,
    created_at: i64,
}

fn index_path(dir: &Path) -> PathBuf {
    dir.join(INDEX_FILE)
}

fn load(dir: &Path) -> Index {
    std::fs::read_to_string(index_path(dir))
        .ok()
        .and_then(|t| serde_json::from_str::<Index>(&t).ok())
        .unwrap_or_default()
}

fn save(dir: &Path, index: &Index) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(index)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = index_path(dir).with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, index_path(dir))
}

/// 记录一批备份的原因。
pub fn record(dir: &Path, backups: &[BackupRef]) {
    if backups.is_empty() {
        return;
    }
    let mut index = load(dir);
    for b in backups {
        index.entries.insert(
            key(&b.path),
            Entry {
                reason: b.reason,
                created_at: b.created_at,
            },
        );
    }
    if let Err(e) = save(dir, &index) {
        // 索引写失败只影响原因显示，不影响备份可用性
        log::warn!("写入备份原因索引失败：{e}");
    }
}

/// 用索引修正磁盘列表里的原因（并清理已删除文件的陈旧条目）。
pub fn apply(dir: &Path, backups: &mut [BackupRef]) {
    let mut index = load(dir);
    let mut dirty = false;

    for b in backups.iter_mut() {
        if let Some(entry) = index.entries.get(&key(&b.path)) {
            b.reason = entry.reason;
        }
    }

    // 清理索引里已经没有对应文件的条目
    let existing: std::collections::HashSet<String> =
        backups.iter().map(|b| key(&b.path)).collect();
    index.entries.retain(|k, _| existing.contains(k));
    if !index.entries.is_empty() {
        // 只有当索引里确实保留了条目时才回写，避免把索引清空
        let before = index.entries.len();
        if before > 0 && save(dir, &index).is_err() {
            dirty = true;
        }
    }
    if dirty {
        log::debug!("备份索引回写失败（不影响使用）");
    }
}

fn key(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(path: &str, reason: BackupReason) -> BackupRef {
        BackupRef {
            id: path.to_owned(),
            path: PathBuf::from(path),
            source_path: PathBuf::from("/rime/custom_phrase_double.txt"),
            created_at: 1_700_000_000_000,
            size: 1024,
            reason,
        }
    }

    #[test]
    fn reason_survives_listing() {
        let dir = tempfile::tempdir().unwrap();
        let manual = sample(
            "/bak/custom_phrase_double.txt_20260101_120000.bak",
            BackupReason::Manual,
        );
        record(dir.path(), std::slice::from_ref(&manual));

        // 模拟从磁盘重建列表（reason 被重置为 BeforeWrite）
        let mut listed = vec![sample(
            "/bak/custom_phrase_double.txt_20260101_120000.bak",
            BackupReason::BeforeWrite,
        )];
        apply(dir.path(), &mut listed);

        assert_eq!(
            listed[0].reason,
            BackupReason::Manual,
            "手动备份的原因应被索引恢复"
        );
    }

    #[test]
    fn missing_index_keeps_disk_reason() {
        let dir = tempfile::tempdir().unwrap();
        let mut listed = vec![sample("/bak/a.bak", BackupReason::BeforeWrite)];
        apply(dir.path(), &mut listed);
        assert_eq!(listed[0].reason, BackupReason::BeforeWrite);
    }

    #[test]
    fn record_then_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        record(
            dir.path(),
            &[
                sample("/bak/x.bak", BackupReason::Manual),
                sample("/bak/y.bak", BackupReason::BeforeRestore),
            ],
        );
        let idx = load(dir.path());
        assert_eq!(idx.entries.len(), 2);
        assert_eq!(idx.entries["x.bak"].reason, BackupReason::Manual);
        assert_eq!(idx.entries["y.bak"].reason, BackupReason::BeforeRestore);
    }
}
