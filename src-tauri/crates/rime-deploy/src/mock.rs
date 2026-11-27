//! 测试/诊断用替身。
//!
//! 提供给本 crate 的单元测试与上层（Tauri 层）做集成测试：
//! **所有替身都不会访问真实会话总线、不会启动部署进程、不会写用户 Rime 目录**
//! （[`MockDeployer`] 只写调用方显式传入的 `build_dir`，测试里是 tempdir）。

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use rime_core::types::{AppError, AppResult, DeployTrigger};

use crate::dbus::{DbusCallOutcome, DbusClient};
use crate::deployer::{DeployContext, DeployOutcome, Deployer};
use crate::process::{BinaryLocator, ProcessCommand, ProcessOutcome, ProcessRunner};

// ───────────────────────── 同步闸门 ─────────────────────────

/// 一次性的「放行闸门」：用于在并发合并测试里把第一轮部署卡在 Running 阶段。
#[derive(Debug, Default)]
pub struct Gate {
    open: Mutex<bool>,
    cv: Condvar,
}

impl Gate {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 放行所有等待者（打开后永久放行）。
    pub fn open(&self) {
        let mut open = self
            .open
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *open = true;
        self.cv.notify_all();
    }

    pub fn is_open(&self) -> bool {
        *self
            .open
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn wait(&self) {
        let mut open = self
            .open
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while !*open {
            open = self
                .cv
                .wait(open)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

// ───────────────────────── MockDeployer ─────────────────────────

/// [`Deployer`] 的可编程替身。
#[derive(Debug, Clone, Default)]
pub struct MockBehavior {
    /// 返回该错误（模拟部署失败）。
    pub error: Option<AppError>,
    /// 部署耗时（在 `build_dir` 写入前 sleep）。
    pub delay: Option<Duration>,
    /// 是否往 `build_dir` 写一个内容随调用次数变化的产物文件（模拟 build/ 变化）。
    pub mutate_build: bool,
}

/// 记录每次调用的部署替身。
pub struct MockDeployer {
    inner: Mutex<MockInner>,
    cv: Condvar,
    gate: Option<Arc<Gate>>,
}

#[derive(Debug, Default)]
struct MockInner {
    triggers: Vec<DeployTrigger>,
    finished: usize,
    behavior: MockBehavior,
}

impl MockDeployer {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MockInner::default()),
            cv: Condvar::new(),
            gate: None,
        }
    }

    /// 部署时先卡在 [`Gate`] 上（并发合并测试用）。
    pub fn with_gate(mut self, gate: Arc<Gate>) -> Self {
        self.gate = Some(gate);
        self
    }

    pub fn with_behavior(self, behavior: MockBehavior) -> Self {
        self.set_behavior(behavior);
        self
    }

    pub fn set_behavior(&self, behavior: MockBehavior) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .behavior = behavior;
    }

    /// 每次调用都会往 build/ 写产物（保证产物验证通过）。
    pub fn mutating() -> Self {
        Self::new().with_behavior(MockBehavior {
            mutate_build: true,
            ..MockBehavior::default()
        })
    }

    /// 调用次数。
    pub fn calls(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .triggers
            .len()
    }

    pub fn triggers(&self) -> Vec<DeployTrigger> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .triggers
            .clone()
    }

    pub fn started_count(&self) -> usize {
        self.calls()
    }

    pub fn finished_count(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .finished
    }

    /// 等待至少 `count` 次调用开始。
    pub fn wait_started(&self, count: usize, timeout: Duration) -> bool {
        self.wait_for(|inner| inner.triggers.len() >= count, timeout)
    }

    /// 等待至少 `count` 次调用结束。
    pub fn wait_finished(&self, count: usize, timeout: Duration) -> bool {
        self.wait_for(|inner| inner.finished >= count, timeout)
    }

    fn wait_for(&self, predicate: impl Fn(&MockInner) -> bool, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if predicate(&inner) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let (guard, _) = self
                .cv
                .wait_timeout(inner, deadline.saturating_duration_since(now))
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            inner = guard;
        }
    }
}

impl Default for MockDeployer {
    fn default() -> Self {
        Self::new()
    }
}

