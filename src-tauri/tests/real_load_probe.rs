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
