//! 反查来源：扫描并加载可只读检索的词库文件。
//!
//! 范围（M2）：
//! - 用户目录下的 `*.dict.yaml`（含 `.user.dict.yaml` / `.user.top.dict.yaml`）与 `*.txt` 自定义短语；
//! - 系统共享目录下的 `*.dict.yaml`（只读预设，标记为 System）。
//!
//! 刻意排除：`build/`（编译产物、体积大且非源文件）、`sync/`（同步快照）、
//! `*.userdb/`（二进制目录）。

use std::path::{Path, PathBuf};

use rime_core::lookup::{LookupEntry, LookupSource, SourceOrigin};
use rime_core::types::{FileFormat, SchemeInfo};
use rime_core::ParsedDict;

/// 单个待加载的来源文件。
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// 绝对路径，作为稳定 id
    pub id: String,
    pub path: PathBuf,
    pub origin: SourceOrigin,
    /// 归属方案（按文件名前缀推断）
    pub schema_id: Option<String>,
    pub format: FileFormat,
}

fn format_label(format: FileFormat) -> &'static str {
    match format {
        FileFormat::RimeDictYaml => "dict.yaml",
        FileFormat::CustomPhrase => "custom_phrase",
        FileFormat::PlainTxt => "txt",
        FileFormat::Csv => "csv",
    }
}

/// 判断是否属于「用户目录里应纳入反查」的文件。
fn is_user_candidate(name: &str) -> Option<FileFormat> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".dict.yaml") || lower.ends_with(".dict.yml") {
        return Some(FileFormat::RimeDictYaml);
    }
    if lower.ends_with(".txt") {
        // 自定义短语通常是 文本<Tab>编码[<Tab>权重]
        return Some(FileFormat::CustomPhrase);
    }
    None
}

fn is_system_candidate(name: &str) -> Option<FileFormat> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".dict.yaml") || lower.ends_with(".dict.yml") {
        return Some(FileFormat::RimeDictYaml);
    }
    None
}

/// 按文件名推断归属方案：`xhup.user.dict.yaml` → `xhup`。
fn infer_schema(name: &str, known: &[String]) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let mut best: Option<String> = None;
    for schema in known {
        let s = schema.to_ascii_lowercase();
        if lower == format!("{s}.dict.yaml") || lower.starts_with(&format!("{s}.")) {
            match &best {
                Some(b) if b.len() >= schema.len() => {}
                _ => best = Some(schema.clone()),
            }
        }
    }
    best
}

fn push_dir(
    dir: &Path,
    origin: SourceOrigin,
    known_schemas: &[String],
    candidate: fn(&str) -> Option<FileFormat>,
    out: &mut Vec<SourceFile>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();

    for path in files {
        let name = match path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };
        // 跳过备份/临时文件
        if name.starts_with('.') || name.ends_with(".bak") || name.contains(".tmp") {
            continue;
        }
        let Some(format) = candidate(&name) else {
            continue;
        };
        out.push(SourceFile {
            id: path.to_string_lossy().to_string(),
            schema_id: infer_schema(&name, known_schemas),
            path,
            origin,
            format,
        });
    }
}

/// 扫描用户目录与系统目录，返回去重后的候选文件列表。
pub fn scan(user_dir: &Path, shared_dir: &Path, known_schemas: &[String]) -> Vec<SourceFile> {
    let mut out = Vec::new();
    push_dir(
        user_dir,
        SourceOrigin::User,
        known_schemas,
        is_user_candidate,
        &mut out,
    );
    if shared_dir != user_dir {
        push_dir(
            shared_dir,
            SourceOrigin::System,
            known_schemas,
            is_system_candidate,
            &mut out,
        );
    }
    out
}

/// 加载单个来源文件。解析失败不 panic，而是记在 `error` 里。
pub fn load(file: &SourceFile) -> LookupSource {
    let name = file
        .path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| file.id.clone());

    let mut source = LookupSource {
        id: file.id.clone(),
        name,
        origin: file.origin,
        schema_id: file.schema_id.clone(),
        format: format_label(file.format).to_owned(),
        entry_count: 0,
        error: None,
        entries: Vec::new(),
    };

    let bytes = match std::fs::read(&file.path) {
        Ok(b) => b,
        Err(e) => {
            source.error = Some(format!("读取失败：{e}"));
            return source;
        }
    };
    let text = match String::from_utf8(bytes) {
        Ok(t) => t,
        Err(_) => {
            source.error = Some("文件不是合法 UTF-8".to_owned());
            return source;
        }
    };

    let parsed = match ParsedDict::parse(&text, file.format) {
        Ok(p) => p,
        Err(e) => {
            source.error = Some(format!("解析失败：{e}"));
            return source;
        }
    };

    source.entries = parsed
        .entries()
        .map(|(idx, e)| LookupEntry {
            text: e.text.clone(),
            code: e.code.clone(),
            weight: e.weight,
            line_no: idx as u32 + 1,
        })
        .collect();
    source.entry_count = source.entries.len() as u32;
    source
}

/// 批量加载。
pub fn load_all(files: &[SourceFile]) -> Vec<LookupSource> {
    files.iter().map(load).collect()
}

// ───────────────────────── 按方案收集词表 ─────────────────────────

/// 一次扫描最多解析多少个文件（防御异常的 import_tables 递归）。
const MAX_FILES_PER_SCHEME: usize = 64;
/// import_tables 的最大递归深度。
const MAX_IMPORT_DEPTH: usize = 4;