impl Deployer for MockDeployer {
    fn method_name(&self) -> &'static str {
        "mock"
    }

    fn deploy(&self, ctx: &DeployContext) -> AppResult<DeployOutcome> {
        let call_index = {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            inner.triggers.push(ctx.trigger);
            self.cv.notify_all();
            inner.triggers.len()
        };

        if let Some(gate) = &self.gate {
            gate.wait();
        }

        let behavior = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .behavior
            .clone();
        if let Some(delay) = behavior.delay {
            std::thread::sleep(delay);
        }
        if behavior.mutate_build {
            std::fs::create_dir_all(&ctx.build_dir).map_err(|error| {
                AppError::new(
                    rime_core::types::ErrorCode::DeployFailed,
                    format!("mock 写 build/ 失败：{error}"),
                )
            })?;
            // 内容带调用序号：即使同一毫秒内重写，size 也会变化，产物判定必然成立。
            let marker = ctx.build_dir.join("mock-build.out");
            std::fs::write(&marker, format!("mock-call={call_index}\n")).map_err(|error| {
                AppError::new(
                    rime_core::types::ErrorCode::DeployFailed,
                    format!("mock 写产物失败：{error}"),
                )
            })?;
        }

        {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            inner.finished += 1;
            self.cv.notify_all();
        }

        match behavior.error {
            Some(error) => Err(error),
            None => Ok(DeployOutcome::new("mock")),
        }
    }
}

// ───────────────────────── MockDbusClient ─────────────────────────

/// 记录下来的 D-Bus 调用（测试断言用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbusCall {
    NameHasOwner(String),
    SetConfig { uri: String, value: String },
    ReloadAddonConfig(String),
    GetCurrentSchema,
}

/// [`DbusClient`] 的可编程替身。
pub struct MockDbusClient {
    inner: Mutex<MockDbusInner>,
}

struct MockDbusInner {
    name_has_owner: bool,
    set_config: DbusCallOutcome,
    reload: DbusCallOutcome,
    current_schema: DbusCallOutcome,
    calls: Vec<DbusCall>,
}

impl Default for MockDbusInner {
    fn default() -> Self {
        Self {
            name_has_owner: true,
            set_config: DbusCallOutcome::Ok {
                reply: "()".to_owned(),
            },
            reload: DbusCallOutcome::Ok {
                reply: "()".to_owned(),
            },
            current_schema: DbusCallOutcome::Ok {
                reply: "('xhup',)".to_owned(),
            },
            calls: Vec::new(),
        }
    }
}

impl MockDbusClient {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MockDbusInner::default()),
        }
    }

    pub fn new_with_owner(owner: bool) -> Self {
        let client = Self::new();
        client.set_name_has_owner(owner);
        client
    }

    pub fn set_name_has_owner(&self, owner: bool) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .name_has_owner = owner;
    }

    pub fn set_set_config_outcome(&self, outcome: DbusCallOutcome) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_config = outcome;
    }

    pub fn set_reload_outcome(&self, outcome: DbusCallOutcome) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .reload = outcome;
    }

    pub fn set_current_schema_outcome(&self, outcome: DbusCallOutcome) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .current_schema = outcome;
    }

    pub fn calls(&self) -> Vec<DbusCall> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .calls
            .clone()
    }
}

impl Default for MockDbusClient {
    fn default() -> Self {
        Self::new()
    }
}

impl DbusClient for MockDbusClient {
    fn name_has_owner(&self, service: &str, _timeout: Duration) -> DbusCallOutcome {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.calls.push(DbusCall::NameHasOwner(service.to_owned()));
        let owner = inner.name_has_owner;
        DbusCallOutcome::Ok {
            reply: format!("({owner},)"),
        }
    }

    fn set_config(&self, uri: &str, value: &str, _timeout: Duration) -> DbusCallOutcome {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.calls.push(DbusCall::SetConfig {
            uri: uri.to_owned(),
            value: value.to_owned(),
        });
        inner.set_config.clone()
    }

    fn reload_addon_config(&self, addon: &str, _timeout: Duration) -> DbusCallOutcome {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner
            .calls
            .push(DbusCall::ReloadAddonConfig(addon.to_owned()));
        inner.reload.clone()
    }

    fn get_current_schema(&self, _timeout: Duration) -> DbusCallOutcome {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.calls.push(DbusCall::GetCurrentSchema);
        inner.current_schema.clone()
    }
}

// ───────────────────────── MockProcessRunner ─────────────────────────

/// [`ProcessRunner`] 的可编程替身：记录命令、按队列返回结果。
pub struct MockProcessRunner {
    inner: Mutex<MockRunnerInner>,
}

struct MockRunnerInner {
    commands: Vec<ProcessCommand>,
    outcomes: VecDeque<ProcessOutcome>,
    default_outcome: ProcessOutcome,
}

