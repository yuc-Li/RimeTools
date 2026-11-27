//! fcitx5 D-Bus 接口封装（PRD FR-5.4 / §6.2）。
//!
//! 只封装两条链路需要的 4 个方法，全部通过 `gdbus` 子进程走会话总线：
//!
//! | 用途 | 目标 | 对象路径 | 方法 |
//! | --- | --- | --- | --- |
//! | 触发部署（首选） | `org.fcitx.Fcitx5` | `/controller` | `org.fcitx.Fcitx.Controller1.SetConfig` |
//! | 触发部署（次选） | `org.fcitx.Fcitx5` | `/controller` | `org.fcitx.Fcitx.Controller1.ReloadAddonConfig` |
//! | 部署状态查询 | `org.fcitx.Fcitx.Rime1` | `/rime` | `org.fcitx.Fcitx.Rime1.GetCurrentSchema` |
//! | 前端在线探测 | `org.freedesktop.DBus` | `/org/freedesktop/DBus` | `org.freedesktop.DBus.NameHasOwner` |
//!
//! **不使用 `fcitx5-remote -r`**：实测它只让 fcitx5 重读全局配置，不会让 Rime
//! 重载 `build/`（PRD FR-5.4 明确禁用）。
//!
//! ### 为什么走 `gdbus` 而不是 zbus / dbus crate
//!
//! * 本 crate 是 P0 关键路径，但 D-Bus 只用到 4 个固定方法；引入 zbus 会给整个工作区
//!   增加数十个传递依赖、拖慢构建并放大锁文件冲突面（多个子代理并行开发）。
//! * `dbus` crate 需要链接 libdbus-1 开发库，Windows 交叉构建不友好。
//! * `gdbus` 是 GLib 自带工具（Ubuntu 桌面默认存在），`process::BinaryLocator`
//!   探测不到时按设计降级到 `rime_deployer` 兜底，不会静默失败。
//!
//! 后续若要换成库绑定，只需替换 [`DbusClient`] 的实现，`Deployer`/状态机无需改动。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::process::{BinaryLocator, ProcessCommand, ProcessOutcome, ProcessRunner};

/// fcitx5 主服务名。
pub const FCITX_SERVICE: &str = "org.fcitx.Fcitx5";
/// fcitx5 控制器对象路径。
pub const CONTROLLER_PATH: &str = "/controller";
/// fcitx5 控制器接口。
pub const CONTROLLER_INTERFACE: &str = "org.fcitx.Fcitx.Controller1";
/// 触发 Rime 部署的 config URI（PRD FR-5.4 ①）。
pub const DEPLOY_CONFIG_URI: &str = "fcitx://config/addon/rime/deploy";
/// fcitx5-rime 状态服务名。
pub const RIME_SERVICE: &str = "org.fcitx.Fcitx.Rime1";
/// fcitx5-rime 状态对象路径。
pub const RIME_PATH: &str = "/rime";
/// fcitx5-rime 状态接口。
pub const RIME_INTERFACE: &str = "org.fcitx.Fcitx.Rime1";

/// D-Bus 方法调用结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbusCallOutcome {
    /// 调用成功，`reply` 为 gdbus 打印的返回值（如 `()`、`(true,)`、`('xhup',)`）。
    Ok { reply: String },
    /// 会话总线或 `gdbus` 不存在（前端整体不可用）。
    Unavailable { detail: String },
    /// 目标服务不在总线上（fcitx5 / rime 附加组件未运行）。
    ServiceUnknown { detail: String },
    /// 服务在，但接口/方法/参数不被支持（版本不匹配）。
    InterfaceUnknown { detail: String },
    /// 调用超时。
    TimedOut { detail: String },
    /// 其他 D-Bus 错误（服务端返回错误、授权失败等）。
    Error {
        name: Option<String>,
        detail: String,
    },
}

impl DbusCallOutcome {
    pub fn is_ok(&self) -> bool {
        matches!(self, DbusCallOutcome::Ok { .. })
    }
}

