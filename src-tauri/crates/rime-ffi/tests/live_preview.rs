//! 真实 librime 集成测试（需手动运行）：
//!
//! ```bash
//! cargo test -p rime-ffi --test live_preview -- --ignored --nocapture
//! ```
//!
//! 这些测试会真正 dlopen 系统 librime、在**临时目录**里部署一个最小方案并跑隔离会话。
//! 它们不会、也不需要访问用户真实 Rime 目录。
//!
//! 非 `#[ignore]` 的测试只做“库缺失时优雅报错”的检查，不依赖 librime 存在。
//!
//! 注意：librime 的 setup/initialize/finalize 是进程级全局状态，本文件内的真实集成
//! 测试用 [`SERIAL`] 互斥量串行执行（Rust 测试默认并行）。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rime_ffi::{
    last_deploy_state, probe_librime, FfiError, HomeContent, HomeSpec, IsolatedRimeHome,
    PreviewMode, RimePreview,
};

/// librime 的 initialize/finalize 是进程级全局状态：Rust 测试默认并行，
/// 所以同一测试二进制内的真实集成测试必须串行。
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("preview_demo")
}

/// 最小 home：方案/码表都从 crate 内夹具复制到临时目录。
fn minimal_spec(disable_auto_select: bool) -> HomeSpec {
    let fixtures = fixtures_dir();
    let spec = HomeSpec::new("preview_demo")
        .shared_file(HomeContent::copy(
            fixtures.join("default.yaml"),
            "default.yaml",
        ))
        .shared_file(HomeContent::copy(
            fixtures.join("preview_demo.schema.yaml"),
            "preview_demo.schema.yaml",
        ))
        .shared_file(HomeContent::copy(
            fixtures.join("preview_demo.dict.yaml"),
            "preview_demo.dict.yaml",
        ));
    if disable_auto_select {
        spec
    } else {
        spec.keep_auto_select()
    }
}

fn open_preview(disable_auto_select: bool) -> RimePreview {
    let home =
        IsolatedRimeHome::prepare(minimal_spec(disable_auto_select)).expect("准备隔离预览目录失败");
    RimePreview::open(home).expect("初始化 librime 隔离会话失败")
}

#[test]
fn probe_reports_cleanly_whether_or_not_librime_exists() {
    match probe_librime() {
        Ok(probe) => {
            println!(
                "librime 可用：{}（RimeApi data_size={}）",
                probe.path.display(),
                probe.api_data_size
            );
            assert!(!probe.path.as_os_str().is_empty());
            assert!(probe.api_data_size >= 100);
        }
        Err(FfiError::LibraryNotFound { .. }) | Err(FfiError::LibraryLoad { .. }) => {
            // 允许：本环境没有 librime 时必须优雅报错，不能 panic。
        }
        Err(other) => panic!("意外的探测错误：{other:?}"),
    }
}

#[test]
#[ignore = "需要系统 librime（1.16.1）+ 可写临时目录；由 lead 手动运行"]
fn live_preview_exact_code_with_auto_select_disabled() {
    let _serial = serial();
    let preview = open_preview(true);
    println!("librime: {}", preview.library_path().display());
    println!("隔离目录: {}", preview.home().root().display());
    println!("部署状态: {}", last_deploy_state());

    // ① 唯一候选的 4 码（真实 xhup 里最常见的音形全码）。
    let unique = preview
        .preview("abcd", Some("甲"))
        .expect("读取 abcd 候选失败");
    println!("preview(abcd) mode={:?}", unique.mode);
    for (index, item) in unique.items.iter().enumerate() {
        println!(
            "  #{} text={:?} comment={:?} weight={:?} is_target={}",
            index + 1,
            item.text,
            item.comment,
            item.weight,
            item.is_target
        );
    }
    println!("note={:?}", unique.note);

    assert_eq!(unique.mode, PreviewMode::Live);
    assert_eq!(
        unique.items.len(),
        1,
        "abcd 只有一个候选（甲），实际 {:?}",
        unique.items
    );
    assert_eq!(unique.items[0].text, "甲");
    assert!(unique.items[0].is_target, "甲 应与当前编辑文本匹配");
    assert!(unique.note.is_none(), "关闭 auto_select 后不应有回退说明");

    // ② 有重码的 4 码：auto_select 本来就不会触发（librime 只在唯一候选时自动上屏）。
    let multiple = preview.preview("abce", None).expect("读取 abce 候选失败");
    println!("preview(abce) items={:?}", multiple.items);
    assert_eq!(multiple.items.len(), 2);
    assert_eq!(multiple.items[0].text, "乙");
    assert_eq!(multiple.items[1].text, "丙");
    assert!(multiple.note.is_none());

    // ③ 3 码编码同样可读。
    let short = preview.preview("abc", None).expect("读取 3 码候选失败");
    println!("preview(abc) items={:?}", short.items);
    assert_eq!(short.items.len(), 1);
    assert_eq!(short.items[0].text, "丁");

    // ④ 无候选编码：Live + 空列表 + 说明，不报错。
    let none = preview.preview("zzz", None).expect("无候选编码不应报错");
    println!("preview(zzz) mode={:?} note={:?}", none.mode, none.note);
    assert_eq!(none.mode, PreviewMode::Live);
    assert!(none.items.is_empty());
    assert!(none.note.is_some());
}

