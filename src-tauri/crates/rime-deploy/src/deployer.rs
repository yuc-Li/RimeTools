//! 部署方式抽象与 Linux fcitx5 实现（PRD FR-5.4）。
//!
//! [`Fcitx5Deployer`] 按优先级尝试：
//! 1. D-Bus `Controller1.SetConfig("fcitx://config/addon/rime/deploy", "")`
//! 2. D-Bus `Controller1.ReloadAddonConfig("rime")`
//! 3. 子进程 `rime_deployer --build <user_dir> <shared_dir> <build_dir>` + 提示重启 fcitx5
//!
//! **严禁 `fcitx5-remote -r`**：它只重读 fcitx5 全局配置，不会让 Rime 重载 `build/`。
//!
//! 三种方式的共同前提是「命令真的执行了」，部署是否**生效**由
//! [`crate::controller::DeployController`] 用 `build/` 产物快照判定（FR-5.5）。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rime_core::types::{AppResult, DeployCapability, DeployTrigger};
use tracing::debug;

use crate::dbus::{
    parse_bool_reply, parse_single_string, DbusCallOutcome, DbusClient, GdbusClient,
    DEPLOY_CONFIG_URI, FCITX_SERVICE,
};
use crate::error;
use crate::process::{
    BinaryLocator, PathBinaryLocator, ProcessCommand, ProcessOutcome, ProcessRunner,
    SystemProcessRunner,
};

/// 一次部署的上下文（不可变；`deadline` 是整轮部署的时间上限）。
#[derive(Debug, Clone)]
pub struct DeployContext {
    pub trigger: DeployTrigger,
    pub user_dir: PathBuf,
    /// fcitx5-rime 共享数据目录（通常是 `/usr/share/rime-data`）。
    pub shared_dir: PathBuf,
    /// 用户目录下的 `build/`。
    pub build_dir: PathBuf,
    pub deadline: Instant,
}

impl DeployContext {
    /// 距离整轮 deadline 的剩余时间。
    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    pub fn expired(&self) -> bool {
        self.remaining().is_zero()
    }
}

/// 部署方式执行成功后的元信息（用于结果提示）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployOutcome {
    /// `dbus-setconfig` / `dbus-reload-addon-config` / `rime-deployer` / `mock`。
    pub method: String,
    /// 面向用户的补充说明（如「请重启 fcitx5 使部署生效」）。
    pub note: Option<String>,
}

impl DeployOutcome {
    pub fn new(method: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            note: None,
        }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

/// 部署方式抽象。上层只依赖此 trait，平台/前端差异由具体实现隔离。
pub trait Deployer: Send + Sync {
    /// 稳定的方法标识（写入部署结果日志）。
    fn method_name(&self) -> &'static str;

    /// 执行部署。`Ok` 仅表示「部署命令已被接受/执行」，产物是否变化由上层校验。
    fn deploy(&self, ctx: &DeployContext) -> AppResult<DeployOutcome>;

    /// 当前环境可用的部署能力（ipc §2 `DeployCapability`）。
    fn capability(&self) -> DeployCapability {
        DeployCapability {
            supported: true,
            method: self.method_name().to_owned(),
            note: None,
        }
    }
}

/// Linux / fcitx5-rime 的真实部署实现。
pub struct Fcitx5Deployer {
    dbus: Arc<dyn DbusClient>,
    runner: Arc<dyn ProcessRunner>,
    locator: Arc<dyn BinaryLocator>,
}

impl Fcitx5Deployer {
    /// 用系统默认组件构造（gdbus + 真实子进程 + PATH 查找）。
    pub fn system() -> Self {
        let runner: Arc<dyn ProcessRunner> = Arc::new(SystemProcessRunner::new());
        let locator: Arc<dyn BinaryLocator> = Arc::new(PathBinaryLocator::default());
        let dbus: Arc<dyn DbusClient> =
            Arc::new(GdbusClient::new(Arc::clone(&runner), Arc::clone(&locator)));
        Self {
            dbus,
            runner,
            locator,
        }
    }

    /// 注入依赖（测试用）。
    pub fn new(
        dbus: Arc<dyn DbusClient>,
        runner: Arc<dyn ProcessRunner>,
        locator: Arc<dyn BinaryLocator>,
    ) -> Self {
        Self {
            dbus,
            runner,
            locator,
        }
    }