/// fcitx5 D-Bus 客户端抽象（可注入 mock，测试不碰真实会话总线）。
pub trait DbusClient: Send + Sync {
    /// `NameHasOwner(service)`，用于判断 fcitx5 是否在线。
    fn name_has_owner(&self, service: &str, timeout: Duration) -> DbusCallOutcome;
    /// `Controller1.SetConfig(DEPLOY_CONFIG_URI, "")`，触发 Rime 重新部署。
    fn set_config(&self, uri: &str, value: &str, timeout: Duration) -> DbusCallOutcome;
    /// `Controller1.ReloadAddonConfig("rime")`，次选部署方式。
    fn reload_addon_config(&self, addon: &str, timeout: Duration) -> DbusCallOutcome;
    /// `org.fcitx.Fcitx.Rime1.GetCurrentSchema`，部署状态查询。
    fn get_current_schema(&self, timeout: Duration) -> DbusCallOutcome;
}

/// 基于 `gdbus` 的真实实现。
#[derive(Clone)]
pub struct GdbusClient {
    runner: Arc<dyn ProcessRunner>,
    locator: Arc<dyn BinaryLocator>,
    program: String,
}

impl GdbusClient {
    pub fn new(runner: Arc<dyn ProcessRunner>, locator: Arc<dyn BinaryLocator>) -> Self {
        Self {
            runner,
            locator,
            program: "gdbus".to_owned(),
        }
    }

    /// 便于测试/自定义安装：指定 `gdbus` 可执行文件名或绝对路径。
    pub fn with_program(mut self, program: impl Into<String>) -> Self {
        self.program = program.into();
        self
    }

    fn call(
        &self,
        destination: &str,
        object_path: &str,
        interface: &str,
        method: &str,
        args: &[String],
        timeout: Duration,
    ) -> DbusCallOutcome {
        let program = match self.locator.locate(&self.program) {
            Some(program) => program,
            None => {
                return DbusCallOutcome::Unavailable {
                    detail: format!("未找到 {}，无法通过 D-Bus 触发部署", self.program),
                };
            }
        };
        let mut command = ProcessCommand::new(program.to_string_lossy().to_string()).args([
            // gdbus 的子命令必须紧跟在程序名之后：`gdbus call [OPTION...]`。
            "call",
            "--session",
            "--dest",
            destination,
            "--object-path",
            object_path,
            "--method",
            &format!("{interface}.{method}"),
            // gdbus 自带超时（秒）；外层 ProcessRunner 还会按精确 deadline 强制 kill。
            "--timeout",
            &timeout_secs(timeout).to_string(),
        ]);
        for arg in args {
            command = command.arg(arg.clone());
        }
        if let Some((key, value)) = session_bus_env_fallback() {
            command = command.env(key, value);
        }

        match self.runner.run(&command, timeout) {
            ProcessOutcome::Exited {
                code: Some(0),
                stdout,
                ..
            } => DbusCallOutcome::Ok {
                reply: stdout.trim().to_owned(),
            },
            ProcessOutcome::Exited { code, stderr, .. } => {
                let mut outcome = classify_gdbus_error(&stderr);
                if matches!(outcome, DbusCallOutcome::Error { .. }) {
                    if let DbusCallOutcome::Error { detail, .. } = &mut outcome {
                        if let Some(code) = code {
                            detail.push_str(&format!("（gdbus 退出码 {code}）"));
                        }
                    }
                }
                outcome
            }
            ProcessOutcome::TimedOut { stderr, .. } => DbusCallOutcome::TimedOut {
                detail: format!("gdbus 调用超时：{}", truncate(&stderr, 500)),
            },
            ProcessOutcome::NotFound { program } => DbusCallOutcome::Unavailable {
                detail: format!("未找到可执行文件 {program}"),
            },
            ProcessOutcome::Failed { message } => DbusCallOutcome::Unavailable { detail: message },
        }
    }
}

