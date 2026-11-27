//! 隔离的 Rime 预览目录（**绝不写入用户真实 Rime 目录**）。
//!
//! 候选预览必须跑在独立 librime 会话里，而 librime 的 `user_data_dir` 是 setup 时固定的。
//! 本模块把预览需要的文件复制到 `tempfile` 临时目录，在那里部署、建会话：
//!
//! ```text
//! <temp>/                       ← TempDir，Drop 时整棵删除
//! ├── shared/                   ← shared_data_dir（只读预设 + 方案文件的副本）
//! │   ├── default.yaml          ← 从系统共享目录复制（或调用方提供）
//! │   └── <schema>.schema.yaml …
//! ├── user/                     ← user_data_dir（build/、*.custom.yaml 都落在这里）
//! │   ├── default.custom.yaml   ← 只启用目标方案
//! │   ├── <schema>.custom.yaml  ← 关闭 speller/auto_select（xhup 4 码顶屏）
//! │   └── build/                ← librime 部署产物
//! └── logs/                     ← 隔离日志
//! ```
//!
//! 注意：`speller/auto_select` 只改了**临时副本**里的方案 patch；用户真实方案文件、
//! `~/.local/share/fcitx5/rime` 与 Weasel 目录都不会被打开写入。

use std::fs;
use std::path::{Component, Path, PathBuf};

use tempfile::TempDir;

use crate::error::FfiError;

/// 从系统共享目录复制的 Rime 预设文件（`import_preset: default` 等依赖）。
const PRESET_FILES: &[&str] = &[
    "default.yaml",
    "symbols.yaml",
    "punctuation.yaml",
    "key_bindings.yaml",
    "essay.txt",
];

/// 需要整目录复制的预设资产（存在才复制）。
const PRESET_DIRS: &[&str] = &["opencc"];

/// 预览目录里的一个文件来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HomeContent {
    /// 复制已有文件（用户真实方案/码表 → 临时副本，只读来源）。
    Copy { source: PathBuf, dest: PathBuf },
    /// 直接写入内容（测试夹具、或工具生成的 patch）。
    Inline { dest: PathBuf, content: String },
}

impl HomeContent {
    pub fn copy(source: impl Into<PathBuf>, dest: impl Into<PathBuf>) -> Self {
        Self::Copy {
            source: source.into(),
            dest: dest.into(),
        }
    }

    pub fn inline(dest: impl Into<PathBuf>, content: impl Into<String>) -> Self {
        Self::Inline {
            dest: dest.into(),
            content: content.into(),
        }
    }
}

/// [`IsolatedRimeHome::prepare`] 的参数。
#[derive(Debug, Clone)]
pub struct HomeSpec {
    /// 目标方案 id（同时决定 `<schema>.custom.yaml` 的文件名）。
    pub schema_id: String,
    /// 系统 Rime 共享数据目录（如 `/usr/share/rime-data`）。复制其中的预设文件。
    pub preset_source: Option<PathBuf>,
    /// 额外复制进 `shared/` 的文件。
    pub shared_files: Vec<HomeContent>,
    /// 额外复制进 `user/` 的文件（用户方案、码表、patch 等）。
    pub user_files: Vec<HomeContent>,
    /// 在方案副本里关闭 `speller/auto_select`（默认开启处理）。xhup 4 码顶屏的根治办法。
    pub disable_auto_select: bool,
    /// 覆盖方案副本里的 `menu/page_size`（N）。
    pub page_size: Option<u32>,
    /// 最小日志级别（0=INFO 1=WARNING 2=ERROR 3=FATAL）。
    pub min_log_level: i32,
    /// librime 的 application name（`rime.<name>`，影响日志文件清理）。
    pub app_name: String,
}

impl HomeSpec {
    /// 默认：关闭 auto_select（预览会话要能稳定读候选）。
    pub fn new(schema_id: impl Into<String>) -> Self {
        Self {
            schema_id: schema_id.into(),
            preset_source: None,
            shared_files: Vec::new(),
            user_files: Vec::new(),
            disable_auto_select: true,
            page_size: None,
            min_log_level: 1,
            app_name: "rimetools-preview".to_owned(),
        }
    }