    /// 部署状态查询：D-Bus `org.fcitx.Fcitx.Rime1.GetCurrentSchema`（PRD FR-5.5）。
    ///
    /// 返回当前方案 id（如 `xhup`）；前端不可用时返回 `DeployUnavailable`，
    /// 上层据此降级（隐藏状态卡片 / 提示手动部署）。
    pub fn query_current_schema(&self) -> AppResult<Option<String>> {
        match self.dbus.get_current_schema(Duration::from_secs(3)) {
            DbusCallOutcome::Ok { reply } => Ok(parse_single_string(&reply)),
            DbusCallOutcome::ServiceUnknown { detail } => Err(error::unavailable(format!(
                "无法查询部署状态：fcitx5-rime 未运行或未加载 Rime 附加组件（{detail}）"
            ))),
            DbusCallOutcome::InterfaceUnknown { detail } => Err(error::unavailable(format!(
                "当前 fcitx5-rime 版本不支持状态查询接口（{detail}）"
            ))),
            DbusCallOutcome::Unavailable { detail } => {
                Err(error::unavailable(format!("D-Bus 不可用：{detail}")))
            }
            DbusCallOutcome::TimedOut { detail } => {
                Err(error::timed_out(format!("查询部署状态超时（{detail}）")))
            }
            DbusCallOutcome::Error { detail, .. } => Err(error::failed(
                "查询部署状态失败",
                detail,
                "请重启 fcitx5 后重试；不影响手动部署",
            )),
        }
    }

    fn fcitx5_online(&self, ctx: &DeployContext) -> bool {
        let timeout = ctx
            .remaining()
            .min(Duration::from_secs(2))
            .max(Duration::from_millis(100));
        match self.dbus.name_has_owner(FCITX_SERVICE, timeout) {
            DbusCallOutcome::Ok { reply } => parse_bool_reply(&reply).unwrap_or(false),
            _ => false,
        }
    }

    fn run_rime_deployer(
        &self,
        program: &Path,
        ctx: &DeployContext,
        attempts: &[String],
    ) -> AppResult<DeployOutcome> {
        let remaining = ctx.remaining();
        if remaining.is_zero() {
            return Err(error::timed_out(
                "部署超时：执行 rime_deployer 前已耗尽时间预算",
            ));
        }
        let command = ProcessCommand::new(program.to_string_lossy().to_string()).args([
            "--build".to_owned(),
            ctx.user_dir.to_string_lossy().to_string(),
            ctx.shared_dir.to_string_lossy().to_string(),
            ctx.build_dir.to_string_lossy().to_string(),
        ]);
        debug!(program = %program.display(), args = ?command.args, "执行 rime_deployer 兜底部署");

        match self.runner.run(&command, remaining) {
            ProcessOutcome::Exited { code: Some(0), .. } => Ok(DeployOutcome::new("rime-deployer").with_note(
                "已通过 rime_deployer 直接构建 build/；请重启 fcitx5 让输入法重载 build/ 后生效",
            )),
            ProcessOutcome::Exited { code, stdout, stderr } => {
                let code_text = code.map(|code| code.to_string()).unwrap_or_else(|| "被信号终止".to_owned());
                Err(error::failed(
                    format!("rime_deployer 构建失败（退出码 {code_text}）"),
                    format!(
                        "D-Bus 尝试：{}\nstderr:\n{}\nstdout:\n{}",
                        attempts.join("；"),
                        truncate(&stderr, 2000),
                        truncate(&stdout, 2000)
                    ),
                    "请检查 build/ 目录权限与词库文件格式后重试；也可在终端手动执行 \
                     rime_deployer --build <用户目录> /usr/share/rime-data <用户目录>/build",
                ))
            }
            ProcessOutcome::TimedOut { .. } => Err(error::timed_out("rime_deployer 构建超时")),
            ProcessOutcome::NotFound { program } => Err(error::unavailable(format!("未找到可执行文件 {program}"))),
            ProcessOutcome::Failed { message } => Err(error::failed(
                "无法执行 rime_deployer",
                message,
                "请确认 rime_deployer 已安装且当前用户可执行",
            )),
        }
    }
}

impl Deployer for Fcitx5Deployer {
    fn method_name(&self) -> &'static str {
        "dbus-setconfig"
    }