impl DbusClient for GdbusClient {
    fn name_has_owner(&self, service: &str, timeout: Duration) -> DbusCallOutcome {
        self.call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            &[service.to_owned()],
            timeout,
        )
    }

    fn set_config(&self, uri: &str, value: &str, timeout: Duration) -> DbusCallOutcome {
        // SetConfig 签名是 (s, v)，第二个参数是 variant<string>：gdbus 用 GVariant
        // 文本格式 `<'...'>` 表示 variant 包裹的字符串（空串即 `<''>`，已在本机实机
        // 用不存在的目标服务验证过该参数形式能被总线接受）。
        self.call(
            FCITX_SERVICE,
            CONTROLLER_PATH,
            CONTROLLER_INTERFACE,
            "SetConfig",
            &[uri.to_owned(), gvariant_variant_string(value)],
            timeout,
        )
    }

    fn reload_addon_config(&self, addon: &str, timeout: Duration) -> DbusCallOutcome {
        self.call(
            FCITX_SERVICE,
            CONTROLLER_PATH,
            CONTROLLER_INTERFACE,
            "ReloadAddonConfig",
            &[addon.to_owned()],
            timeout,
        )
    }

    fn get_current_schema(&self, timeout: Duration) -> DbusCallOutcome {
        self.call(
            RIME_SERVICE,
            RIME_PATH,
            RIME_INTERFACE,
            "GetCurrentSchema",
            &[],
            timeout,
        )
    }
}

/// 把字符串编码成 GVariant 文本格式里的 `variant<string>`（`<'...'>`）。
fn gvariant_variant_string(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() + 4);
    encoded.push_str("<'");
    for ch in value.chars() {
        match ch {
            '\\' => encoded.push_str("\\\\"),
            '\'' => encoded.push_str("\\'"),
            other => encoded.push(other),
        }
    }
    encoded.push_str("'>");
    encoded
}

/// 把 gdbus 的报错文本归类成可判定的 [`DbusCallOutcome`]。
///
/// gdbus 的报错形如
/// `Error: GDBus.Error:org.freedesktop.DBus.Error.ServiceUnknown: The name ... was not provided...`
/// 关键字（`GDBus.Error:` 后的错误名）与 locale 无关，日志里即使出现中文前缀也能解析。
fn classify_gdbus_error(stderr: &str) -> DbusCallOutcome {
    let detail = truncate(stderr.trim(), 800);
    let name = extract_dbus_error_name(stderr);
    let short = name
        .as_deref()
        .map(|name| name.rsplit('.').next().unwrap_or(name));
    match short {
        Some("ServiceUnknown") | Some("NameHasNoOwner") => {
            DbusCallOutcome::ServiceUnknown { detail }
        }
        Some("UnknownInterface")
        | Some("UnknownMethod")
        | Some("UnknownObject")
        | Some("InvalidArgs") => DbusCallOutcome::InterfaceUnknown { detail },
        Some("Timeout") | Some("NoReply") => DbusCallOutcome::TimedOut { detail },
        _ => {
            let lowered = stderr.to_ascii_lowercase();
            if lowered.contains("timed out") || lowered.contains("timeout") {
                DbusCallOutcome::TimedOut { detail }
            } else if lowered.contains("cannot autolaunch")
                || lowered.contains("dbus_session_bus_address")
                || lowered.contains("failed to connect to socket")
                || lowered.contains("connection refused")
            {
                DbusCallOutcome::Unavailable { detail }
            } else {
                DbusCallOutcome::Error { name, detail }
            }
        }
    }
}

/// 从 `GDBus.Error:<name>:` 中提取错误名。
fn extract_dbus_error_name(text: &str) -> Option<String> {
    const MARKER: &str = "GDBus.Error:";
    let start = text.find(MARKER)? + MARKER.len();
    let rest = &text[start..];
    let end = rest
        .find(|c: char| c == ':' || c.is_whitespace())
        .unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        Some(rest[..end].to_owned())
    }
}

