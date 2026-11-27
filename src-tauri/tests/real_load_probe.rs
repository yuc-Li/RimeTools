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