    fn deploy(&self, ctx: &DeployContext) -> AppResult<DeployOutcome> {
        let mut attempts: Vec<String> = Vec::new();
        let mut last_dbus_outcome: Option<DbusCallOutcome> = None;

        if self.fcitx5_online(ctx) {
            // ① SetConfig("fcitx://config/addon/rime/deploy", "")
            let remaining = ctx.remaining();
            if remaining.is_zero() {
                return Err(error::timed_out(
                    "部署超时：调用 fcitx5 D-Bus 前已耗尽时间预算",
                ));
            }
            match self.dbus.set_config(DEPLOY_CONFIG_URI, "", remaining) {
                DbusCallOutcome::Ok { .. } => {
                    return Ok(DeployOutcome::new("dbus-setconfig")
                        .with_note("已通过 fcitx5 D-Bus 触发 Rime 重新部署"));
                }
                outcome if is_timeout(&outcome) => {
                    // 超时说明前端无响应：立刻失败，不要再叠加 ReloadAddonConfig 造成重复触发。
                    return Err(deploy_timeout_error("SetConfig", &outcome));
                }
                outcome => attempts.push(format!("SetConfig: {}", describe(&outcome))),
            }

            // ② ReloadAddonConfig("rime")
            let remaining = ctx.remaining();
            if remaining.is_zero() {
                return Err(error::timed_out(
                    "部署超时：调用 ReloadAddonConfig 前已耗尽时间预算",
                ));
            }
            match self.dbus.reload_addon_config("rime", remaining) {
                DbusCallOutcome::Ok { .. } => {
                    return Ok(DeployOutcome::new("dbus-reload-addon-config")
                        .with_note("已通过 fcitx5 D-Bus 重载 Rime 附加组件"));
                }
                outcome if is_timeout(&outcome) => {
                    return Err(deploy_timeout_error("ReloadAddonConfig", &outcome));
                }
                outcome => {
                    attempts.push(format!("ReloadAddonConfig: {}", describe(&outcome)));
                    last_dbus_outcome = Some(outcome);
                }
            }
        } else {
            attempts.push("D-Bus: fcitx5 不在会话总线上".to_owned());
        }

        // ③ 兜底：rime_deployer 直接构建 build/
        if let Some(program) = self.locator.locate("rime_deployer") {
            return self.run_rime_deployer(&program, ctx, &attempts);
        }

        // 没有任何可用方式：区分「前端不可用」与「部署执行失败」。
        let unavailable = match &last_dbus_outcome {
            Some(outcome) => matches!(
                outcome,
                DbusCallOutcome::Unavailable { .. }
                    | DbusCallOutcome::ServiceUnknown { .. }
                    | DbusCallOutcome::InterfaceUnknown { .. }
            ),
            None => true,
        };
        if unavailable {
            Err(error::unavailable(format!(
                "无法触发部署：{}",
                attempts.join("；")
            )))
        } else {
            Err(error::failed(
                "部署失败",
                attempts.join("；"),
                "请重启 fcitx5 后重试，或手动执行 rime_deployer --build <用户目录> /usr/share/rime-data <用户目录>/build",
            ))
        }
    }

    fn capability(&self) -> DeployCapability {
        match self
            .dbus
            .name_has_owner(FCITX_SERVICE, Duration::from_secs(2))
        {
            DbusCallOutcome::Ok { reply } if parse_bool_reply(&reply).unwrap_or(false) => {
                DeployCapability {
                    supported: true,
                    method: "dbus-setconfig".to_owned(),
                    note: Some("通过 fcitx5 D-Bus 触发 Rime 重新部署".to_owned()),
                }
            }
            _ => {
                if self.locator.locate("rime_deployer").is_some() {
                    DeployCapability {
                        supported: true,
                        method: "rime-deployer".to_owned(),
                        note: Some(
                            "D-Bus 不可用，将直接执行 rime_deployer 构建 build/；完成后需重启 fcitx5 才会生效"
                                .to_owned(),
                        ),
                    }
                } else {
                    DeployCapability {
                        supported: false,
                        method: "none".to_owned(),
                        note: Some(
                            "未检测到 fcitx5 D-Bus 或 rime_deployer，请手动部署并重启 fcitx5"
                                .to_owned(),
                        ),
                    }
                }
            }
        }
    }
}

/// 把 D-Bus 结果转成可读的一行（写进错误 detail / 日志）。
fn describe(outcome: &DbusCallOutcome) -> String {
    match outcome {
        DbusCallOutcome::Ok { .. } => "成功".to_owned(),
        DbusCallOutcome::Unavailable { detail } => format!("不可用（{detail}）"),
        DbusCallOutcome::ServiceUnknown { detail } => format!("服务不存在（{detail}）"),
        DbusCallOutcome::InterfaceUnknown { detail } => format!("接口不支持（{detail}）"),
        DbusCallOutcome::TimedOut { detail } => format!("超时（{detail}）"),
        DbusCallOutcome::Error { name, detail } => {
            format!("错误 {}（{detail}）", name.as_deref().unwrap_or("未知"))
        }
    }
}