    pub fn preset_source(mut self, path: impl Into<PathBuf>) -> Self {
        self.preset_source = Some(path.into());
        self
    }

    pub fn shared_file(mut self, file: HomeContent) -> Self {
        self.shared_files.push(file);
        self
    }

    pub fn user_file(mut self, file: HomeContent) -> Self {
        self.user_files.push(file);
        self
    }

    pub fn page_size(mut self, page_size: u32) -> Self {
        self.page_size = Some(page_size);
        self
    }

    /// 保留方案原本的 `speller/auto_select`（仅用于复现“4 码自动上屏”实验）。
    pub fn keep_auto_select(mut self) -> Self {
        self.disable_auto_select = false;
        self
    }
}

/// 准备好的隔离 Rime 目录。
#[derive(Debug)]
pub struct IsolatedRimeHome {
    root: TempDir,
    schema_id: String,
    shared_dir: PathBuf,
    user_dir: PathBuf,
    log_dir: PathBuf,
    min_log_level: i32,
    app_name: String,
}

impl IsolatedRimeHome {
    /// 创建临时目录并复制文件、写 patch。
    pub fn prepare(spec: HomeSpec) -> Result<Self, FfiError> {
        if spec.schema_id.trim().is_empty() {
            return Err(FfiError::Home {
                reason: "schema_id 不能为空".to_owned(),
            });
        }
        if spec.schema_id.contains(['/', '\\', '\0']) {
            return Err(FfiError::Home {
                reason: format!("schema_id 不能包含路径分隔符：{:?}", spec.schema_id),
            });
        }

        let root = tempfile::Builder::new()
            .prefix("rimetools-rime-preview-")
            .tempdir()
            .map_err(|err| FfiError::Home {
                reason: format!("无法创建临时目录：{err}"),
            })?;
        let base = root.path().to_path_buf();
        let shared_dir = base.join("shared");
        let user_dir = base.join("user");
        let log_dir = base.join("logs");
        for dir in [&shared_dir, &user_dir, &log_dir] {
            fs::create_dir_all(dir).map_err(|err| FfiError::Home {
                reason: format!("无法创建隔离目录 {}：{err}", dir.display()),
            })?;
        }

        if let Some(preset_source) = spec.preset_source.as_ref() {
            copy_presets(preset_source, &shared_dir)?;
        }

        for file in &spec.shared_files {
            materialize(file, &shared_dir)?;
        }
        for file in &spec.user_files {
            materialize(file, &user_dir)?;
        }

        // 只启用目标方案：避免部署时把系统里其余方案也编译一遍（慢，且没必要）。
        let mut default_patch = String::from(
            "# 由 RimeTools 生成：隔离预览会话使用，仅存在于临时目录\npatch:\n  schema_list:\n",
        );
        default_patch.push_str(&format!("    - schema: {}\n", spec.schema_id));
        write_text(&user_dir.join("default.custom.yaml"), &default_patch)?;

        // xhup 4 码 auto_select 的关键处理：只在临时副本里关掉它。
        let mut schema_patch =
            String::from("# 由 RimeTools 生成：预览会话关闭自动上屏，仅存在于临时目录\npatch:\n");
        if spec.disable_auto_select {
            schema_patch.push_str("  \"speller/auto_select\": false\n");
        }
        if let Some(page_size) = spec.page_size {
            schema_patch.push_str(&format!("  \"menu/page_size\": {page_size}\n"));
        }
        if spec.disable_auto_select || spec.page_size.is_some() {
            write_text(
                &user_dir.join(format!("{}.custom.yaml", spec.schema_id)),
                &schema_patch,
            )?;
        }

        Ok(Self {
            root,
            schema_id: spec.schema_id,
            shared_dir,
            user_dir,
            log_dir,
            min_log_level: spec.min_log_level,
            app_name: spec.app_name,
        })
    }

    pub fn schema_id(&self) -> &str {
        &self.schema_id
    }

    pub fn root(&self) -> &Path {
        self.root.path()
    }

    pub fn shared_dir(&self) -> &Path {
        &self.shared_dir
    }

