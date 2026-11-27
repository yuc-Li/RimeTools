//! 方案级配置读写：快速上屏、最大码长、词表挂载。
//!
//! 写入策略（不破坏可维护性）：
//! - **speller 配置**写 `<schema>.custom.yaml` 的 `patch:`，这是 Rime 官方的覆盖机制，
//!   方案文件升级时不会被冲掉；已有 patch 会保留其他键；
//! - **词表挂载**只能改主词典的 `import_tables`（Rime 没有词典补丁机制），
//!   因此只做「启用/注释」的行级切换，其余内容逐字保留。
//!
//! 本模块只做**文本 → 文本**的变换，文件 IO 与备份由命令层负责，便于单测。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpellerConfig {
    pub auto_select: Option<bool>,
    pub max_code_length: Option<u32>,
    pub auto_select_pattern: Option<String>,
    pub auto_clear: Option<String>,
    pub alphabet: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MountEntry {
    /// `import_tables` 里写的名字，如 `xhup_dicts/xhup.primary`
    pub name: String,
    pub enabled: bool,
    /// 行尾注释（去掉 `#` 与首尾空白）
    pub comment: Option<String>,
    /// 解析到的实际文件（不存在则为 null）
    pub path: Option<String>,
    pub entry_count: Option<u32>,
    /// 用户码表建议保持挂载
    pub is_user_table: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemeConfigSnapshot {
    pub schema_id: String,
    pub schema_name: String,
    /// 方案文件路径（只读，用于展示与判定）
    pub schema_path: String,
    /// 覆盖文件路径（不存在时为将要创建的路径）
    pub custom_path: String,
    pub custom_exists: bool,
    /// 生效值（方案文件 + custom 覆盖后的结果）
    pub speller: SpellerConfig,
    /// 每个字段的来源：`schema` 或 `custom`
    pub speller_source: String,
    /// 主词典路径（`import_tables` 所在文件）
    pub dict_path: Option<String>,
    pub mounts: Vec<MountEntry>,
    /// 提示：改完需要重新部署
    pub deploy_hint: String,
}

/// 输入法配置是否可编辑（当前只保证音形类方案；其他方案允许查看但会提示风险）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigSupport {
    /// 已适配并实测
    Supported,
    /// 结构符合但没有实测，编辑前提示
    Experimental,
}

// ───────────────────────── 读取 ─────────────────────────

/// 解析 `import_tables` 区段里的挂载项（含被注释掉的）。
pub fn parse_mounts(dict_text: &str) -> Vec<(String, bool, Option<String>)> {
    let mut out = Vec::new();
    let mut in_block = false;
    let mut block_indent = 0usize;

    for line in dict_text.lines() {
        let trimmed = line.trim_start();
        if !in_block {
            if trimmed.starts_with("import_tables:") {
                in_block = true;
                block_indent = line.len() - trimmed.len();
            }
            continue;
        }

        // 退出条件：出现同级或更浅缩进的非空、非注释、非列表项行
        let indent = line.len() - trimmed.len();
        if !trimmed.is_empty()
            && !trimmed.starts_with('-')
            && !trimmed.starts_with('#')
            && indent <= block_indent
        {
            break;
        }
        if trimmed.is_empty() {
            continue;
        }

        if let Some(item) = parse_mount_line(trimmed) {
            out.push(item);
        }
    }
    out
}

/// 解析一行挂载项：`- "name" # 注释` 或 `# - "name" # 注释`。
fn parse_mount_line(trimmed: &str) -> Option<(String, bool, Option<String>)> {
    let (rest, enabled) = if let Some(r) = trimmed.strip_prefix("- ") {
        (r, true)
    } else if let Some(r) = trimmed.strip_prefix("#- ") {
        (r, false)
    } else if let Some(r) = trimmed.strip_prefix("# - ") {
        (r, false)
    } else {
        return None;
    };

    // 名字可能带引号
    let rest = rest.trim_start();
    let (name, tail) = if let Some(stripped) = rest.strip_prefix('"') {
        let end = stripped.find('"')?;
        (stripped[..end].trim().to_owned(), &stripped[end + 1..])
    } else if let Some(stripped) = rest.strip_prefix('\'') {
        let end = stripped.find('\'')?;
        (stripped[..end].trim().to_owned(), &stripped[end + 1..])
    } else {
        // 无引号：到空白或注释为止
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '#')
            .unwrap_or(rest.len());
        (rest[..end].trim().to_owned(), &rest[end..])
    };

    if name.is_empty() {
        return None;
    }
    let comment = tail
        .trim()
        .strip_prefix('#')
        .map(|c| c.trim().to_owned())
        .filter(|c| !c.is_empty());
    Some((name, enabled, comment))
}

