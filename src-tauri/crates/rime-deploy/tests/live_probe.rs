//! 真机 D-Bus **只读**冒烟测试（默认 `#[ignore]`，CI 不受影响）。
//!
//! 只调用 `NameHasOwner` / `GetCurrentSchema` 这类只读方法，**绝不触发真实部署**
//! （不会调用 `SetConfig` / `ReloadAddonConfig`，不写任何 Rime 目录）。
//!
//! 手动运行：
//! ```text
//! cargo test -p rime-deploy --test live_probe -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::time::Duration;

use rime_deploy::{
    DbusCallOutcome, DbusClient, Deployer, Fcitx5Deployer, GdbusClient, PathBinaryLocator,
    SystemProcessRunner, FCITX_SERVICE,
};

#[test]
#[ignore = "需要真实 fcitx5 会话总线；只读，不触发部署"]
fn live_probe_reports_fcitx5_deploy_capability() {
    let deployer = Fcitx5Deployer::system();
    let capability = deployer.capability();
    println!("DeployCapability = {capability:?}");
    assert!(capability.supported, "本机应至少有一种可用部署方式");
    assert!(
        capability.method == "dbus-setconfig" || capability.method == "rime-deployer",
        "unexpected method: {}",
        capability.method
    );
}

#[test]
#[ignore = "需要真实会话总线；仅 NameHasOwner 只读查询"]
fn live_gdbus_name_has_owner_is_read_only() {
    let client = GdbusClient::new(
        Arc::new(SystemProcessRunner::new()),
        Arc::new(PathBinaryLocator::default()),
    );
    let outcome = client.name_has_owner(FCITX_SERVICE, Duration::from_secs(3));
    println!("NameHasOwner({FCITX_SERVICE}) = {outcome:?}");
    match outcome {
        DbusCallOutcome::Ok { .. } => {}
        DbusCallOutcome::Unavailable { .. } => {
            println!("没有可用的会话总线，跳过（无图形会话时属正常）");
        }
        DbusCallOutcome::ServiceUnknown { .. } => {
            println!("总线可用但 fcitx5 未运行");
        }
        other => panic!("NameHasOwner 出现意外结果：{other:?}"),
    }
}

#[test]
#[ignore = "需要 fcitx5-rime 正在运行；只读状态查询"]
fn live_get_current_schema_is_read_only() {
    let deployer = Fcitx5Deployer::system();
    match deployer.query_current_schema() {
        Ok(Some(schema)) => {
            println!("GetCurrentSchema = {schema}");
            assert!(!schema.is_empty());
        }
        Ok(None) => println!("GetCurrentSchema 返回非字符串结果"),
        Err(err) => println!("状态查询不可用（rime 附加组件未激活时属预期）：{err}"),
    }
}

/// 只读诊断：本机 build/ 产物是否已不比词库源文件旧。
///
/// 这解释了「部署命令成功但 build/ 无变化」的现象：librime 增量部署在
/// 词库未改动时会跳过重新编译，此时产物无变化属于**已生效**而非失败。
#[test]
#[ignore = "需要本机真实 fcitx5-rime 环境；只读，不触发部署"]
fn live_report_build_freshness() {
    let user_dir = std::path::PathBuf::from(std::env::var("HOME").expect("需要 HOME"))
        .join(".local/share/fcitx5/rime");
    if !user_dir.is_dir() {
        eprintln!("跳过：未找到 {}", user_dir.display());
        return;
    }
    let build_dir = user_dir.join("build");
    let up_to_date =
        rime_deploy::build_is_up_to_date(&user_dir, &build_dir).expect("新鲜度诊断失败");
    println!("用户目录: {}", user_dir.display());
    println!("构建目录: {}", build_dir.display());
    println!("产物是否已最新: {up_to_date}");
    if up_to_date {
        println!("→ 结论：此时部署会「成功但产物无变化」，属正常（无需重新编译）");
    } else {
        println!("→ 结论：产物落后于词库源文件，此时部署应当会重写 build/");
    }
}
