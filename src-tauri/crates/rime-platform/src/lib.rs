//! 平台适配与环境检测（PRD FR-1）。
//!
//! 本 crate 是唯一允许出现「平台分支」的地方：其余 crate 必须与平台无关。
//! Windows（Weasel）适配器在本阶段仅保留接口与路径约定，不做实测（用户决策：
//! 开发验证以 Linux 本机为准）。

use rime_core::types::{
    ColumnKind, DeployCapability, FileFormat, FileId, FileRole, FrontendInfo, FrontendKind,
    ManagedFileRef, SchemeInfo,
};
use std::path::{Path, PathBuf};

mod schema;

pub mod scheme_config;
pub mod sources;
pub mod symbols;

pub use schema::{parse_schema, SchemaParseError, SchemaRaw};

/// fcitx5 用户目录（固定位置）。
pub fn fcitx5_user_dir() -> PathBuf {
    home_dir().join(".local/share/fcitx5/rime")
}

/// fcitx5 共享数据目录（发行版默认位置）。
pub fn fcitx5_shared_dir() -> PathBuf {
    PathBuf::from("/usr/share/rime-data")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// 探测本机可用的 Rime 前端。
///
/// 只返回**确实存在**的前端；多前端共存时全部返回，由上层让用户选择（FR-1.6），不静默猜测。
pub fn detect_frontends() -> Vec<FrontendInfo> {
    let mut out = Vec::new();

    let fcitx5_dir = fcitx5_user_dir();
    if fcitx5_dir.is_dir() {
        out.push(FrontendInfo {
            kind: FrontendKind::Fcitx5Rime,
            display_name: "fcitx5-rime".to_owned(),
            version: read_distribution_version(&fcitx5_dir),
            librime_version: read_rime_version(&fcitx5_dir),
            user_dir: fcitx5_dir,
            shared_dir: Some(fcitx5_shared_dir()),
            deploy_capability: fcitx5_deploy_capability(),
        });
    }

    // Windows：%APPDATA%\Rime。本阶段仅保留探测逻辑，不做真机验证。
    #[cfg(windows)]
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let dir = PathBuf::from(appdata).join("Rime");
        if dir.is_dir() {
            out.push(FrontendInfo {
                kind: FrontendKind::Weasel,
                display_name: "小狼毫".to_owned(),
                version: None,
                librime_version: read_rime_version(&dir),
                user_dir: dir,
                shared_dir: None,
                deploy_capability: DeployCapability {
                    supported: true,
                    method: "weasel-deployer".to_owned(),
                    note: Some("调用 WeaselDeployer.exe /deploy".to_owned()),
                },
            });
        }
    }

    out
}

/// fcitx5 部署能力描述。
///
/// 结论来源：本机实测 + 社区实测记录。`fcitx5-remote -r` 已确认**无效**
/// （只重读 fcitx5 全局配置，不会让 Rime 重载 build/），因此不作为方法。
pub fn fcitx5_deploy_capability() -> DeployCapability {
    DeployCapability {
        supported: true,
        method: "dbus-setconfig".to_owned(),
        note: Some(
            "通过 D-Bus 调用 fcitx5 的 rime 部署接口；失败时回退到 rime_deployer 预构建".to_owned(),
        ),
    }
}

/// 从 installation.yaml 读取前端发行版版本。
fn read_distribution_version(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("installation.yaml")).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("distribution_version:") {
            return Some(rest.trim().trim_matches('"').to_owned());
        }
    }
    None
}

/// 从 installation.yaml 读取 librime 版本。
fn read_rime_version(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("installation.yaml")).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("rime_version:") {
            return Some(rest.trim().trim_matches('"').to_owned());
        }
    }
    None
}