impl MockProcessRunner {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MockRunnerInner {
                commands: Vec::new(),
                outcomes: VecDeque::new(),
                default_outcome: ProcessOutcome::Exited {
                    code: Some(0),
                    stdout: String::new(),
                    stderr: String::new(),
                },
            }),
        }
    }

    /// 依次返回一个预设结果（先入先出）。
    pub fn push_outcome(&self, outcome: ProcessOutcome) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .outcomes
            .push_back(outcome);
    }

    /// 队列空时的默认结果。
    pub fn set_default_outcome(&self, outcome: ProcessOutcome) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .default_outcome = outcome;
    }

    pub fn commands(&self) -> Vec<ProcessCommand> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .commands
            .clone()
    }
}

impl Default for MockProcessRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessRunner for MockProcessRunner {
    fn run(&self, command: &ProcessCommand, _timeout: Duration) -> ProcessOutcome {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.commands.push(command.clone());
        inner
            .outcomes
            .pop_front()
            .unwrap_or_else(|| inner.default_outcome.clone())
    }
}

// ───────────────────────── StaticBinaryLocator ─────────────────────────

/// 固定结果的 [`BinaryLocator`]（测试不受宿主 PATH 影响）。
#[derive(Debug, Default)]
pub struct StaticBinaryLocator {
    available: Mutex<Vec<String>>,
}

impl StaticBinaryLocator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(names: &[&str]) -> Self {
        let locator = Self::new();
        {
            let mut available = locator
                .available
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            available.extend(names.iter().map(|name| (*name).to_owned()));
        }
        locator
    }

    pub fn add(&self, name: impl Into<String>) {
        self.available
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(name.into());
    }
}

impl BinaryLocator for StaticBinaryLocator {
    fn locate(&self, name: &str) -> Option<PathBuf> {
        let available = self
            .available
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        available
            .iter()
            .any(|item| item == name)
            .then(|| PathBuf::from(format!("/usr/bin/{name}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deployer::DeployContext;
    use std::time::Instant;

    fn context(build_dir: PathBuf) -> DeployContext {
        DeployContext {
            trigger: DeployTrigger::AfterSave,
            user_dir: build_dir.parent().unwrap_or(&build_dir).to_path_buf(),
            shared_dir: PathBuf::from("/usr/share/rime-data"),
            build_dir,
            deadline: Instant::now() + Duration::from_secs(5),
        }
    }

    #[test]
    fn mock_deployer_records_calls_and_writes_build_artifacts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let build = dir.path().join("build");
        let mock = MockDeployer::mutating();
        mock.deploy(&context(build.clone())).expect("mock deploy");
        assert_eq!(mock.calls(), 1);
        assert_eq!(mock.triggers(), vec![DeployTrigger::AfterSave]);
        assert!(build.join("mock-build.out").is_file());
    }

    #[test]
    fn gate_blocks_until_opened() {
        let dir = tempfile::tempdir().expect("tempdir");
        let gate = Gate::new();
        let mock = Arc::new(MockDeployer::mutating().with_gate(Arc::clone(&gate)));
        let worker = {
            let mock = Arc::clone(&mock);
            let ctx = context(dir.path().join("build"));
            std::thread::spawn(move || mock.deploy(&ctx))
        };
        assert!(mock.wait_started(1, Duration::from_secs(2)));
        assert!(!gate.is_open());
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(mock.finished_count(), 0, "闸门未开时不应完成");
        gate.open();
        worker.join().expect("join").expect("deploy");
        assert_eq!(mock.finished_count(), 1);
    }

    #[test]
    fn static_locator_matches_exact_names() {
        let locator = StaticBinaryLocator::with(&["gdbus"]);
        assert!(locator.locate("gdbus").is_some());
        assert!(locator.locate("rime_deployer").is_none());
    }

    #[test]
    fn mock_process_runner_queues_outcomes() {
        let runner = MockProcessRunner::new();
        runner.push_outcome(ProcessOutcome::TimedOut {
            stdout: String::new(),
            stderr: String::new(),
        });
        let command = ProcessCommand::new("gdbus");
        let first = runner.run(&command, Duration::from_secs(1));
        let second = runner.run(&command, Duration::from_secs(1));
        assert!(matches!(first, ProcessOutcome::TimedOut { .. }));
        assert!(second.is_success());
        assert_eq!(runner.commands().len(), 2);
    }
}