    pub fn user_dir(&self) -> &Path {
        &self.user_dir
    }

    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }

    pub fn min_log_level(&self) -> i32 {
        self.min_log_level
    }

    pub fn app_name(&self) -> &str {
        &self.app_name
    }

    /// librime 的 staging 目录（build 产物，默认 `${user_data_dir}/build`）。
    pub fn build_dir(&self) -> PathBuf {
        self.user_dir.join("build")
    }
}

fn copy_presets(source: &Path, shared_dir: &Path) -> Result<(), FfiError> {
    if !source.is_dir() {
        // 预设目录不存在不算致命：测试夹具可以自带全部文件。
        tracing::debug!(path = %source.display(), "共享目录不存在，跳过预设复制");
        return Ok(());
    }
    for name in PRESET_FILES {
        let src = source.join(name);
        if src.is_file() {
            copy_file(&src, &shared_dir.join(name))?;
        }
    }
    for name in PRESET_DIRS {
        let src = source.join(name);
        if src.is_dir() {
            copy_dir(&src, &shared_dir.join(name))?;
        }
    }
    Ok(())
}

fn materialize(file: &HomeContent, base: &Path) -> Result<(), FfiError> {
    match file {
        HomeContent::Copy { source, dest } => {
            let target = safe_join(base, dest)?;
            copy_file(source, &target)
        }
        HomeContent::Inline { dest, content } => {
            let target = safe_join(base, dest)?;
            write_text(&target, content)
        }
    }
}

/// 把相对路径安全地拼到 `base` 下：拒绝绝对路径、`..`、Windows 前缀。
fn safe_join(base: &Path, relative: &Path) -> Result<PathBuf, FfiError> {
    let mut out = base.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(name) => out.push(name),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(FfiError::Home {
                    reason: format!("非法的相对路径：{}", relative.display()),
                });
            }
        }
    }
    if out == base {
        return Err(FfiError::Home {
            reason: "文件目标路径不能为空".to_owned(),
        });
    }
    Ok(out)
}

fn copy_file(source: &Path, target: &Path) -> Result<(), FfiError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|err| FfiError::Home {
            reason: format!("无法创建目录 {}：{err}", parent.display()),
        })?;
    }
    fs::copy(source, target).map_err(|err| FfiError::Home {
        reason: format!(
            "复制 {} → {} 失败：{err}",
            source.display(),
            target.display()
        ),
    })?;
    Ok(())
}

fn copy_dir(source: &Path, target: &Path) -> Result<(), FfiError> {
    fs::create_dir_all(target).map_err(|err| FfiError::Home {
        reason: format!("无法创建目录 {}：{err}", target.display()),
    })?;
    let entries = fs::read_dir(source).map_err(|err| FfiError::Home {
        reason: format!("读取目录 {} 失败：{err}", source.display()),
    })?;
    for entry in entries {
        let entry = entry.map_err(|err| FfiError::Home {
            reason: format!("读取目录项失败：{err}"),
        })?;
        let ty = entry.file_type().map_err(|err| FfiError::Home {
            reason: format!("读取目录项类型失败：{err}"),
        })?;
        let dest = target.join(entry.file_name());
        if ty.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else if ty.is_file() {
            copy_file(&entry.path(), &dest)?;
        }
    }
    Ok(())
}