#[test]
#[ignore = "需要系统 librime（1.16.1）+ 可写临时目录；由 lead 手动运行"]
fn live_preview_observes_xhup_style_auto_select_without_workaround() {
    let _serial = serial();
    // 故意保留方案里的 speller/auto_select: true，复现 xhup 4 码顶屏：
    // 第 4 键同步上屏 → context 清空 → 完整编码的候选读不到。
    let preview = open_preview(false);

    // 唯一候选「abcd」：第 4 码自动上屏「甲」，context 被清空。
    let result = preview
        .preview("abcd", Some("甲"))
        .expect("读取 abcd 候选失败");
    println!("preview(abcd) [auto_select=true] mode={:?}", result.mode);
    for item in &result.items {
        println!("  text={:?} is_target={}", item.text, item.is_target);
    }
    println!("note={:?}", result.note);

    assert_eq!(result.mode, PreviewMode::Live);
    let note = result.note.expect(
        "auto_select=true 且唯一候选时第 4 码会上屏、context 清空，必须给出回退说明；\
         若此处失败，说明 librime 的 auto_select 行为与预期不同，请把输出贴给 lead",
    );
    assert!(note.contains("自动上屏"), "note = {note}");
    assert!(note.contains("甲"), "note 应包含已上屏文本，实际 {note}");
    assert!(
        result.items.iter().any(|item| item.text == "丁"),
        "应回退到 abc 的候选（丁），实际 {:?}",
        result.items
    );
    assert!(
        !result.items.iter().any(|item| item.text == "甲"),
        "候选列表不应包含已被 auto_select 上屏的「甲」，实际 {:?}",
        result.items
    );

    // 重码「abce」不会触发 auto_select（librime 只在唯一候选时自动上屏），
    // 预览应正常拿到 2 个候选。
    let multiple = preview.preview("abce", None).expect("读取 abce 候选失败");
    println!(
        "preview(abce) [auto_select=true] items={:?} note={:?}",
        multiple.items, multiple.note
    );
    assert_eq!(multiple.items.len(), 2);
    assert_eq!(multiple.items[0].text, "乙");
    assert_eq!(multiple.items[1].text, "丙");
    assert!(multiple.note.is_none());
}
// ───────────────────────── 真实 xhup 方案副本（仓库 tests/fixtures/rime-crane-xhup） ─────────────────────────

/// 仓库根目录下的 xhup 夹具（rime-crane 方案的一部分）。
fn xhup_fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/rime-crane-xhup")
}

/// 系统共享数据目录（提供 default.yaml / punctuation.yaml / key_bindings.yaml 预设）。
fn system_shared_dir() -> PathBuf {
    PathBuf::from("/usr/share/rime-data")
}

/// 把真实 xhup 方案裁成“可在隔离目录里独立部署”的副本：
/// - 去掉本机没有的 lua 组件（xhup/shortcut_processor、date/time/shortcut translator）；
/// - 去掉 `dependencies: xhup_reverse` 与反查 translator；
/// - 去掉依赖 OpenCC 资产（系统未装 rime opencc 数据）的 simplifier 系列 filter；
/// - `import_tables` 只保留夹具里真实存在的三张表。
///
/// 保留下来的正是预览关心的部分：`speller/alphabet|initials|finals|max_code_length|
/// auto_select|auto_select_pattern|auto_clear`、`translator: table_translator`、`menu/page_size`。
fn sanitized_xhup_schema(original: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    let mut skipping_block = false;
    for line in original.lines() {
        // 跳过 unavailable 的顶层配置节（只影响被删掉的组件，留着也无害，这里一并清理）。
        if let Some(head) = line.strip_suffix(':') {
            if matches!(head, "xhup_reverse" | "short_hints" | "simplification") {
                skipping_block = true;
                continue;
            }
        }
        if skipping_block {
            if !line.starts_with(' ') && !line.starts_with('#') {
                skipping_block = false;
            } else {
                continue;
            }
        }
        if line.contains("lua_processor@") || line.contains("lua_translator@") {
            continue;
        }
        if line.trim() == "- reverse_lookup_translator@xhup_reverse"
            || line.trim() == "- history_translator@history"
        {
            continue;
        }
        if line.trim() == "dependencies:" || line.trim() == "- xhup_reverse" {
            continue;
        }
        if line.trim_start().starts_with("- simplifier") {
            continue;
        }
        lines.push(line);
    }
    let mut schema = lines.join("\n");
    schema.push('\n');
    schema
}

