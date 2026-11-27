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
    /// 可配置项（表驱动，见 `OPTION_SPECS`）
    pub options: Vec<OptionState>,
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

// ───────────────────────── 可配置项目录（表驱动） ─────────────────────────
//
// 字段名与取值依据 Rime 官方文档：
// - `RimeWithSchemata`（github.com/rime/home/wiki）
// - 《Schema.yaml 詳解》(LEOYoon-Tsaw/Rime_collections) 的「設定項速查」
// - `rime-prelude/default.yaml` 的默认值
//
// 只收录「改了就生效、且不破坏方案结构」的标量项；列表型（engine.filters、
// switches、key_binder.bindings）涉及增删顺序，不在本版本自动改写。

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OptionKind {
    Bool,
    Int,
    Float,
    Text,
    /// 取值受限的文本（如 `auto_clear`）
    Enum,
}

/// 一个可配置项的值（只支持标量）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OptionValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

impl OptionValue {
    /// 渲染为 YAML 标量。
    fn to_yaml(&self) -> String {
        match self {
            OptionValue::Bool(b) => b.to_string(),
            OptionValue::Number(n) => {
                if n.fract() == 0.0 {
                    format!("{}", *n as i64)
                } else {
                    n.to_string()
                }
            }
            // 单引号包裹最稳：正则、空格、冒号都不会被误解析
            OptionValue::Text(t) => format!("'{}'", t.replace('\'', "''")),
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            OptionValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            OptionValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            OptionValue::Text(t) => Some(t),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionChoice {
    pub value: String,
    pub label: String,
}

/// 配置项定义（静态目录）。
#[derive(Debug, Clone)]
pub struct OptionSpec {
    /// 补丁键，如 `speller/max_code_length`
    pub key: &'static str,
    pub kind: OptionKind,
    pub group: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub choices: &'static [(&'static str, &'static str)],
}

/// 一个配置项的当前状态（目录 + 取值）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionState {
    pub key: String,
    pub group: String,
    pub label: String,
    pub description: String,
    pub kind: OptionKind,
    /// 当前生效值（方案文件，被 custom 覆盖时取 custom）
    pub value: Option<OptionValue>,
    /// 方案文件里的原始值（用于「恢复默认」）
    pub default: Option<OptionValue>,
    /// custom.yaml 里是否有显式覆盖
    pub overridden: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub choices: Vec<OptionChoice>,
}

const GROUP_SPELLER: &str = "输入与上屏";
const GROUP_MENU: &str = "候选与选字";
const GROUP_TRANSLATOR: &str = "词典与学习";
const GROUP_OTHER: &str = "标点与按键";

/// 官方文档里较常用、改了不会破坏方案结构的配置项。
pub const OPTION_SPECS: &[OptionSpec] = &[
    // ── 输入与上屏 ──
    OptionSpec {
        key: "speller/max_code_length",
        kind: OptionKind::Int,
        group: GROUP_SPELLER,
        label: "最大码长",
        description: "超过这个长度不再接收编码；音形类通常是 4，拼音类通常不设。",
        min: Some(1.0),
        max: Some(32.0),
        choices: &[],
    },
    OptionSpec {
        key: "speller/auto_select",
        kind: OptionKind::Bool,
        group: GROUP_SPELLER,
        label: "快速上屏",
        description: "编码唯一时直接上屏，不再等选词。",
        min: None,
        max: None,
        choices: &[],
    },
    OptionSpec {
        key: "speller/auto_select_pattern",
        kind: OptionKind::Text,
        group: GROUP_SPELLER,
        label: "自动上屏规则",
        description: "正则表达式；输入串匹配时自动顶字上屏（配合 auto_select 使用）。",
        min: None,
        max: None,
        choices: &[],
    },
    OptionSpec {
        key: "speller/auto_clear",
        kind: OptionKind::Enum,
        group: GROUP_SPELLER,
        label: "空码清理",
        description: "空码时的行为：manual 按键确认、auto 自动清屏、max_length 达到最长码后清屏。",
        min: None,
        max: None,
        choices: &[
            ("manual", "手动确认"),
            ("auto", "自动清理"),
            ("max_length", "达到最长码时清理"),
        ],
    },
    OptionSpec {
        key: "speller/use_space",
        kind: OptionKind::Bool,
        group: GROUP_SPELLER,
        label: "空格作为编码",
        description: "允许空格键进入编码串（少数方案使用）。",
        min: None,
        max: None,
        choices: &[],
    },
    OptionSpec {
        key: "speller/delimiter",
        kind: OptionKind::Text,
        group: GROUP_SPELLER,
        label: "分音符",
        description: "音節之间的分隔符，例如 ` '`（空格与单引号）。",
        min: None,
        max: None,
        choices: &[],
    },
    // ── 候选与选字 ──
    OptionSpec {
        key: "menu/page_size",
        kind: OptionKind::Int,
        group: GROUP_MENU,
        label: "每页候选数",
        description: "候选窗一页显示多少个候选（官方默认 5）。",
        min: Some(1.0),
        max: Some(20.0),
        choices: &[],
    },
    OptionSpec {
        key: "menu/alternative_select_keys",
        kind: OptionKind::Text,
        group: GROUP_MENU,
        label: "备用选字键",
        description: "当数字键被编码占用时，用这些键选字，例如 ASDFGHJKL。",
        min: None,
        max: None,
        choices: &[],
    },
    // ── 词典与学习 ──
    OptionSpec {
        key: "translator/enable_sentence",
        kind: OptionKind::Bool,
        group: GROUP_TRANSLATOR,
        label: "智能造句",
        description: "允许把多个编码段连成一句（码表类方案通常关闭）。",
        min: None,
        max: None,
        choices: &[],
    },
    OptionSpec {
        key: "translator/enable_completion",
        kind: OptionKind::Bool,
        group: GROUP_TRANSLATOR,
        label: "编码提示",
        description: "提前显示尚未输入完整码的字（简码提示）。",
        min: None,
        max: None,
        choices: &[],
    },
    OptionSpec {
        key: "translator/enable_user_dict",
        kind: OptionKind::Bool,
        group: GROUP_TRANSLATOR,
        label: "用户词典学习",
        description: "记录动态字词频与你输入的词；关闭后不再学习。",
        min: None,
        max: None,
        choices: &[],
    },
    OptionSpec {
        key: "translator/enable_encoder",
        kind: OptionKind::Bool,
        group: GROUP_TRANSLATOR,
        label: "自动造词",
        description: "码表类方案：允许按编码规则自动成词（仅 table_translator）。",
        min: None,
        max: None,
        choices: &[],
    },
    OptionSpec {
        key: "translator/encode_commit_history",
        kind: OptionKind::Bool,
        group: GROUP_TRANSLATOR,
        label: "上屏词入词库",
        description: "把刚上屏的词按规则自动记入词库（仅 table_translator）。",
        min: None,
        max: None,
        choices: &[],
    },
    OptionSpec {
        key: "translator/max_phrase_length",
        kind: OptionKind::Int,
        group: GROUP_TRANSLATOR,
        label: "自动成词最大词长",
        description: "配合自动造词使用，限制自动成词的长度。",
        min: Some(1.0),
        max: Some(20.0),
        choices: &[],
    },
    OptionSpec {
        key: "translator/initial_quality",
        kind: OptionKind::Float,
        group: GROUP_TRANSLATOR,
        label: "出字优先级",
        description: "多个翻译器结果合并时的排序权重，数值越大越靠前。",
        min: Some(0.0),
        max: Some(10.0),
        choices: &[],
    },
    OptionSpec {
        key: "translator/spelling_hints",
        kind: OptionKind::Int,
        group: GROUP_TRANSLATOR,
        label: "拼音提示字数",
        description: "多少个字以内标注完整带调拼音（仅 script_translator）。",
        min: Some(0.0),
        max: Some(10.0),
        choices: &[],
    },
    OptionSpec {
        key: "translator/always_show_comments",
        kind: OptionKind::Bool,
        group: GROUP_TRANSLATOR,
        label: "始终显示提示码",
        description: "候选旁始终显示提示信息（仅 script_translator）。",
        min: None,
        max: None,
        choices: &[],
    },
    OptionSpec {
        key: "translator/enable_charset_filter",
        kind: OptionKind::Bool,
        group: GROUP_TRANSLATOR,
        label: "字符集过滤",
        description: "按字符集开关过滤候选（仅 table_translator，或配合 cjk_minifier）。",
        min: None,
        max: None,
        choices: &[],
    },
    OptionSpec {
        key: "translator/strict_spelling",
        kind: OptionKind::Bool,
        group: GROUP_TRANSLATOR,
        label: "严格拼写",
        description: "配合简拼规则时，只按略拼码组词（仅 table_translator）。",
        min: None,
        max: None,
        choices: &[],
    },
    // ── 标点与按键 ──
    OptionSpec {
        key: "punctuator/import_preset",
        kind: OptionKind::Text,
        group: GROUP_OTHER,
        label: "符号表预设",
        description: "使用哪份预设符号表，如 default / symbols。",
        min: None,
        max: None,
        choices: &[],
    },
    OptionSpec {
        key: "key_binder/import_preset",
        kind: OptionKind::Text,
        group: GROUP_OTHER,
        label: "按键绑定预设",
        description: "使用哪份预设按键绑定，如 default。",
        min: None,
        max: None,
        choices: &[],
    },
    OptionSpec {
        key: "recognizer/import_preset",
        kind: OptionKind::Text,
        group: GROUP_OTHER,
        label: "识别器预设",
        description: "使用哪份预设识别规则（网址、邮箱、反查等）。",
        min: None,
        max: None,
        choices: &[],
    },
];

// ───────────────────────── 通用读取 ─────────────────────────

/// 从 YAML 文本里读某个二级标量：`section/key`。
pub fn read_scalar(text: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    let mut base_indent = 0usize;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        let indent = line.len() - trimmed.len();
        if !in_section {
            if trimmed.starts_with(&format!("{section}:")) {
                in_section = true;
                base_indent = indent;
            }
            continue;
        }
        if indent <= base_indent {
            break; // 段落结束
        }
        let Some((k, v)) = trimmed.split_once(':') else {
            continue;
        };
        if k.trim().trim_matches('"').trim_matches('\'') != key {
            continue;
        }
        let value = v.split(" #").next().unwrap_or("").trim();
        let value = value.trim_matches('"').trim_matches('\'');
        if value.is_empty() {
            continue;
        }
        return Some(value.to_owned());
    }
    None
}

/// 读 custom 文件 `patch:` 里的原始文本值（不做类型转换）。
pub fn read_patch_raw(custom_text: &str, key: &str) -> Option<String> {
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
        let Some((k, v)) = trimmed.split_once(':') else {
            continue;
        };
        if k.trim().trim_matches('"').trim_matches('\'') != key {
            continue;
        }
        let value = v.split(" #").next().unwrap_or("").trim();
        let value = value.trim_matches('"').trim_matches('\'');
        return Some(value.to_owned());
    }
    None
}