/// 解析 gdbus 打印的单个字符串返回值，例如 `('xhup',)` → `xhup`。
pub(crate) fn parse_single_string(reply: &str) -> Option<String> {
    let inner = reply.trim().strip_prefix('(')?.strip_suffix(')')?.trim();
    let rest = inner.strip_prefix('\'')?;
    let mut value = String::new();
    let mut chars = rest.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => value.push(chars.next()?),
            '\'' => return Some(value),
            other => value.push(other),
        }
    }
    None
}

/// 解析 gdbus 打印的布尔返回值，例如 `(true,)` / `(false,)`。
pub(crate) fn parse_bool_reply(reply: &str) -> Option<bool> {
    let inner = reply.trim().strip_prefix('(')?.strip_suffix(')')?.trim();
    let first = inner.split(',').next()?.trim();
    match first {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn timeout_secs(timeout: Duration) -> u64 {
    let secs = timeout.as_secs();
    if timeout.subsec_nanos() > 0 {
        (secs + 1).max(1)
    } else {
        secs.max(1)
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push('…');
    out
}

/// 未显式设置 `DBUS_SESSION_BUS_ADDRESS` 时，用 logind 的约定路径兜底。
/// 典型场景：从桌面启动器/守护进程拉起，环境变量缺失但总线存在。
fn session_bus_env_fallback() -> Option<(String, String)> {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() {
        return None;
    }
    let uid = current_uid()?;
    let socket = PathBuf::from(format!("/run/user/{uid}/bus"));
    if socket.exists() {
        Some((
            "DBUS_SESSION_BUS_ADDRESS".to_owned(),
            format!("unix:path={}", socket.display()),
        ))
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn current_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self")
        .ok()
        .map(|metadata| metadata.uid())
}

#[cfg(not(target_os = "linux"))]
fn current_uid() -> Option<u32> {
    std::env::var("UID").ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{MockProcessRunner, StaticBinaryLocator};
    use crate::process::ProcessOutcome;

    fn client(runner: Arc<MockProcessRunner>) -> GdbusClient {
        GdbusClient::new(runner, Arc::new(StaticBinaryLocator::with(&["gdbus"])))
    }

    #[test]
    fn set_config_uses_exact_fcitx5_contract() {
        let runner = Arc::new(MockProcessRunner::new());
        runner.push_outcome(ProcessOutcome::Exited {
            code: Some(0),
            stdout: "()\n".to_owned(),
            stderr: String::new(),
        });
        let outcome =
            client(runner.clone()).set_config(DEPLOY_CONFIG_URI, "", Duration::from_secs(5));
        assert!(outcome.is_ok(), "got {outcome:?}");
        let command = runner.commands().pop().expect("应执行一次 gdbus");
        assert!(
            command.program.ends_with("gdbus"),
            "program: {}",
            command.program
        );
        let expected = [
            "call",
            "--session",
            "--dest",
            FCITX_SERVICE,
            "--object-path",
            CONTROLLER_PATH,
            "--method",
            "org.fcitx.Fcitx.Controller1.SetConfig",
            "--timeout",
            "5",
            DEPLOY_CONFIG_URI,
            "<''>",
        ];
        assert_eq!(command.args, expected);
    }

    #[test]
    fn reload_addon_config_uses_rime_addon() {
        let runner = Arc::new(MockProcessRunner::new());
        runner.push_outcome(ProcessOutcome::Exited {
            code: Some(0),
            stdout: "()\n".to_owned(),
            stderr: String::new(),
        });
        let outcome = client(runner.clone()).reload_addon_config("rime", Duration::from_secs(3));
        assert!(outcome.is_ok());
        let command = runner.commands().pop().expect("gdbus");
        assert_eq!(command.args.last().map(String::as_str), Some("rime"));
        assert!(command
            .args
            .contains(&"org.fcitx.Fcitx.Controller1.ReloadAddonConfig".to_owned()));
    }

    #[test]
    fn service_unknown_is_classified() {
        let runner = Arc::new(MockProcessRunner::new());
        runner.push_outcome(ProcessOutcome::Exited {
            code: Some(1),
            stdout: String::new(),
            stderr: "Error: GDBus.Error:org.freedesktop.DBus.Error.ServiceUnknown: The name org.fcitx.Fcitx5 was not provided by any .service files\n".to_owned(),
        });
        let outcome = client(runner).name_has_owner(FCITX_SERVICE, Duration::from_secs(2));
        assert!(
            matches!(outcome, DbusCallOutcome::ServiceUnknown { .. }),
            "got {outcome:?}"
        );
    }

    #[test]
    fn timeout_is_classified() {
        let runner = Arc::new(MockProcessRunner::new());
        runner.push_outcome(ProcessOutcome::Exited {
            code: Some(1),
            stdout: String::new(),
            stderr: "Error: GDBus.Error:org.freedesktop.DBus.Error.NoReply: Message recipient disconnected\n".to_owned(),
        });
        let outcome = client(runner).set_config(DEPLOY_CONFIG_URI, "", Duration::from_secs(2));
        assert!(
            matches!(outcome, DbusCallOutcome::TimedOut { .. }),
            "got {outcome:?}"
        );
    }

    #[test]
    fn missing_gdbus_is_unavailable() {
        let runner = Arc::new(MockProcessRunner::new());
        let client = GdbusClient::new(runner.clone(), Arc::new(StaticBinaryLocator::with(&[])));
        let outcome = client.get_current_schema(Duration::from_secs(2));
        assert!(
            matches!(outcome, DbusCallOutcome::Unavailable { .. }),
            "got {outcome:?}"
        );
        assert!(runner.commands().is_empty(), "缺失 gdbus 时不应尝试执行");
    }

    #[test]
    fn process_timeout_maps_to_timed_out() {
        let runner = Arc::new(MockProcessRunner::new());
        runner.push_outcome(ProcessOutcome::TimedOut {
            stdout: String::new(),
            stderr: String::new(),
        });
        let outcome = client(runner).get_current_schema(Duration::from_secs(2));
        assert!(matches!(outcome, DbusCallOutcome::TimedOut { .. }));
    }

    #[test]
    fn parses_single_string_reply() {
        assert_eq!(parse_single_string("('xhup',)"), Some("xhup".to_owned()));
        assert_eq!(
            parse_single_string("('double_pinyin_flypy',)"),
            Some("double_pinyin_flypy".to_owned())
        );
        assert_eq!(parse_single_string("('a\\'b',)"), Some("a'b".to_owned()));
        assert_eq!(parse_single_string("()"), None);
        assert_eq!(parse_single_string("(true,)"), None);
    }

    #[test]
    fn parses_bool_reply() {
        assert_eq!(parse_bool_reply("(true,)"), Some(true));
        assert_eq!(parse_bool_reply("(false,)"), Some(false));
        assert_eq!(parse_bool_reply("('xhup',)"), None);
    }

    #[test]
    fn truncates_long_detail() {
        let long = "x".repeat(2000);
        let truncated = truncate(&long, 10);
        assert_eq!(truncated.chars().count(), 11);
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn encodes_variant_string_for_set_config() {
        assert_eq!(gvariant_variant_string(""), "<''>");
        assert_eq!(gvariant_variant_string("abc"), "<'abc'>");
        assert_eq!(gvariant_variant_string("a'b"), "<'a\\'b'>");
        assert_eq!(gvariant_variant_string("a\\b"), "<'a\\\\b'>");
    }

    #[test]
    fn timeout_seconds_rounds_up() {
        assert_eq!(timeout_secs(Duration::from_millis(1)), 1);
        assert_eq!(timeout_secs(Duration::from_secs(5)), 5);
        assert_eq!(timeout_secs(Duration::from_millis(5500)), 6);
    }
}