fn is_timeout(outcome: &DbusCallOutcome) -> bool {
    matches!(outcome, DbusCallOutcome::TimedOut { .. })
}

fn deploy_timeout_error(method: &str, outcome: &DbusCallOutcome) -> rime_core::types::AppError {
    error::timed_out(format!(
        "D-Bus {method} 调用超时，前端无响应（{}）",
        describe(outcome)
    ))
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push('…');
    out
}

/// 供上层在 UI 里展示部署能力的便捷函数（探测系统默认环境）。
pub fn probe_linux_deploy_capability() -> DeployCapability {
    Fcitx5Deployer::system().capability()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{DbusCall, MockDbusClient, MockProcessRunner, StaticBinaryLocator};
    use rime_core::types::ErrorCode;

    fn context() -> DeployContext {
        DeployContext {
            trigger: DeployTrigger::Manual,
            user_dir: PathBuf::from("/tmp/rime-user"),
            shared_dir: PathBuf::from("/usr/share/rime-data"),
            build_dir: PathBuf::from("/tmp/rime-user/build"),
            deadline: Instant::now() + Duration::from_secs(5),
        }
    }

    fn make_deployer(
        dbus: Arc<MockDbusClient>,
        runner: Arc<MockProcessRunner>,
        binaries: &[&str],
    ) -> Fcitx5Deployer {
        Fcitx5Deployer::new(dbus, runner, Arc::new(StaticBinaryLocator::with(binaries)))
    }

    #[test]
    fn prefers_dbus_set_config() {
        let dbus = Arc::new(MockDbusClient::new());
        let runner = Arc::new(MockProcessRunner::new());
        let outcome = make_deployer(Arc::clone(&dbus), Arc::clone(&runner), &["rime_deployer"])
            .deploy(&context())
            .expect("应成功");
        assert_eq!(outcome.method, "dbus-setconfig");
        assert_eq!(
            dbus.calls(),
            vec![
                DbusCall::NameHasOwner(FCITX_SERVICE.to_owned()),
                DbusCall::SetConfig {
                    uri: DEPLOY_CONFIG_URI.to_owned(),
                    value: String::new()
                },
            ]
        );
        assert!(
            runner.commands().is_empty(),
            "首选成功时不得执行 rime_deployer"
        );
    }

    #[test]
    fn falls_back_to_reload_addon_config() {
        let dbus = Arc::new(MockDbusClient::new());
        dbus.set_set_config_outcome(DbusCallOutcome::ServiceUnknown {
            detail: "no such method".to_owned(),
        });
        let runner = Arc::new(MockProcessRunner::new());
        let outcome = make_deployer(Arc::clone(&dbus), Arc::clone(&runner), &["rime_deployer"])
            .deploy(&context())
            .expect("应成功");
        assert_eq!(outcome.method, "dbus-reload-addon-config");
        assert_eq!(
            dbus.calls(),
            vec![
                DbusCall::NameHasOwner(FCITX_SERVICE.to_owned()),
                DbusCall::SetConfig {
                    uri: DEPLOY_CONFIG_URI.to_owned(),
                    value: String::new()
                },
                DbusCall::ReloadAddonConfig("rime".to_owned()),
            ]
        );
        assert!(runner.commands().is_empty());
    }

    #[test]
    fn falls_back_to_rime_deployer_with_exact_args() {
        let dbus = Arc::new(MockDbusClient::new());
        dbus.set_set_config_outcome(DbusCallOutcome::ServiceUnknown {
            detail: String::new(),
        });
        dbus.set_reload_outcome(DbusCallOutcome::InterfaceUnknown {
            detail: String::new(),
        });
        let runner = Arc::new(MockProcessRunner::new()); // 默认 exit 0
        let outcome = make_deployer(Arc::clone(&dbus), Arc::clone(&runner), &["rime_deployer"])
            .deploy(&context())
            .expect("应成功");
        assert_eq!(outcome.method, "rime-deployer");
        assert!(outcome
            .note
            .as_deref()
            .unwrap_or_default()
            .contains("重启 fcitx5"));
        let commands = runner.commands();
        assert_eq!(commands.len(), 1);
        assert!(commands[0].program.ends_with("rime_deployer"));
        assert_eq!(
            commands[0].args,
            vec![
                "--build".to_owned(),
                "/tmp/rime-user".to_owned(),
                "/usr/share/rime-data".to_owned(),
                "/tmp/rime-user/build".to_owned(),
            ]
        );
    }

    #[test]
    fn reports_unavailable_when_nothing_works() {
        let dbus = Arc::new(MockDbusClient::new());
        dbus.set_name_has_owner(false);
        let runner = Arc::new(MockProcessRunner::new());
        let err = make_deployer(Arc::clone(&dbus), runner, &[])
            .deploy(&context())
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::DeployUnavailable);
        assert!(err
            .hint
            .as_deref()
            .unwrap_or_default()
            .contains("rime_deployer"));
    }

    #[test]
    fn set_config_timeout_short_circuits() {
        let dbus = Arc::new(MockDbusClient::new());
        dbus.set_set_config_outcome(DbusCallOutcome::TimedOut {
            detail: "dbus busy".to_owned(),
        });
        let err = make_deployer(
            Arc::clone(&dbus),
            Arc::new(MockProcessRunner::new()),
            &["rime_deployer"],
        )
        .deploy(&context())
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::DeployTimeout);
        // 超时后不应再尝试 ReloadAddonConfig / 兜底（否则同一轮会重复触发部署）。
        assert_eq!(dbus.calls().len(), 2, "calls: {:?}", dbus.calls());
    }

    #[test]
    fn offline_dbus_uses_rime_deployer() {
        let dbus = Arc::new(MockDbusClient::new());
        dbus.set_name_has_owner(false);
        let runner = Arc::new(MockProcessRunner::new());
        let outcome = make_deployer(Arc::clone(&dbus), Arc::clone(&runner), &["rime_deployer"])
            .deploy(&context())
            .expect("应成功");
        assert_eq!(outcome.method, "rime-deployer");
        assert_eq!(
            dbus.calls(),
            vec![DbusCall::NameHasOwner(FCITX_SERVICE.to_owned())]
        );
    }

    #[test]
    fn rime_deployer_nonzero_exit_is_deploy_failed() {
        let dbus = Arc::new(MockDbusClient::new());
        dbus.set_name_has_owner(false);
        let runner = Arc::new(MockProcessRunner::new());
        runner.push_outcome(ProcessOutcome::Exited {
            code: Some(2),
            stdout: String::new(),
            stderr: "invalid dict yaml".to_owned(),
        });
        let err = make_deployer(Arc::clone(&dbus), runner, &["rime_deployer"])
            .deploy(&context())
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::DeployFailed);
        assert!(err
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("invalid dict yaml"));
    }

    #[test]
    fn capability_prefers_dbus_then_deployer_then_none() {
        let dbus = Arc::new(MockDbusClient::new());
        let deployer = make_deployer(
            Arc::clone(&dbus),
            Arc::new(MockProcessRunner::new()),
            &["rime_deployer"],
        );
        let capability = deployer.capability();
        assert!(capability.supported);
        assert_eq!(capability.method, "dbus-setconfig");

        dbus.set_name_has_owner(false);
        let capability = deployer.capability();
        assert!(capability.supported);
        assert_eq!(capability.method, "rime-deployer");

        let none = make_deployer(
            Arc::new(MockDbusClient::new_with_owner(false)),
            Arc::new(MockProcessRunner::new()),
            &[],
        )
        .capability();
        assert!(!none.supported);
        assert_eq!(none.method, "none");
    }

    #[test]
    fn query_current_schema_returns_active_schema() {
        let dbus = Arc::new(MockDbusClient::new());
        let deployer = make_deployer(dbus, Arc::new(MockProcessRunner::new()), &[]);
        assert_eq!(
            deployer.query_current_schema().expect("应成功").as_deref(),
            Some("xhup")
        );
    }

    #[test]
    fn query_current_schema_degrades_when_service_missing() {
        let dbus = Arc::new(MockDbusClient::new());
        dbus.set_current_schema_outcome(DbusCallOutcome::ServiceUnknown {
            detail: "Rime1 down".to_owned(),
        });
        let deployer = make_deployer(dbus, Arc::new(MockProcessRunner::new()), &[]);
        let err = deployer.query_current_schema().unwrap_err();
        assert_eq!(err.code, ErrorCode::DeployUnavailable);
    }

    #[test]
    fn context_remaining_is_bounded() {
        let mut ctx = context();
        ctx.deadline = Instant::now();
        assert!(ctx.expired());
        assert!(ctx.remaining().is_zero());
    }
}