fn coerce(kind: OptionKind, raw: &str) -> Option<OptionValue> {
    match kind {
        OptionKind::Bool => Some(OptionValue::Bool(matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "true" | "yes" | "1"
        ))),
        OptionKind::Int | OptionKind::Float => {
            raw.trim().parse::<f64>().ok().map(OptionValue::Number)
        }
        OptionKind::Text | OptionKind::Enum => Some(OptionValue::Text(raw.to_owned())),
    }
}

/// 计算全部配置项的当前状态。
pub fn read_options(schema_text: &str, custom_text: Option<&str>) -> Vec<OptionState> {
    OPTION_SPECS
        .iter()
        .map(|spec| {
            let (section, key) = spec
                .key
                .split_once('/')
                .expect("配置键必须是 section/key 形式");

            let default =
                read_scalar(schema_text, section, key).and_then(|raw| coerce(spec.kind, &raw));

            let custom_raw = custom_text.and_then(|t| read_patch_raw(t, spec.key));
            let overridden = custom_raw.is_some();
            let value = match &custom_raw {
                Some(raw) => coerce(spec.kind, raw).or_else(|| default.clone()),
                None => default.clone(),
            };

            OptionState {
                key: spec.key.to_owned(),
                group: spec.group.to_owned(),
                label: spec.label.to_owned(),
                description: spec.description.to_owned(),
                kind: spec.kind,
                value,
                default,
                overridden,
                min: spec.min,
                max: spec.max,
                choices: spec
                    .choices
                    .iter()
                    .map(|(v, l)| OptionChoice {
                        value: (*v).to_owned(),
                        label: (*l).to_owned(),
                    })
                    .collect(),
            }
        })
        .collect()
}