/// 读取方案文件里的 speller 配置。
pub fn read_speller_from_schema(schema_text: &str) -> SpellerConfig {
    let mut cfg = SpellerConfig::default();
    let mut in_speller = false;
    let mut base_indent = 0usize;

    for line in schema_text.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if !in_speller {
            if trimmed.starts_with("speller:") {
                in_speller = true;
                base_indent = indent;
            }
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if indent <= base_indent && !trimmed.starts_with('-') {
            break;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let value = value.split('#').next().unwrap_or("").trim();
        let value = value.trim_matches('"').trim_matches('\'');
        match key.trim() {
            "auto_select" => cfg.auto_select = Some(value == "true"),
            "max_code_length" => cfg.max_code_length = value.parse().ok(),
            "auto_select_pattern" => cfg.auto_select_pattern = Some(value.to_owned()),
            "auto_clear" => cfg.auto_clear = Some(value.to_owned()),
            "alphabet" => cfg.alphabet = Some(value.to_owned()),
            _ => {}
        }
    }
    cfg
}

/// 读取 custom 文件里 `patch:` 段生效的 speller 覆盖。
pub fn read_speller_from_custom(custom_text: &str) -> SpellerConfig {
    let mut cfg = SpellerConfig::default();
    let mut in_patch = false;

    for line in custom_text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("patch:") {
            in_patch = true;
            continue;
        }
        if !in_patch {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim().trim_matches('"').trim_matches('\'');
        let value = value.split('#').next().unwrap_or("").trim();
        let value = value.trim_matches('"').trim_matches('\'');
        match key {
            "speller/auto_select" => cfg.auto_select = Some(value == "true"),
            "speller/max_code_length" => cfg.max_code_length = value.parse().ok(),
            "speller/auto_select_pattern" => cfg.auto_select_pattern = Some(value.to_owned()),
            "speller/auto_clear" => cfg.auto_clear = Some(value.to_owned()),
            _ => {}
        }
    }
    cfg
}

// ───────────────────────── 写入：custom.yaml ─────────────────────────

/// 生成/更新 `<schema>.custom.yaml` 的 patch，只改动传入的键。
///
/// 处理的形态：
/// - 文件不存在 → 创建最小 patch；
/// - 存在但 `patch:` 被注释（上游模板常见）→ 取消注释后插入；
/// - 已生效 → 替换同名键的值，保留其它键与所有注释。
pub fn patch_custom_yaml(
    existing: Option<&str>,
    auto_select: Option<bool>,
    max_code_length: Option<u32>,
) -> String {
    const KEY_AUTO: &str = "speller/auto_select";
    const KEY_MAX: &str = "speller/max_code_length";

    let mut lines: Vec<String> = existing
        .unwrap_or("")
        .lines()
        .map(|l| l.to_owned())
        .collect();

    // 1) 找到（或创建）生效的 patch 行
    let mut patch_idx: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        if t.starts_with("patch:") && !t.starts_with('#') {
            patch_idx = Some(i);
            break;
        }
    }
    if patch_idx.is_none() {
        // 找被注释的 patch:
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim_start();
            let uncommented = t.strip_prefix('#').map(|s| s.trim_start()).unwrap_or("");
            if uncommented.starts_with("patch:") {
                lines[i] = "patch:".to_owned();
                patch_idx = Some(i);
                break;
            }
        }
    }
    if patch_idx.is_none() {
        if !lines.is_empty() && !lines.last().map(|l| l.trim().is_empty()).unwrap_or(true) {
            lines.push(String::new());
        }
        lines.push("patch:".to_owned());
        patch_idx = Some(lines.len() - 1);
    }
    let patch_idx = patch_idx.expect("刚刚保证存在");

    // 2) 删除已有的**生效**同名键（注释里的模板行保留）
    let targets = [KEY_AUTO, KEY_MAX];
    lines = lines
        .into_iter()
        .enumerate()
        .filter(|(i, line)| {
            if Some(*i) == Some(patch_idx) {
                return true;
            }
            let t = line.trim_start();
            if t.starts_with('#') {
                return true; // 注释行一律保留
            }
            !targets
                .iter()
                .any(|k| t.starts_with(&format!("{k}:")) || t.starts_with(&format!("\"{k}\":")))
        })
        .map(|(_, l)| l)
        .collect();

    // 重新定位 patch（删除后行号可能变化）
    let patch_idx = lines
        .iter()
        .position(|l| l.trim_start().starts_with("patch:") && !l.trim_start().starts_with('#'))
        .expect("patch 行必须存在");

    // 3) 在 patch 之后插入新键（保持输入顺序，便于阅读）
    let mut insert: Vec<String> = Vec::new();
    if let Some(v) = auto_select {
        insert.push(format!("  {KEY_AUTO}: {v}"));
    }
    if let Some(v) = max_code_length {
        insert.push(format!("  {KEY_MAX}: {v}"));
    }
    for (offset, line) in insert.into_iter().enumerate() {
        lines.insert(patch_idx + 1 + offset, line);
    }

    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

