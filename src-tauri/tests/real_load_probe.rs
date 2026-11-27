//! 只读诊断：用真实 Rime 目录跑一遍工作台的加载链路（不含 Tauri 层）。
//! `cargo test -p rimetools --test real_load_probe -- --ignored --nocapture`

use rime_core::store::DictStore;
use rime_core::types::EntryQuery;

#[test]
#[ignore = "需要本机真实 Rime 环境；只读"]
fn real_workbench_load_chain() {
    let frontends = rime_platform::detect_frontends();
    assert!(!frontends.is_empty(), "应检测到前端");
    let user_dir = &frontends[0].user_dir;
    println!("用户目录: {}", user_dir.display());

    let schemes = rime_platform::detect_schemes(user_dir);
    println!("方案数: {}", schemes.len());
    for s in &schemes {
        println!(
            "  {} ({}) translator={:?} active={} 受管文件={}",
            s.name,
            s.schema_id,
            s.translator_kind,
            s.is_active,
            s.managed_files.len()
        );
        for mf in &s.managed_files {
            println!(
                "     [{:?}] {} 列={:?}",
                mf.role,
                mf.path.display(),
                mf.columns
            );
            match DictStore::open(&mf.path, mf.format, mf.role) {
                Ok(store) => {
                    println!("      打开成功 total={}", store.total());
                    // 命令层已按受管 id 路由，store 内不再按 file_id 过滤
                    let page = store.query(&EntryQuery {
                        file_id: None,
                        offset: 0,
                        limit: 5,
                        ..Default::default()
                    });
                    println!("      查询 total={} items={}", page.total, page.items.len());
                    for e in page.items.iter().take(3) {
                        println!(
                            "        {:?} code={} weight={:?}",
                            e.key.text, e.key.code, e.weight
                        );
                    }
                    assert!(
                        !(page.items.is_empty() && store.total() > 0),
                        "BUG：store 有条目但查询返回空（fileId 过滤未处理）"
                    );
                }
                Err(e) => println!("      打开失败: {e:?}"),
            }
        }
    }
}

/// M2 只读探针：反查索引与符号表在真实环境下是否可用。
#[test]
#[ignore = "需要本机真实 Rime 环境；只读"]
fn real_lookup_and_symbols() {
    use rime_core::lookup::{query, LookupMode, LookupQuery};
    use rime_platform::{sources, symbols};

    let frontends = rime_platform::detect_frontends();
    assert!(!frontends.is_empty(), "应检测到前端");
    let user_dir = &frontends[0].user_dir;
    let shared_dir = rime_platform::fcitx5_shared_dir();
    println!("用户目录: {}", user_dir.display());

    let schema_ids: Vec<String> = rime_platform::detect_schemes(user_dir)
        .into_iter()
        .map(|s| s.schema_id)
        .collect();
    println!("方案: {schema_ids:?}");

    // 按方案收集：小鹤音形应拿到 xhup_dicts/* 而不是别的方案
    let schemes = rime_platform::detect_schemes(user_dir);
    if let Some(xhup) = schemes.iter().find(|s| s.schema_id == "xhup") {
        let scoped = sources::scan_for_scheme(user_dir, xhup);
        println!("xhup 方案词表（{} 个）:", scoped.len());
        let mut total_scoped = 0u64;
        for f in &scoped {
            let loaded = rime_platform::sources::load(f);
            total_scoped += loaded.entry_count as u64;
            println!(
                "    {:<45} {:>8} 条",
                f.path.strip_prefix(user_dir).unwrap_or(&f.path).display(),
                loaded.entry_count
            );
        }
        println!("    scoped 条目合计: {total_scoped}");
        assert!(
            scoped.iter().any(|f| f.id.contains("xhup_dicts")),
            "应按 import_tables 找到 xhup_dicts 下的词表"
        );
        assert!(
            scoped.iter().all(|f| f.id.contains("xhup")),
            "不应混入其他方案"
        );
    }

    let files = sources::scan(user_dir, user_dir, &schema_ids);
    println!("可反查文件: {}", files.len());
    let loaded = sources::load_all(&files);
    let total: u64 = loaded.iter().map(|s| s.entry_count as u64).sum();
    for s in &loaded {
        println!(
            "  {}  {} 条  origin={:?} schema={:?} 错误={:?}",
            s.name, s.entry_count, s.origin, s.schema_id, s.error
        );
    }
    assert!(files.len() > 0, "至少应扫到一个词库文件");
    assert!(total > 0, "索引条目不应为空");

    // 按编码反查：随便取一条真实词条的编码
    let sample = loaded
        .iter()
        .flat_map(|s| s.entries.iter())
        .next()
        .expect("应至少有一条词");
    let result = query(
        &loaded,
        &LookupQuery {
            mode: LookupMode::Code,
            keyword: sample.code.clone(),
            exact: true,
            ..Default::default()
        },
    );
    println!("按编码 {:?} 反查命中 {} 条", sample.code, result.total);
    assert!(result.total >= 1, "按自身编码应能查到");

    let table = symbols::load(&shared_dir, user_dir, &[]);
    println!(
        "符号表条目: {} 来源={:?}",
        table.entries.len(),
        table.sources
    );
    assert!(!table.entries.is_empty(), "符号表不应为空");

    let hit = symbols::search(&table, "《", 5);
    println!(
        "符号《 的按键: {:?}",
        hit.iter().map(|h| h.key.clone()).collect::<Vec<_>>()
    );
}