/// 扫描用户目录中的全部方案，并识别各自的个人词条入口。
pub fn detect_schemes(user_dir: &Path) -> Vec<SchemeInfo> {
    let active = read_active_schema_ids(user_dir);
    let mut out = Vec::new();

    let Ok(entries) = std::fs::read_dir(user_dir) else {
        return out;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".schema.yaml") {
            continue;
        }
        // 跳过 *.custom.yaml 生成的模板等
        if name.ends_with(".custom.schema.yaml") {
            continue;
        }
        let Ok(raw) = schema::parse_schema(&path) else {
            continue;
        };
        let schema_id = raw.schema_id.clone();
        let managed = detect_managed_files(user_dir, &raw);
        out.push(SchemeInfo {
            is_active: active.contains(&schema_id),
            is_current: false, // 由调用方用 D-Bus 查询后填充
            schema_id,
            name: raw.name,
            version: raw.version,
            translator_kind: raw.translator_kind,
            alphabet: raw.alphabet,
            max_code_length: raw.max_code_length,
            is_double_pinyin: raw.is_double_pinyin,
            algebra: raw.algebra,
            managed_files: managed,
        });
    }

    out.sort_by(|a, b| a.schema_id.cmp(&b.schema_id));
    out
}

/// 读取 `default.custom.yaml` / `default.yaml` 中启用的方案 id 列表。
fn read_active_schema_ids(user_dir: &Path) -> Vec<String> {
    // 优先级：default.custom.yaml（用户 patch）覆盖 default.yaml
    for name in ["default.custom.yaml", "default.yaml"] {
        let path = user_dir.join(name);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(&text) {
            let list = value
                .get("patch")
                .and_then(|p| p.get("schema_list"))
                .or_else(|| value.get("schema_list"));
            if let Some(seq) = list.and_then(|v| v.as_sequence()) {
                let ids: Vec<String> = seq
                    .iter()
                    .filter_map(|item| {
                        item.get("schema")
                            .and_then(|s| s.as_str())
                            .map(|s| s.to_owned())
                    })
                    .collect();
                if !ids.is_empty() {
                    return ids;
                }
            }
        }
    }
    Vec::new()
}

/// 依据方案结构识别个人词条受管文件（PRD FR-3.1，不硬编码方案名）。
fn detect_managed_files(user_dir: &Path, raw: &SchemaRaw) -> Vec<ManagedFileRef> {
    let mut files = Vec::new();

    // ① custom_phrase 挂载：拼音/双拼方案
    if let Some(user_dict) = &raw.custom_phrase_user_dict {
        for candidate in dict_yaml_candidates(user_dir, user_dict) {
            files.push(ManagedFileRef {
                id: format!("cp:{}", candidate.display()),
                path: candidate,
                format: FileFormat::CustomPhrase,
                role: FileRole::CustomPhrase,
                columns: vec![ColumnKind::Text, ColumnKind::Code, ColumnKind::Weight],
            });
        }
    }

    // ② 音形类：主词典 import_tables 中的用户码表
    if let Some(dict) = &raw.primary_dictionary {
        let dict_path = user_dir.join(format!("{dict}.dict.yaml"));
        if let Ok(text) = std::fs::read_to_string(&dict_path) {
            for import in parse_import_tables(&text) {
                let is_user = import.ends_with(".user") || import.contains(".user.");
                let is_top = import.contains(".user.top");
                if !is_user {
                    continue;
                }
                let path = user_dir.join(format!("{import}.dict.yaml"));
                if !path.is_file() {
                    continue;
                }
                files.push(ManagedFileRef {
                    id: format!("dict:{}", path.display()),
                    path,
                    format: FileFormat::RimeDictYaml,
                    role: if is_top {
                        FileRole::UserTopDict
                    } else {
                        FileRole::UserDict
                    },
                    columns: vec![ColumnKind::Text, ColumnKind::Code],
                });
            }
        }
    }

    files
}

/// custom_phrase 的 `user_dict` 值可带或不带 `.txt` 后缀，统一展开为候选路径。
fn dict_yaml_candidates(user_dir: &Path, user_dict: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let with_txt = user_dir.join(format!("{user_dict}.txt"));
    let bare = user_dir.join(user_dict);
    if with_txt.is_file() {
        out.push(with_txt);
    } else if bare.is_file() {
        out.push(bare);
    }
    out
}