// ───────────────────────── 通用写入 ─────────────────────────

/// 把一批 `(key, Option<value>)` 写进 custom 文件的 `patch:`。
///
/// `None` 表示**删除**该键（恢复方案文件里的默认值）。
pub fn patch_custom_values(
    existing: Option<&str>,
    updates: &[(String, Option<OptionValue>)],
) -> String {
    let mut lines: Vec<String> = existing
        .unwrap_or("")
        .lines()
        .map(|l| l.to_owned())
        .collect();

    // 1) 确保有生效的 patch: 行
    let mut patch_idx: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        if t.starts_with("patch:") && !t.starts_with('#') {
            patch_idx = Some(i);
            break;
        }
    }
    if patch_idx.is_none() {
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
    }

    // 2) 删除这些键的既有生效行（注释行保留）
    let keys: Vec<&str> = updates.iter().map(|(k, _)| k.as_str()).collect();
    lines = lines
        .into_iter()
        .filter(|line| {
            let t = line.trim_start();
            if t.starts_with('#') || t.starts_with("patch:") {
                return true;
            }
            let Some((k, _)) = t.split_once(':') else {
                return true;
            };
            let k = k.trim().trim_matches('"').trim_matches('\'');
            !keys.contains(&k)
        })
        .collect();

    let patch_idx = lines
        .iter()
        .position(|l| l.trim_start().starts_with("patch:") && !l.trim_start().starts_with('#'))
        .expect("patch 行必须存在");

    // 3) 插入新值
    let mut insert: Vec<String> = Vec::new();
    for (key, value) in updates {
        if let Some(v) = value {
            insert.push(format!("  {key}: {}", v.to_yaml()));
        }
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
    // 兼容入口：旧调用方只关心这两项，统一走通用写入逻辑
    let mut updates: Vec<(String, Option<OptionValue>)> = Vec::new();
    if let Some(v) = auto_select {
        updates.push(("speller/auto_select".to_owned(), Some(OptionValue::Bool(v))));
    }
    if let Some(v) = max_code_length {
        updates.push((
            "speller/max_code_length".to_owned(),
            Some(OptionValue::Number(v as f64)),
        ));
    }
    patch_custom_values(existing, &updates)
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

#[cfg(test)]
mod option_tests {
    use super::*;

    const SCHEMA: &str = "\
schema:
  schema_id: xhup
speller:
  alphabet: \"abcdefghijklmnopqrstuvwxyz;'\"
  max_code_length: 4
  auto_select: true   #顶字上屏
  auto_select_pattern: ^;.$|^\\w{4}$
  auto_clear: max_length
menu:
  page_size: 5
translator:
  dictionary: xhup
  enable_sentence: false
  enable_completion: false
  max_phrase_length: 5
  initial_quality: 0.75
";

    #[test]
    fn reads_scalars_from_sections() {
        assert_eq!(
            read_scalar(SCHEMA, "speller", "max_code_length").as_deref(),
            Some("4")
        );
        assert_eq!(
            read_scalar(SCHEMA, "speller", "auto_select").as_deref(),
            Some("true")
        );
        assert_eq!(
            read_scalar(SCHEMA, "menu", "page_size").as_deref(),
            Some("5")
        );
        assert_eq!(
            read_scalar(SCHEMA, "translator", "initial_quality").as_deref(),
            Some("0.75")
        );
        // 段落未声明的键返回 None
        assert!(read_scalar(SCHEMA, "translator", "enable_encoder").is_none());
        // 不能跨段落误读
        assert!(read_scalar(SCHEMA, "menu", "max_code_length").is_none());
    }

    #[test]
    fn options_are_typed_and_marked_overridden() {
        let custom = "patch:\n  speller/auto_select: false\n  menu/page_size: 9\n";
        let opts = read_options(SCHEMA, Some(custom));

        let auto = opts
            .iter()
            .find(|o| o.key == "speller/auto_select")
            .unwrap();
        assert_eq!(auto.value, Some(OptionValue::Bool(false)));
        assert_eq!(auto.default, Some(OptionValue::Bool(true)));
        assert!(auto.overridden);

        let page = opts.iter().find(|o| o.key == "menu/page_size").unwrap();
        assert_eq!(page.value, Some(OptionValue::Number(9.0)));
        assert!(page.overridden);
        assert_eq!(page.group, "候选与选字");

        // 未覆盖的项：值等于默认值，overridden=false
        let completion = opts
            .iter()
            .find(|o| o.key == "translator/enable_completion")
            .unwrap();
        assert_eq!(completion.value, Some(OptionValue::Bool(false)));
        assert!(!completion.overridden);
    }

    #[test]
    fn option_catalog_has_no_duplicate_keys() {
        let mut seen = std::collections::HashSet::new();
        for spec in OPTION_SPECS {
            assert!(seen.insert(spec.key), "重复的配置键：{}", spec.key);
            assert!(
                spec.key.contains('/'),
                "配置键必须是 section/key：{}",
                spec.key
            );
            if spec.kind == OptionKind::Enum {
                assert!(!spec.choices.is_empty(), "枚举项必须给出取值：{}", spec.key);
            }
        }
    }

    #[test]
    fn writes_bool_int_float_and_text() {
        let updates = vec![
            (
                "speller/auto_select".to_owned(),
                Some(OptionValue::Bool(false)),
            ),
            ("menu/page_size".to_owned(), Some(OptionValue::Number(9.0))),
            (
                "translator/initial_quality".to_owned(),
                Some(OptionValue::Number(0.75)),
            ),
            (
                "speller/delimiter".to_owned(),
                Some(OptionValue::Text(" '".to_owned())),
            ),
        ];
        let out = patch_custom_values(None, &updates);
        assert!(out.contains("  speller/auto_select: false"));
        assert!(out.contains("  menu/page_size: 9"), "整数不带小数点");
        assert!(out.contains("  translator/initial_quality: 0.75"));
        assert!(out.contains("  speller/delimiter: ' '"), "字符串要加引号");
    }

    #[test]
    fn removing_a_key_deletes_the_patch_line() {
        let existing = "patch:\n  speller/auto_select: false\n  menu/page_size: 9\n";
        let out = patch_custom_values(Some(existing), &[("speller/auto_select".to_owned(), None)]);
        assert!(!out.contains("speller/auto_select"), "应删除该键");
        assert!(out.contains("menu/page_size: 9"), "其他键要保留");
    }

    #[test]
    fn patching_twice_is_idempotent() {
        let updates = vec![(
            "speller/max_code_length".to_owned(),
            Some(OptionValue::Number(6.0)),
        )];
        let once = patch_custom_values(None, &updates);
        let twice = patch_custom_values(Some(&once), &updates);
        assert_eq!(once, twice);
        assert_eq!(twice.matches("speller/max_code_length").count(), 1);
    }

    #[test]
    fn quoted_patch_keys_are_recognized() {
        let custom = "patch:\n  \"speller/auto_select\": false\n";
        assert_eq!(
            read_patch_raw(custom, "speller/auto_select").as_deref(),
            Some("false")
        );
    }

    #[test]
    fn values_roundtrip_through_yaml() {
        let updates = vec![
            (
                "speller/auto_clear".to_owned(),
                Some(OptionValue::Text("manual".to_owned())),
            ),
            (
                "speller/auto_select_pattern".to_owned(),
                Some(OptionValue::Text("^;.$|^\\w{4}$".to_owned())),
            ),
        ];
        let out = patch_custom_values(None, &updates);
        assert_eq!(
            read_patch_raw(&out, "speller/auto_clear").as_deref(),
            Some("manual")
        );
        assert_eq!(
            read_patch_raw(&out, "speller/auto_select_pattern").as_deref(),
            Some("^;.$|^\\w{4}$")
        );
    }
}