fn write_text(target: &Path, content: &str) -> Result<(), FfiError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|err| FfiError::Home {
            reason: format!("无法创建目录 {}：{err}", parent.display()),
        })?;
    }
    fs::write(target, content).map_err(|err| FfiError::Home {
        reason: format!("写入 {} 失败：{err}", target.display()),
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_source(name: &str, content: &str) -> PathBuf {
        let dir = tempfile::tempdir().unwrap();
        // 保持 TempDir 存活：这里直接泄漏（测试进程结束回收），换取简单。
        let dir = Box::leak(Box::new(dir));
        let path = dir.path().join(name);
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn prepare_creates_isolated_dirs_and_patches() {
        let source = temp_source("demo.schema.yaml", "schema:\n  schema_id: demo\n");
        let home = IsolatedRimeHome::prepare(
            HomeSpec::new("demo")
                .preset_source("/nonexistent/rime-presets")
                .shared_file(HomeContent::copy(&source, "demo.schema.yaml"))
                .user_file(HomeContent::inline(
                    "demo.dict.yaml",
                    "---\nname: demo\n...\n",
                )),
        )
        .unwrap();

        assert!(home.root().is_dir());
        assert!(home.shared_dir().join("demo.schema.yaml").is_file());
        assert!(home.user_dir().join("demo.dict.yaml").is_file());

        let default_patch =
            fs::read_to_string(home.user_dir().join("default.custom.yaml")).unwrap();
        assert!(default_patch.contains("schema: demo"));
        assert!(default_patch.contains("patch:"));

        let schema_patch = fs::read_to_string(home.user_dir().join("demo.custom.yaml")).unwrap();
        assert!(schema_patch.contains("speller/auto_select"));
        assert!(schema_patch.contains("false"));

        // 全部产物都在临时根目录内。
        assert!(home.shared_dir().starts_with(home.root()));
        assert!(home.user_dir().starts_with(home.root()));
        assert_eq!(home.build_dir(), home.user_dir().join("build"));
    }

    #[test]
    fn prepare_without_auto_select_patch_keeps_original_behavior() {
        let home = IsolatedRimeHome::prepare(
            HomeSpec::new("xhup")
                .keep_auto_select()
                .user_file(HomeContent::inline("xhup.schema.yaml", "schema: {}\n")),
        )
        .unwrap();
        assert!(!home.user_dir().join("xhup.custom.yaml").exists());
    }

    #[test]
    fn prepare_page_size_override_writes_schema_patch() {
        let home = IsolatedRimeHome::prepare(HomeSpec::new("xhup").page_size(7).keep_auto_select())
            .unwrap();
        let patch = fs::read_to_string(home.user_dir().join("xhup.custom.yaml")).unwrap();
        assert!(patch.contains("menu/page_size"));
        assert!(patch.contains('7'));
        assert!(!patch.contains("auto_select"));
    }

    #[test]
    fn preset_files_are_copied_when_present() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("default.yaml"), "schema_list: []\n").unwrap();
        fs::write(dir.path().join("symbols.yaml"), "{}\n").unwrap();
        fs::create_dir(dir.path().join("opencc")).unwrap();
        fs::write(dir.path().join("opencc/s2t.json"), "{}\n").unwrap();

        let home =
            IsolatedRimeHome::prepare(HomeSpec::new("demo").preset_source(dir.path())).unwrap();
        // 预设照抄，但 default.custom.yaml 由我们覆盖为单方案列表。
        assert!(home.shared_dir().join("default.yaml").is_file());
        assert!(home.shared_dir().join("symbols.yaml").is_file());
        assert!(home.shared_dir().join("opencc/s2t.json").is_file());
        let patch = fs::read_to_string(home.user_dir().join("default.custom.yaml")).unwrap();
        assert!(patch.contains("schema: demo"));
    }

    #[test]
    fn path_traversal_is_rejected() {
        let err = IsolatedRimeHome::prepare(
            HomeSpec::new("demo").user_file(HomeContent::inline("../escape.yaml", "x")),
        )
        .expect_err("必须拒绝 .. 路径");
        assert!(matches!(err, FfiError::Home { .. }));

        let err = IsolatedRimeHome::prepare(
            HomeSpec::new("demo").user_file(HomeContent::inline("/etc/passwd", "x")),
        )
        .expect_err("必须拒绝绝对路径");
        assert!(matches!(err, FfiError::Home { .. }));
    }

    #[test]
    fn invalid_schema_id_is_rejected() {
        for schema_id in ["", "   ", "../xhup", "a/b"] {
            let err = IsolatedRimeHome::prepare(HomeSpec::new(schema_id))
                .expect_err("非法 schema_id 必须报错");
            assert!(
                matches!(err, FfiError::Home { .. }),
                "schema_id={schema_id:?}"
            );
        }
    }

    #[test]
    fn missing_source_file_is_reported() {
        let err = IsolatedRimeHome::prepare(
            HomeSpec::new("demo")
                .user_file(HomeContent::copy("/nonexistent/file.yaml", "file.yaml")),
        )
        .expect_err("复制不存在的文件必须报错");
        match err {
            FfiError::Home { reason } => assert!(reason.contains("复制")),
            other => panic!("期望 Home 错误，实际 {other:?}"),
        }
    }
}