/// 解析 dict.yaml 头部 `import_tables` 列表。
pub(crate) fn parse_import_tables(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_imports = false;
    for line in text.lines() {
        if line.starts_with("import_tables:") {
            in_imports = true;
            continue;
        }
        if in_imports {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("- ") {
                // 去掉行尾注释与引号
                let val = rest.split('#').next().unwrap_or("").trim();
                let val = val.trim_matches('"').trim_matches('\'').trim();
                if !val.is_empty() {
                    out.push(val.to_owned());
                }
            } else if !trimmed.is_empty() && !trimmed.starts_with('#') && !line.starts_with(' ') {
                // import_tables 块结束
                break;
            }
        }
    }
    out
}

/// 生成受管文件的稳定 id。
pub fn make_file_id(path: &Path) -> FileId {
    format!("{}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rime_core::types::TranslatorKind;

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("定位仓库根失败")
            .join("tests/fixtures")
    }

    #[test]
    fn parses_import_tables_from_xhup_dict() {
        let text = std::fs::read_to_string(fixture_root().join("rime-crane-xhup/xhup.dict.yaml"))
            .expect("读取夹具失败");
        let imports = parse_import_tables(&text);
        assert!(
            imports.iter().any(|s| s == "xhup_dicts/xhup.user"),
            "应找到用户码表 import"
        );
        assert!(
            imports.iter().any(|s| s == "xhup_dicts/xhup.user.top"),
            "应找到置顶码表 import"
        );
        // 被注释掉的 import 不应被收集
        assert!(
            !imports.iter().any(|s| s.contains("secondary.simple")),
            "注释行不应被当作 import"
        );
    }

    #[test]
    fn detects_double_pinyin_scheme() {
        let path = fixture_root().join("rime-crane-flypy/double_pinyin_flypy.schema.yaml");
        let raw = schema::parse_schema(&path).expect("解析小鹤双拼方案失败");
        assert_eq!(raw.schema_id, "double_pinyin_flypy");
        assert!(raw.is_double_pinyin, "应识别为双拼方案");
        assert_eq!(
            raw.custom_phrase_user_dict.as_deref(),
            Some("custom_phrase_double")
        );
        assert_eq!(
            raw.alphabet,
            "zyxwvutsrqponmlkjihgfedcbaZYXWVUTSRQPONMLKJIHGFEDCBA`"
        );
        assert!(raw.algebra.is_some(), "双拼方案应有 algebra");
    }

    #[test]
    fn detects_table_scheme_with_user_dicts() {
        let path = fixture_root().join("rime-crane-xhup/xhup.schema.yaml");
        let raw = schema::parse_schema(&path).expect("解析小鹤音形方案失败");
        assert_eq!(raw.schema_id, "xhup");
        assert_eq!(raw.translator_kind, TranslatorKind::Table);
        assert_eq!(raw.primary_dictionary.as_deref(), Some("xhup"));
        assert_eq!(raw.max_code_length, Some(4));
        assert!(raw.alphabet.contains(';'), "xhup 字母表应含分号");
        assert!(!raw.is_double_pinyin, "table 方案不算双拼");
    }
}
#[cfg(test)]
mod real_env {
    //! 真实环境冒烟（`cargo test -p rime-platform -- --ignored --nocapture`）。
    //! 只读访问用户真实 Rime 目录，不做任何写入；用于人工验证受管文件映射。
    use super::*;

    #[test]
    #[ignore = "需要本机真实 Rime 环境"]
    fn smoke_real_environment() {
        let frontends = detect_frontends();
        println!("\n=== 检测到 {} 个前端 ===", frontends.len());
        for f in &frontends {
            println!(
                "  {:?} {} (librime {:?})\n    用户目录: {}\n    部署: {} / {}",
                f.kind,
                f.display_name,
                f.librime_version,
                f.user_dir.display(),
                f.deploy_capability.supported,
                f.deploy_capability.method
            );
        }
        assert!(!frontends.is_empty(), "本机应至少检测到一个前端");

        let dir = &frontends[0].user_dir;
        let schemes = detect_schemes(dir);
        println!("\n=== 方案 {} 个 ===", schemes.len());
        for s in &schemes {
            println!(
                "\n  {} ({}) 类型={:?} 双拼={} 启用={}\n    alphabet={:?} max_code_length={:?}",
                s.name,
                s.schema_id,
                s.translator_kind,
                s.is_double_pinyin,
                s.is_active,
                s.alphabet,
                s.max_code_length
            );
            if s.managed_files.is_empty() {
                println!("    受管文件: 无（该方案未挂载个人词条入口）");
            }
            for mf in &s.managed_files {
                println!(
                    "    受管: {:?} {} 列={:?}",
                    mf.role,
                    mf.path.display(),
                    mf.columns
                );
            }
        }
        assert!(!schemes.is_empty(), "应识别出方案");
    }
}

