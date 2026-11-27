//! 方案级配置命令：快速上屏 / 最大码长 / 词表挂载（M2 设置增强）。
//!
//! 写入位置与安全性：
//! - speller 配置 → `<schema>.custom.yaml` 的 `patch:`（Rime 官方覆盖机制，
//!   方案文件升级不会被冲掉）；
//! - 词表挂载 → 主词典的 `import_tables`（Rime 没有词典补丁机制，只能改原文件），
//!   仅切换行首 `# ` 前缀，其余字符逐字保留；
//! - 两者都走「写前备份 + 指纹校验 + 原子写」，文件期间被外部改动会拒绝覆盖。

use crate::state::AppState;
use rime_core::types::{AppError, ErrorCode, Fingerprint, SchemeInfo};
use rime_platform::scheme_config::{self, MountEntry, SchemeConfigSnapshot, SpellerConfig};
use rime_safety::{save_file, WriteSafetyOptions};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::State;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MountToggle {
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemeConfigUpdate {
    pub schema_id: String,
    /// 不传表示不改
    pub auto_select: Option<bool>,
    pub max_code_length: Option<u32>,
    /// 不传表示不改
    pub mounts: Option<Vec<MountToggle>>,
    #[serde(default)]
    pub deploy_after: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemeConfigUpdateResult {
    pub snapshot: SchemeConfigSnapshot,
    /// 实际写入了哪些文件（便于界面提示「需要重新部署」）
    pub written: Vec<String>,
    pub deploy: Option<rime_core::types::DeployResult>,
}

fn user_dir(state: &AppState) -> PathBuf {
    if let Some(dir) = state.settings().user_dir {
        return dir;
    }
    rime_platform::detect_frontends()
        .into_iter()
        .next()
        .map(|f| f.user_dir)
        .unwrap_or_default()
}

fn resolve_scheme(state: &AppState, schema_id: Option<&str>) -> Result<SchemeInfo, AppError> {
    if let Some(id) = schema_id {
        if let Some(s) = state.scheme(id) {
            return Ok(s);
        }
    }
    state.active_scheme().ok_or_else(|| {
        AppError::new(
            ErrorCode::SchemeNotDetected,
            "未检测到可用方案，请先在设置里指定 Rime 用户目录",
        )
    })
}

fn schema_path(dir: &Path, schema_id: &str) -> PathBuf {
    let yaml = dir.join(format!("{schema_id}.schema.yaml"));
    if yaml.is_file() {
        yaml
    } else {
        dir.join(format!("{schema_id}.schema.yml"))
    }
}

fn custom_path(dir: &Path, schema_id: &str) -> PathBuf {
    dir.join(format!("{schema_id}.custom.yaml"))
}

/// 主词典路径（`import_tables` 所在文件）。
fn primary_dict_path(dir: &Path, scheme: &SchemeInfo) -> Option<PathBuf> {
    let raw = rime_platform::parse_schema(&schema_path(dir, &scheme.schema_id)).ok()?;
    let primary = raw.primary_dictionary?;
    let path = scheme_config::dict_path_for(dir, &primary);
    path.is_file().then_some(path)
}

/// 快速统计词条数（超过 5MB 的词典不解析，避免设置页卡顿）。
fn entry_count(path: &Path) -> Option<u32> {
    let size = std::fs::metadata(path).ok()?.len();
    if size > 5 * 1024 * 1024 {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let parsed =
        rime_core::ParsedDict::parse(&text, rime_core::types::FileFormat::RimeDictYaml).ok()?;
    Some(parsed.entry_count() as u32)
}

fn build_snapshot(state: &AppState, scheme: &SchemeInfo) -> Result<SchemeConfigSnapshot, AppError> {
    let dir = user_dir(state);
    let schema_file = schema_path(&dir, &scheme.schema_id);
    let custom_file = custom_path(&dir, &scheme.schema_id);

    let schema_text = std::fs::read_to_string(&schema_file).map_err(|e| {
        AppError::new(
            ErrorCode::FileNotFound,
            format!("读取方案文件失败（{}）：{e}", schema_file.display()),
        )
    })?;

    let mut speller: SpellerConfig = scheme_config::read_speller_from_schema(&schema_text);
    let custom_text = std::fs::read_to_string(&custom_file).ok();
    let custom_overlay = custom_text
        .as_deref()
        .map(scheme_config::read_speller_from_custom)
        .unwrap_or_default();

    let mut source = "schema".to_owned();
    if custom_overlay.auto_select.is_some() {
        speller.auto_select = custom_overlay.auto_select;
        source = "custom".to_owned();
    }
    if custom_overlay.max_code_length.is_some() {
        speller.max_code_length = custom_overlay.max_code_length;
        source = "custom".to_owned();
    }
    if custom_overlay.auto_select_pattern.is_some() {
        speller.auto_select_pattern = custom_overlay.auto_select_pattern;
    }
    if custom_overlay.auto_clear.is_some() {
        speller.auto_clear = custom_overlay.auto_clear;
    }

    // 词表挂载
    let dict_file = primary_dict_path(&dir, scheme);
    let mut mounts: Vec<MountEntry> = Vec::new();
    if let Some(dict) = &dict_file {
        if let Ok(text) = std::fs::read_to_string(dict) {
            for (name, enabled, comment) in scheme_config::parse_mounts(&text) {
                let path = scheme_config::dict_path_for(&dir, &name);
                let resolved = path.is_file().then(|| path.clone());
                let is_user_table = name.ends_with(".user")
                    || name.contains(".user.")
                    || name.ends_with(".user.top");
                mounts.push(MountEntry {
                    entry_count: resolved.as_deref().and_then(entry_count),
                    is_user_table,
                    path: resolved.map(|p| p.to_string_lossy().to_string()),
                    name,
                    enabled,
                    comment,
                });
            }
        }
    }

    Ok(SchemeConfigSnapshot {
        schema_id: scheme.schema_id.clone(),
        schema_name: scheme.name.clone(),
        schema_path: schema_file.to_string_lossy().to_string(),
        custom_path: custom_file.to_string_lossy().to_string(),
        custom_exists: custom_file.is_file(),
        speller,
        speller_source: source,
        dict_path: dict_file.map(|p| p.to_string_lossy().to_string()),
        mounts,
        deploy_hint: "改完配置后需要重新部署才会生效".to_owned(),
    })
}

fn safety_options(state: &AppState) -> WriteSafetyOptions {
    let settings = state.settings();
    WriteSafetyOptions {
        backup_dir: Some(state.backup_dir()),
        fingerprint_policy: Default::default(),
        validate_staged_utf8: true,
        prune_keep_count: settings
            .backup
            .enabled
            .then_some(settings.backup.keep_count),
    }
}

fn current_fingerprint(path: &Path) -> Result<Fingerprint, AppError> {
    rime_core::fingerprint_file(path).map_err(|e| {
        AppError::new(
            ErrorCode::FileNotFound,
            format!("读取文件指纹失败（{}）：{e}", path.display()),
        )
    })
}

/// 原子创建新文件（目标不存在时用）。
fn create_file_atomic(path: &Path, content: &str) -> Result<(), AppError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if !parent.as_os_str().is_empty() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::new(ErrorCode::FileNotWritable, format!("无法创建目录：{e}")))?;
    }
    let tmp = path.with_file_name(format!(
        ".{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    std::fs::write(&tmp, content.as_bytes())
        .map_err(|e| AppError::new(ErrorCode::FileNotWritable, format!("写入失败：{e}")))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        AppError::new(ErrorCode::AtomicWriteFailed, format!("提交失败：{e}"))
    })?;
    Ok(())
}