/// 解析一个 import 名到实际文件路径。
///
/// `import_tables` 里写的是**不带扩展名的相对名**，且可能带子目录，
/// 例如 `xhup_dicts/xhup.user.top` → `xhup_dicts/xhup.user.top.dict.yaml`。
fn resolve_import(user_dir: &Path, name: &str) -> Option<PathBuf> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let base = user_dir.join(name);
    let mut candidates = Vec::new();
    if name.ends_with(".dict.yaml") || name.ends_with(".dict.yml") || name.ends_with(".yaml") {
        candidates.push(base);
    } else {
        // 注意：必须**追加**扩展名，不能用 `with_extension`。
        // `xhup_dicts/xhup.primary` 的 `.primary` 会被当成扩展名替换掉，
        // 结果找成 `xhup_dicts/xhup.dict.yaml`，静默漏掉整张主码表。
        candidates.push(PathBuf::from(format!("{}.dict.yaml", base.display())));
        candidates.push(PathBuf::from(format!("{}.yaml", base.display())));
        candidates.push(base);
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// 按**方案**收集它实际用到的词表。
///
/// 规则（与 Rime 的加载顺序一致）：
/// 1. 方案主词典（`translator/dictionary`）；
/// 2. 主词典 `import_tables` 里的每一张表，递归展开；
/// 3. 方案的个人词条文件（`managed_files`，如 custom_phrase / *.user.dict.yaml）。
///
/// 这样「小鹤音形」拿到的是 `xhup_dicts/*`（用户码表 + 主码表 + 次选…），
/// 而不是把别的方案的词表混进来。
pub fn scan_for_scheme(user_dir: &Path, scheme: &SchemeInfo) -> Vec<SourceFile> {
    let mut out: Vec<SourceFile> = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let schema_id = Some(scheme.schema_id.clone());

    let mut push_file = |path: PathBuf, format: FileFormat, out: &mut Vec<SourceFile>| {
        if out.len() >= MAX_FILES_PER_SCHEME {
            return;
        }
        let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if !seen.insert(canonical) {
            return;
        }
        out.push(SourceFile {
            id: path.to_string_lossy().to_string(),
            schema_id: schema_id.clone(),
            path,
            origin: SourceOrigin::User,
            format,
        });
    };

    // 1) 方案自己的受管文件（个人词条，最常用）
    for managed in &scheme.managed_files {
        if managed.path.is_file() {
            push_file(managed.path.clone(), managed.format, &mut out);
        }
    }

    // 2) 主词典 + import_tables（BFS）
    //
    // `SchemeInfo` 不带主词典名，需要回头读一遍方案文件（同一份文件，成本可忽略）。
    let mut queue: Vec<(PathBuf, usize)> = Vec::new();
    let schema_path = {
        let yaml = user_dir.join(format!("{}.schema.yaml", scheme.schema_id));
        let yml = user_dir.join(format!("{}.schema.yml", scheme.schema_id));
        if yaml.is_file() {
            Some(yaml)
        } else if yml.is_file() {
            Some(yml)
        } else {
            None
        }
    };
    if let Some(path) = schema_path {
        if let Ok(raw) = crate::parse_schema(&path) {
            if let Some(dict) = raw.primary_dictionary {
                if let Some(dict_path) = resolve_import(user_dir, &dict) {
                    queue.push((dict_path, 0));
                }
            }
        }
    }

    while let Some((path, depth)) = queue.pop() {
        if !path.is_file() {
            continue;
        }
        let before = out.len();
        push_file(path.clone(), FileFormat::RimeDictYaml, &mut out);
        // 已在集合里说明处理过（同时避免环）
        if out.len() == before {
            continue;
        }
        if depth >= MAX_IMPORT_DEPTH {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for import in crate::parse_import_tables(&text) {
            if let Some(child) = resolve_import(user_dir, &import) {
                queue.push((child, depth + 1));
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_schema_from_file_name() {
        let known = vec!["xhup".to_owned(), "double_pinyin_flypy".to_owned()];
        assert_eq!(
            infer_schema("xhup.user.dict.yaml", &known).as_deref(),
            Some("xhup")
        );
        assert_eq!(
            infer_schema("double_pinyin_flypy.user.dict.yaml", &known).as_deref(),
            Some("double_pinyin_flypy")
        );
        assert_eq!(infer_schema("unrelated.dict.yaml", &known), None);
    }

    #[test]
    fn scans_only_supported_files() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        std::fs::write(base.join("a.dict.yaml"), "---\nname: a\n...\n\n词\tci\n").unwrap();
        std::fs::write(base.join("b.txt"), "词\tci\n").unwrap();
        std::fs::write(base.join("c.yaml"), "schema:\n").unwrap();
        std::fs::write(base.join("d.dict.yaml.bak"), "x\n").unwrap();
        std::fs::create_dir(base.join("build")).unwrap();

        let files = scan(base, base, &[]);
        let names: Vec<_> = files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.dict.yaml", "b.txt"]);
    }

    #[test]
    fn loads_entries_with_line_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.dict.yaml");
        std::fs::write(&path, "---\nname: a\n...\n\n你好\tnihc\t2\n世界\tuijm\n").unwrap();
        let files = scan(dir.path(), dir.path(), &[]);
        let sources = load_all(&files);
        assert_eq!(sources.len(), 1);
        let s = &sources[0];
        assert_eq!(s.entry_count, 2);
        assert_eq!(s.entries[0].text, "你好");
        assert_eq!(s.entries[0].weight, Some(2));
        assert_eq!(s.entries[0].line_no, 5);
        assert!(s.error.is_none());
    }

    #[test]
    fn bad_utf8_is_reported_not_panicked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.dict.yaml");
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        let files = scan(dir.path(), dir.path(), &[]);
        let sources = load_all(&files);
        assert!(sources[0].error.is_some());
        assert_eq!(sources[0].entry_count, 0);
    }
}