/// `import_tables` 只指向夹具真实存在的表。
fn sanitized_xhup_dict(original: &str) -> String {
    let mut output = String::new();
    let mut in_imports = false;
    for line in original.lines() {
        if line.starts_with("import_tables:") {
            in_imports = true;
            output.push_str(
                "import_tables:\n  - \"xhup_dicts/xhup.user.top\"\n  - \"xhup_dicts/xhup.primary\"\n  - \"xhup_dicts/xhup.user\"\n",
            );
            continue;
        }
        if in_imports {
            let trimmed = line.trim();
            if trimmed == "..." {
                in_imports = false;
                output.push_str(line);
                output.push('\n');
            }
            // 丢弃原有 import 行
            continue;
        }
        output.push_str(line);
        output.push('\n');
    }
    output
}

fn real_xhup_spec(disable_auto_select: bool) -> HomeSpec {
    let fixtures = xhup_fixtures();
    let schema = std::fs::read_to_string(fixtures.join("xhup.schema.yaml"))
        .expect("读取 xhup.schema.yaml 失败");
    let dict =
        std::fs::read_to_string(fixtures.join("xhup.dict.yaml")).expect("读取 xhup.dict.yaml 失败");
    let spec = HomeSpec::new("xhup")
        .preset_source(system_shared_dir())
        .shared_file(HomeContent::inline(
            "xhup.schema.yaml",
            sanitized_xhup_schema(&schema),
        ))
        .shared_file(HomeContent::inline(
            "xhup.dict.yaml",
            sanitized_xhup_dict(&dict),
        ))
        .shared_file(HomeContent::copy(
            fixtures.join("xhup.user.dict.yaml"),
            "xhup_dicts/xhup.user.dict.yaml",
        ))
        .shared_file(HomeContent::copy(
            fixtures.join("xhup.user.top.dict.yaml"),
            "xhup_dicts/xhup.user.top.dict.yaml",
        ))
        .shared_file(HomeContent::copy(
            fixtures.join("xhup.primary.sample.dict.yaml"),
            "xhup_dicts/xhup.primary.dict.yaml",
        ));
    if disable_auto_select {
        spec
    } else {
        spec.keep_auto_select()
    }
}

#[test]
#[ignore = "需要系统 librime（1.16.1）+ /usr/share/rime-data 预设；由 lead 手动运行"]
fn live_preview_real_xhup_fixture_with_and_without_auto_select_workaround() {
    let _serial = serial();
    if !xhup_fixtures().is_dir() {
        eprintln!(
            "跳过：未找到仓库夹具 {}（在 src-tauri/crates/rime-ffi 下运行测试）",
            xhup_fixtures().display()
        );
        return;
    }
    if !system_shared_dir().join("default.yaml").is_file() {
        eprintln!(
            "跳过：系统共享数据目录 {} 缺少 default.yaml",
            system_shared_dir().display()
        );
        return;
    }

    // ① 默认隔离预览目录（自动关闭 speller/auto_select）：4 码「anui」候选可读。
    {
        let preview = RimePreview::open(
            IsolatedRimeHome::prepare(real_xhup_spec(true)).expect("准备 xhup 隔离目录失败"),
        )
        .expect("初始化 xhup 隔离会话失败");
        println!(
            "xhup（auto_select 关闭）librime={} 隔离目录={} 部署={}",
            preview.library_path().display(),
            preview.home().root().display(),
            last_deploy_state()
        );
        let result = preview
            .preview("anui", Some("按时"))
            .expect("预览 anui 失败");
        println!("preview(anui) mode={:?}", result.mode);
        for item in &result.items {
            println!(
                "  text={:?} comment={:?} is_target={}",
                item.text, item.comment, item.is_target
            );
        }
        println!("note={:?}", result.note);
        assert_eq!(result.mode, PreviewMode::Live);
        assert!(
            result.items.iter().any(|item| item.text == "按时"),
            "anui 应能预览到「按时」，实际 {:?}",
            result.items
        );
        assert!(
            result
                .items
                .iter()
                .any(|item| item.text == "按时" && item.is_target),
            "「按时」应标记 is_target"
        );
    }

    // ② 保留真实方案的 speller/auto_select: true：4 码「anui」是唯一候选 → 立即上屏，
    //    context 清空，完整编码候选读不到；必须回退并说明（这是 xhup 4 码坑的真实复现）。
    {
        let preview = RimePreview::open(
            IsolatedRimeHome::prepare(real_xhup_spec(false)).expect("准备 xhup 隔离目录失败"),
        )
        .expect("初始化 xhup 隔离会话失败");
        let result = preview
            .preview("anui", Some("按时"))
            .expect("预览 anui 失败");
        println!("preview(anui) [auto_select=true] mode={:?}", result.mode);
        for item in &result.items {
            println!("  text={:?} is_target={}", item.text, item.is_target);
        }
        println!("note={:?}", result.note);
        let note = result.note.expect(
            "真实 xhup 的 4 码唯一候选应触发 auto_select 上屏；若没有 note，说明行为与预期不同",
        );
        assert!(note.contains("自动上屏"), "note = {note}");
        assert!(note.contains("按时"), "note 应包含已上屏文本，实际 {note}");
        assert!(
            !result.items.iter().any(|item| item.text == "按时"),
            "上屏文本不应再出现在候选列表，实际 {:?}",
            result.items
        );
    }
}