/// 读取当前配置快照。
#[tauri::command]
pub fn scheme_config_get(
    schema_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<SchemeConfigSnapshot, AppError> {
    let scheme = resolve_scheme(&state, schema_id.as_deref())?;
    build_snapshot(&state, &scheme)
}

/// 更新配置：speller 写 custom.yaml，词表挂载改 import_tables。
#[tauri::command]
pub fn scheme_config_update(
    request: SchemeConfigUpdate,
    state: State<'_, AppState>,
) -> Result<SchemeConfigUpdateResult, AppError> {
    let scheme = resolve_scheme(&state, Some(&request.schema_id))?;
    let dir = user_dir(&state);
    let mut written: Vec<String> = Vec::new();

    // ── 1) speller 配置 → custom.yaml ──
    if request.auto_select.is_some() || request.max_code_length.is_some() {
        let path = custom_path(&dir, &scheme.schema_id);
        let existing = std::fs::read_to_string(&path).ok();
        let next = scheme_config::patch_custom_yaml(
            existing.as_deref(),
            request.auto_select,
            request.max_code_length,
        );

        if existing.as_deref() != Some(next.as_str()) {
            match existing {
                Some(_) => {
                    let expected = current_fingerprint(&path)?;
                    save_file(&path, &expected, next.as_bytes(), &safety_options(&state))?;
                }
                None => create_file_atomic(&path, &next)?,
            }
            written.push(path.to_string_lossy().to_string());
        }
    }

    // ── 2) 词表挂载 → import_tables ──
    if let Some(toggles) = &request.mounts {
        if let Some(dict) = primary_dict_path(&dir, &scheme) {
            let text = std::fs::read_to_string(&dict).map_err(|e| {
                AppError::new(
                    ErrorCode::FileNotFound,
                    format!("读取词表索引失败（{}）：{e}", dict.display()),
                )
            })?;
            let pairs: Vec<(String, bool)> = toggles
                .iter()
                .map(|t| (t.name.clone(), t.enabled))
                .collect();
            if let Some(next) = scheme_config::toggle_mounts(&text, &pairs) {
                let expected = current_fingerprint(&dict)?;
                save_file(&dict, &expected, next.as_bytes(), &safety_options(&state))?;
                written.push(dict.to_string_lossy().to_string());
            }
        }
    }

    // ── 3) 可选部署 ──
    let deploy = if request.deploy_after && !written.is_empty() {
        Some(crate::commands::dict::trigger_deploy(
            &state,
            rime_core::types::DeployTrigger::Manual,
        )?)
    } else {
        None
    };

    Ok(SchemeConfigUpdateResult {
        snapshot: build_snapshot(&state, &scheme)?,
        written,
        deploy,
    })
}
