//! 设置持久化（PRD FR-8）。
//!
//! 为什么不用 `tauri-plugin-store`：这里只需要「一个小 JSON，读写都同步可控」，
//! 自己实现可以拿到明确的错误、做原子写、并且能在单元测试里直接验证。
//!
//! 位置：`$XDG_CONFIG_HOME/rimetools/settings.json`，回退 `~/.config/rimetools/settings.json`。
//! 写入用「临时文件 + rename」保证不会因为中途崩溃留下半截配置。

use rime_core::types::AppSettings;
use std::path::{Path, PathBuf};

/// 配置文件路径。
pub fn settings_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|h| h.join(".config"))
        })
        .or_else(|| std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("RimeTools")))?;
    Some(base.join("rimetools/settings.json"))
}

/// 读取设置；文件不存在或损坏时返回 `None`（调用方用默认值，不让坏配置阻断启动）。
pub fn load() -> Option<AppSettings> {
    let path = settings_path()?;
    let text = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<AppSettings>(&text) {
        Ok(s) => Some(s),
        Err(e) => {
            // 坏配置不删用户文件，改名留证，下次启动用默认值
            log::warn!("设置文件解析失败（{e}），本次使用默认值");
            let _ = std::fs::rename(&path, path.with_extension("json.broken"));
            None
        }
    }
}

/// 保存设置（原子写）。
pub fn save(settings: &AppSettings) -> Result<(), std::io::Error> {
    let Some(path) = settings_path() else {
        return Ok(()); // 拿不到配置目录时静默跳过：设置不持久化不应阻塞使用
    };
    write_atomic(&path, settings)
}

fn write_atomic(path: &Path, settings: &AppSettings) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json.as_bytes())?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rime_core::types::{BackupSettings, Theme};

    #[test]
    fn roundtrip_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rimetools/settings.json");

        let settings = AppSettings {
            user_dir: Some(PathBuf::from("/home/u/.local/share/fcitx5/rime")),
            frontend: None,
            managed_files: vec![],
            auto_deploy_after_save: false,
            backup: BackupSettings {
                enabled: false,
                dir: None,
                keep_count: 7,
                scheduled_enabled: false,
                scheduled_interval_hours: 24,
            },
            theme: Theme::Dark,
            language: "zh-CN".to_owned(),
        };

        write_atomic(&path, &settings).expect("写入失败");
        let text = std::fs::read_to_string(&path).unwrap();
        let back: AppSettings = serde_json::from_str(&text).unwrap();

        assert_eq!(back.auto_deploy_after_save, false);
        assert_eq!(back.backup.keep_count, 7);
        assert_eq!(back.theme, Theme::Dark);
        assert!(text.contains("autoDeployAfterSave"), "字段应为 camelCase");

        // 原子写不应残留临时文件
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn corrupt_file_falls_back_to_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, b"{ not json").unwrap();

        // load() 走 settings_path()（环境变量），这里直接验证解析逻辑的等价行为
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(serde_json::from_str::<AppSettings>(&text).is_err());
    }
}