// ───────────────────────── 写入：import_tables ─────────────────────────

/// 按 `(name, enabled)` 切换挂载状态，其余内容逐字保留。
///
/// 返回 `None` 表示没有改动。
pub fn toggle_mounts(dict_text: &str, toggles: &[(String, bool)]) -> Option<String> {
    if toggles.is_empty() {
        return None;
    }
    let mut changed = false;
    let mut out = String::with_capacity(dict_text.len());

    for line in dict_text.lines() {
        match toggle_one_line(line, toggles) {
            Some(new_line) => {
                changed = true;
                out.push_str(&new_line);
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }

    if !changed {
        return None;
    }
    // 保持原文件末尾是否有换行的风格
    if !dict_text.ends_with('\n') {
        out.pop();
    }
    Some(out)
}

/// 只切换行首的 `# ` 前缀：`- "x"  # 注释` ⇄ `# - "x"  # 注释`。
///
/// 不做任何重排，因此原有的列对齐、引号风格与行尾注释都逐字保留。
fn toggle_one_line(line: &str, toggles: &[(String, bool)]) -> Option<String> {
    let trimmed = line.trim_start();
    let indent_len = line.len() - trimmed.len();

    // 找出列表项在行内的起始偏移（跳过 `# ` 前缀）
    let (item_off, enabled) = if trimmed.starts_with("- ") {
        (0, true)
    } else {
        let hash = trimmed.find('#')?;
        let after_hash = &trimmed[hash + 1..];
        let spaces = after_hash.len() - after_hash.trim_start().len();
        let off = hash + 1 + spaces;
        if !trimmed[off..].starts_with("- ") {
            return None;
        }
        (off, false)
    };

    let (name, _, _) = parse_mount_line(&trimmed[item_off..])?;
    let (_, want) = toggles.iter().find(|(n, _)| n == &name)?;
    if *want == enabled {
        return None;
    }

    let mut out = String::with_capacity(line.len() + 2);
    out.push_str(&line[..indent_len]);
    if *want {
        // 启用：去掉 `#` 与其后的空白
        out.push_str(&trimmed[item_off..]);
    } else {
        // 停用：在列表项前插入 `# `
        out.push_str("# ");
        out.push_str(&trimmed[item_off..]);
    }
    Some(out)
}

// ───────────────────────── 快照 ─────────────────────────

/// 解析方案主词典路径与挂载项。
pub fn dict_path_for(user_dir: &Path, primary: &str) -> PathBuf {
    let base = user_dir.join(primary);
    let with_ext = PathBuf::from(format!("{}.dict.yaml", base.display()));
    if with_ext.is_file() {
        with_ext
    } else {
        let yaml = PathBuf::from(format!("{}.yaml", base.display()));
        if yaml.is_file() {
            yaml
        } else {
            with_ext
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = "\
schema:
  schema_id: xhup
  name: 小鹤音形
speller:
  alphabet: \"abcdefghijklmnopqrstuvwxyz;'\"
  initials: ';abcdefghijklmnopqrstuvwxyz'
  max_code_length: 4
  auto_select: true   #顶字上屏
  auto_select_pattern: ^;.$|^\\w{4}$
translator:
  dictionary: xhup
";

    /// 上游模板：patch 整段被注释
    const TEMPLATE: &str = "\
##  一些常用的设置项
# patch:
  # speller/max_code_length: 100 # 提高最大编码长度
  # speller/auto_select: true # 自动上屏
  # style/candidate_list_layout: linear
";

    const DICT: &str = "\
---
name: xhup
import_tables:
  # --- 用户优先层 ---
  - \"xhup_dicts/xhup.user.top\"          # 用户置顶码表
  - \"xhup_dicts/xhup.user\"              # 用户自用词库

  # --- 扩展 ---
  - \"xhup_dicts/xhup.primary\"           # 首选字词
  # - \"xhup_dicts/xhup.rare.chars\"        # 生僻字
...
";

    #[test]
    fn reads_speller_from_schema() {
        let cfg = read_speller_from_schema(SCHEMA);
        assert_eq!(cfg.auto_select, Some(true));
        assert_eq!(cfg.max_code_length, Some(4));
        assert_eq!(cfg.auto_select_pattern.as_deref(), Some("^;.$|^\\w{4}$"));
        assert!(cfg.alphabet.unwrap().contains(';'));
    }

    #[test]
    fn parses_mounts_including_commented_and_keeps_comments() {
        let mounts = parse_mounts(DICT);
        assert_eq!(mounts.len(), 4);
        assert_eq!(mounts[0].0, "xhup_dicts/xhup.user.top");
        assert!(mounts[0].1);
        assert_eq!(mounts[0].2.as_deref(), Some("用户置顶码表"));
        assert_eq!(mounts[3].0, "xhup_dicts/xhup.rare.chars");
        assert!(!mounts[3].1, "被注释的条目应视为未挂载");
    }

    #[test]
    fn mounts_block_stops_at_next_top_level_key() {
        let text = "import_tables:\n  - a\nname: x\n  - b\n";
        let mounts = parse_mounts(text);
        assert_eq!(mounts.len(), 1, "遇到同级键应停止解析");
    }

    #[test]
    fn creates_custom_file_when_missing() {
        let out = patch_custom_yaml(None, Some(false), Some(4));
        assert!(out.starts_with("patch:\n"));
        assert!(out.contains("  speller/auto_select: false"));
        assert!(out.contains("  speller/max_code_length: 4"));
    }

    #[test]
    fn activates_commented_patch_from_template() {
        let out = patch_custom_yaml(Some(TEMPLATE), Some(false), None);
        assert!(out.contains("\npatch:\n"), "应取消注释生成生效的 patch");
        assert!(out.contains("  speller/auto_select: false"));
        // 模板里的说明行必须保留
        assert!(out.contains("# style/candidate_list_layout: linear"));
        assert!(out.contains("# 提高最大编码长度"));
        // 只声明了 auto_select，不应顺手写 max_code_length（注释掉的模板行不算）
        assert!(
            !out.lines()
                .any(|l| l.trim_start().starts_with("speller/max_code_length:")),
            "未传入的键不应被写入"
        );
    }

    #[test]
    fn replaces_existing_key_without_duplicating() {
        let existing = "patch:\n  speller/auto_select: true\n  style/inline_preedit: true\n";
        let out = patch_custom_yaml(Some(existing), Some(false), Some(4));
        assert_eq!(
            out.matches("speller/auto_select").count(),
            1,
            "不得出现重复键"
        );
        assert!(out.contains("speller/auto_select: false"));
        assert!(out.contains("style/inline_preedit: true"), "其它键必须保留");
        assert!(out.contains("speller/max_code_length: 4"));
    }

    #[test]
    fn reads_effective_values_from_custom() {
        let custom = "patch:\n  speller/auto_select: false\n  speller/max_code_length: 8\n";
        let cfg = read_speller_from_custom(custom);
        assert_eq!(cfg.auto_select, Some(false));
        assert_eq!(cfg.max_code_length, Some(8));
    }

    #[test]
    fn commented_keys_in_custom_are_ignored() {
        let custom = "# patch:\n#   speller/auto_select: false\n";
        let cfg = read_speller_from_custom(custom);
        assert_eq!(cfg.auto_select, None, "注释掉的补丁不应生效");
    }

    #[test]
    fn toggles_mount_off_and_on_preserving_format() {
        let off = toggle_mounts(DICT, &[("xhup_dicts/xhup.primary".into(), false)]).unwrap();
        assert!(
            off.lines()
                .any(|l| l == "  # - \"xhup_dicts/xhup.primary\"           # 首选字词"),
            "停用后应只多出 `# ` 前缀，其余字符不变；实际：{off}"
        );
        // 其他行逐字保留
        assert!(off.contains("  # --- 用户优先层 ---"));
        assert!(off.contains("- \"xhup_dicts/xhup.user.top\""));

        let back = toggle_mounts(&off, &[("xhup_dicts/xhup.primary".into(), true)]).unwrap();
        assert!(
            back.lines()
                .any(|l| l == "  - \"xhup_dicts/xhup.primary\"           # 首选字词"),
            "重新启用应恢复原行"
        );
        assert!(!back.contains("# - \"xhup_dicts/xhup.primary\""));
    }

    #[test]
    fn toggle_enables_a_commented_entry() {
        let out = toggle_mounts(DICT, &[("xhup_dicts/xhup.rare.chars".into(), true)]).unwrap();
        assert!(
            out.lines()
                .any(|l| l == "  - \"xhup_dicts/xhup.rare.chars\"        # 生僻字"),
            "启用后应恢复为列表项且保留行尾注释；实际：{out}"
        );
    }

    #[test]
    fn toggle_without_change_is_none() {
        assert!(toggle_mounts(DICT, &[("xhup_dicts/xhup.primary".into(), true)]).is_none());
        assert!(toggle_mounts(DICT, &[]).is_none());
    }

    #[test]
    fn toggle_is_idempotent_and_stable() {
        let once = toggle_mounts(DICT, &[("xhup_dicts/xhup.primary".into(), false)]).unwrap();
        let twice = toggle_mounts(&once, &[("xhup_dicts/xhup.primary".into(), false)]);
        assert!(twice.is_none(), "重复设置同一状态不应产生改动");
    }
}
