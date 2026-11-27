//! `build/` 部署产物快照与差异判定。
//!
//! PRD FR-5.5：部署是否生效**不能只看命令退出码**，要看 `build/` 产物的
//! mtime + size 是否真的变化（Rime 部署进程异步写 build/，D-Bus 调用立即返回）。
//!
//! 快照只读，不创建/修改任何文件。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 单个产物文件的特征：修改时间 + 大小。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStamp {
    pub mtime: Option<SystemTime>,
    pub size: u64,
}

impl FileStamp {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        Self {
            mtime: metadata.modified().ok(),
            size: metadata.len(),
        }
    }
}

/// `build/` 目录（递归）的产物快照。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildSnapshot {
    files: BTreeMap<PathBuf, FileStamp>,
}

/// 两次快照的差异（路径相对于快照根目录）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildDelta {
    pub added: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
    pub modified: Vec<PathBuf>,
}

impl BuildDelta {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }

    pub fn total(&self) -> usize {
        self.added.len() + self.removed.len() + self.modified.len()
    }

    /// 面向用户/日志的一行摘要。
    pub fn summary(&self) -> String {
        format!(
            "新增 {}，修改 {}，删除 {}",
            self.added.len(),
            self.modified.len(),
            self.removed.len()
        )
    }
}

/// 遍历目录时的硬上限（防止异常目录拖慢部署/校验）。
const MAX_ENTRIES: usize = 50_000;
/// 遍历深度上限。
const MAX_DEPTH: usize = 8;

impl BuildSnapshot {
    /// 读取 `dir` 下的全部文件。目录不存在返回空快照（首次部署前的正常状态）。
    pub fn capture(dir: &Path) -> Self {
        let mut files = BTreeMap::new();
        let mut stack: Vec<(PathBuf, usize)> = vec![(dir.to_path_buf(), 0)];
        let mut visited = 0usize;
        while let Some((current, depth)) = stack.pop() {
            if depth > MAX_DEPTH || visited >= MAX_ENTRIES {
                continue;
            }
            let entries = match std::fs::read_dir(&current) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                visited += 1;
                if visited > MAX_ENTRIES {
                    break;
                }
                let path = entry.path();
                let file_type = match entry.file_type() {
                    Ok(file_type) => file_type,
                    Err(_) => continue,
                };
                if file_type.is_dir() {
                    stack.push((path, depth + 1));
                } else if file_type.is_file() {
                    let metadata = match entry.metadata() {
                        Ok(metadata) => metadata,
                        Err(_) => continue,
                    };
                    let relative = path
                        .strip_prefix(dir)
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|_| path.clone());
                    files.insert(relative, FileStamp::from_metadata(&metadata));
                }
            }
        }
        Self { files }
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn files(&self) -> &BTreeMap<PathBuf, FileStamp> {
        &self.files
    }

    /// `self` 为部署前快照，`after` 为部署后快照。
    pub fn diff(&self, after: &Self) -> BuildDelta {
        let mut delta = BuildDelta::default();
        for (path, stamp) in &after.files {
            match self.files.get(path) {
                None => delta.added.push(path.clone()),
                Some(before) if before != stamp => delta.modified.push(path.clone()),
                Some(_) => {}
            }
        }
        for path in self.files.keys() {
            if !after.files.contains_key(path) {
                delta.removed.push(path.clone());
            }
        }
        delta
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn write(dir: &Path, name: &str, content: &str) {
        std::fs::create_dir_all(dir).expect("mkdir");
        std::fs::write(dir.join(name), content).expect("write");
    }

    #[test]
    fn missing_directory_yields_empty_snapshot() {
        let snapshot = BuildSnapshot::capture(Path::new("/nonexistent/rime-build-dir"));
        assert!(snapshot.is_empty());
    }

    #[test]
    fn detects_added_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let build = dir.path().join("build");
        std::fs::create_dir_all(&build).expect("mkdir");
        let before = BuildSnapshot::capture(&build);
        write(&build, "default.yaml", "schema: xhup\n");
        let after = BuildSnapshot::capture(&build);
        let delta = before.diff(&after);
        assert_eq!(delta.added, vec![PathBuf::from("default.yaml")]);
        assert!(delta.removed.is_empty());
        assert!(delta.modified.is_empty());
        assert!(before.diff(&before).is_empty());
    }

    #[test]
    fn detects_modified_file_content_size() {
        let dir = tempfile::tempdir().expect("tempdir");
        let build = dir.path().join("build");
        write(&build, "xhup.table.bin", "aaaa");
        let before = BuildSnapshot::capture(&build);
        write(&build, "xhup.table.bin", "aaaa-bbbb");
        let after = BuildSnapshot::capture(&build);
        let delta = before.diff(&after);
        assert_eq!(delta.modified, vec![PathBuf::from("xhup.table.bin")]);
        assert_eq!(delta.total(), 1);
    }

    #[test]
    fn detects_removed_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let build = dir.path().join("build");
        write(&build, "old.bin", "x");
        let before = BuildSnapshot::capture(&build);
        std::fs::remove_file(build.join("old.bin")).expect("remove");
        let after = BuildSnapshot::capture(&build);
        let delta = before.diff(&after);
        assert_eq!(delta.removed, vec![PathBuf::from("old.bin")]);
    }

    #[test]
    fn detects_mtime_only_change() {
        let dir = tempfile::tempdir().expect("tempdir");
        let build = dir.path().join("build");
        write(&build, "same-size.bin", "abcd");
        let before = BuildSnapshot::capture(&build);
        // 内容长度不变，仅 touch 改变 mtime（模拟 Rime 重写同一产物）。
        std::thread::sleep(Duration::from_millis(20));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(build.join("same-size.bin"))
            .expect("open");
        file.set_len(0).expect("truncate");
        drop(file);
        write(&build, "same-size.bin", "wxyz");
        let after = BuildSnapshot::capture(&build);
        let delta = before.diff(&after);
        assert_eq!(delta.modified, vec![PathBuf::from("same-size.bin")]);
    }

    #[test]
    fn summary_is_stable() {
        let delta = BuildDelta {
            added: vec![PathBuf::from("a")],
            removed: vec![],
            modified: vec![PathBuf::from("b"), PathBuf::from("c")],
        };
        assert_eq!(delta.summary(), "新增 1，修改 2，删除 0");
    }
}