/// M2 只读探针：方案配置的补丁逻辑能否正确处理真实文件格式。
///
/// 全程只读：用真实文件内容跑一遍纯函数，不写入任何文件。
#[test]
#[ignore = "需要本机真实 Rime 环境；只读"]
fn real_scheme_config_patch_handles_actual_files() {
    use rime_platform::scheme_config;

    let frontends = rime_platform::detect_frontends();
    assert!(!frontends.is_empty(), "应检测到前端");
    let user_dir = &frontends[0].user_dir;

    // ── 1) custom.yaml：真实模板里 patch 是被注释掉的 ──
    let custom_path = user_dir.join("xhup.custom.yaml");
    if let Ok(original) = std::fs::read_to_string(&custom_path) {
        println!("原始 custom 行数: {}", original.lines().count());
        let patched = scheme_config::patch_custom_yaml(Some(&original), Some(false), Some(4));

        assert!(
            patched.lines().any(|l| l.trim() == "patch:"),
            "应生成生效的 patch: 行"
        );
        assert!(
            patched.contains("speller/auto_select: false"),
            "应写入 auto_select"
        );
        assert!(
            patched.contains("speller/max_code_length: 4"),
            "应写入 max_code_length"
        );
        // 模板里的说明与注释必须保留
        assert!(
            patched.contains("# style/candidate_list_layout"),
            "注释应保留"
        );
        // 生效键只能出现一次（模板里被注释的示例行不算）
        let active_auto = patched
            .lines()
            .filter(|l| !l.trim_start().starts_with('#') && l.contains("speller/auto_select:"))
            .count();
        assert_eq!(active_auto, 1, "生效的 auto_select 只能有一行");
        // 被注释的示例行要原样保留
        assert!(
            patched
                .lines()
                .any(|l| l.trim_start().starts_with('#') && l.contains("speller/auto_select")),
            "模板注释行应保留"
        );

        // 幂等：同样输入再打一次补丁，结果不变
        let again = scheme_config::patch_custom_yaml(Some(&patched), Some(false), Some(4));
        assert_eq!(patched, again, "重复打补丁应幂等");

        println!("--- 补丁后前 12 行 ---");
        for line in patched.lines().take(12) {
            println!("{line}");
        }
    }

    // ── 1.5) 通用配置项：在真实方案上取值 ──
    let schema_path = user_dir.join("xhup.schema.yaml");
    if let Ok(schema_text) = std::fs::read_to_string(&schema_path) {
        let opts = scheme_config::read_options(&schema_text, None);
        println!("可配置项 {} 个：", opts.len());
        for opt in opts.iter().take(8) {
            println!(
                "  [{}] {} = {:?}（默认 {:?}）",
                opt.group, opt.label, opt.value, opt.default
            );
        }
        let max_len = opts
            .iter()
            .find(|o| o.key == "speller/max_code_length")
            .expect("目录里应有最大码长");
        assert_eq!(
            max_len.value,
            Some(rime_platform::scheme_config::OptionValue::Number(4.0)),
            "真实方案的 max_code_length 应为 4"
        );
        let auto = opts
            .iter()
            .find(|o| o.key == "speller/auto_select")
            .unwrap();
        assert_eq!(
            auto.value,
            Some(rime_platform::scheme_config::OptionValue::Bool(true)),
            "真实方案的 auto_select 应为 true"
        );
        // 方案里没有声明的项应为空值，而不是瞎猜
        let hints = opts
            .iter()
            .find(|o| o.key == "translator/spelling_hints")
            .unwrap();
        println!("  translator/spelling_hints = {:?}", hints.value);
    }

    // ── 2) 主词典：挂载项解析 + 开关往返 ──
    let dict_path = user_dir.join("xhup_dicts").join("xhup.primary.dict.yaml");
    let index_path = user_dir.join("xhup.dict.yaml");
    for path in [&index_path, &dict_path] {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let mounts = scheme_config::parse_mounts(&text);
        println!(
            "{}：解析出 {} 个挂载项",
            path.file_name().unwrap().to_string_lossy(),
            mounts.len()
        );
        for (name, enabled, _) in mounts.iter().take(6) {
            println!("  [{}] {name}", if *enabled { "开" } else { "关" });
        }
        if mounts.is_empty() {
            continue;
        }

        // 关掉再打开，应与原文逐字相同
        let first = &mounts[0].0;
        let off = scheme_config::toggle_mounts(&text, &[(first.clone(), false)]);
        if let Some(off_text) = off {
            assert_ne!(off_text, text, "关闭后内容应变化");
            assert!(
                off_text.contains(&format!("# - \"{first}\"")),
                "应通过行首注释实现停用"
            );
            let back = scheme_config::toggle_mounts(&off_text, &[(first.clone(), true)])
                .expect("重新启用应产生改动");
            assert_eq!(back, text, "关→开 应逐字还原原文");
            println!("  {first}：关→开 往返一致 ✓");
        }
    }
}