// ───────────────────────── 本地拼音数据源 ─────────────────────────

/// 从本机 Rime 安装的字表中加载「汉字 → 拼音读音」映射。
///
/// 设计理由（对应 M0 报告遗留项 3）：
/// - 内置拼音表只覆盖常用字，生僻字无法生成编码建议；
/// - 用户本机的 `cn_dicts/8105.dict.yaml` 等字表覆盖 8105 个通用规范汉字，
///   且含多音字多读音，正是我们需要的；
/// - **运行时读取、随工具不分发**，因此不涉及第三方码表的许可传染（PRD §4 许可边界）。
///
/// 返回 `HashMap<char, Vec<String>>`，可直接注入 `rime_suggest::PinyinSource`。
pub fn load_char_pinyin(user_dir: &Path) -> std::collections::HashMap<char, Vec<String>> {
    let mut out: std::collections::HashMap<char, Vec<String>> = std::collections::HashMap::new();

    for path in candidate_char_dicts(user_dir) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut in_data = false;
        for line in text.lines() {
            if !in_data {
                if line.trim() == "..." {
                    in_data = true;
                }
                continue;
            }
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut fields = line.split('\t');
            let (Some(text_field), Some(code_field)) = (fields.next(), fields.next()) else {
                continue;
            };
            // 只要单字（多字词条对单字读音无帮助，且拼音含空格）
            let mut chars = text_field.chars();
            let (Some(ch), None) = (chars.next(), chars.next()) else {
                continue;
            };
            // 单字读音应是单音节；含空格的是词级拼音，跳过
            let code = code_field.trim();
            if code.is_empty() || code.contains(' ') {
                continue;
            }
            out.entry(ch).or_default().push(code.to_owned());
        }
        if !out.is_empty() {
            break; // 找到一个可用字表即可
        }
    }

    // 去重，保留首次出现顺序（首读音常为最常用读音）
    for readings in out.values_mut() {
        let mut seen = std::collections::HashSet::new();
        readings.retain(|r| seen.insert(r.clone()));
    }
    out
}

/// 候选字表位置：rime-ice 系默认在 `cn_dicts/` 下，也兼容放在根目录的情况。
fn candidate_char_dicts(user_dir: &Path) -> Vec<PathBuf> {
    const NAMES: [&str; 4] = ["8105", "41448", "standard", "hanzi"];
    let mut out = Vec::new();
    for sub in ["cn_dicts", ""] {
        for name in NAMES {
            let p = if sub.is_empty() {
                user_dir.join(format!("{name}.dict.yaml"))
            } else {
                user_dir.join(sub).join(format!("{name}.dict.yaml"))
            };
            if p.is_file() {
                out.push(p);
            }
        }
    }
    out
}

#[cfg(test)]
mod pinyin_source {
    use super::*;

    #[test]
    fn loads_char_pinyin_from_real_installation() {
        // 使用本机真实 Rime 目录（只读）；无安装时跳过，避免在其他机器上失败
        let dir = fcitx5_user_dir();
        if !dir.is_dir() {
            eprintln!("跳过：本机无 fcitx5 rime 目录");
            return;
        }
        let map = load_char_pinyin(&dir);
        if map.is_empty() {
            eprintln!("跳过：未找到字表");
            return;
        }
        println!("加载单字读音：{} 个", map.len());
        // 「中」「州」「韵」都应存在（内置表只有 326 字时缺后两个）
        for ch in ['中', '州', '韵'] {
            let r = map.get(&ch);
            println!("  {ch} -> {r:?}");
            assert!(r.is_some_and(|v| !v.is_empty()), "{ch} 应有读音");
        }
        // 多音字应给出多个读音
        if let Some(v) = map.get(&'行') {
            println!("  行（多音字）-> {v:?}");
            assert!(v.len() >= 2, "「行」应有多个读音，实际 {v:?}");
        }
    }
}
