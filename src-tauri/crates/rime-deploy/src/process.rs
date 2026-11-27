//! 子进程执行抽象。
//!
//! 部署链路里有两类外部程序：`gdbus`（D-Bus 方法调用）与 `rime_deployer`（兜底构建）。
//! 二者都通过 [`ProcessRunner`] 抽象，测试注入 [`crate::mock::MockProcessRunner`]，
//! 保证单测**不会真正启动部署进程、不会写用户 Rime 目录**。
//!
//! 超时在这里硬保证：到点 `kill` 子进程并返回 [`ProcessOutcome::TimedOut`]，
//! 不依赖子进程自觉退出。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

/// 单次读取子进程输出的上限（防止异常程序灌爆内存）。超出部分丢弃。
const MAX_CAPTURED_OUTPUT: usize = 256 * 1024;

/// 一条待执行的子进程命令。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessCommand {
    pub program: String,
    pub args: Vec<String>,
    /// 追加/覆盖的环境变量（实现会额外强制 `LC_ALL=C`，便于稳定解析错误信息）。
    pub env: Vec<(String, String)>,
}

impl ProcessCommand {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
}

/// 子进程执行结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessOutcome {
    /// 正常退出（`code == Some(0)` 才算成功）。
    Exited {
        code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    /// 到达超时被强制杀掉（保留已捕获的输出，便于日志）。
    TimedOut { stdout: String, stderr: String },
    /// 可执行文件不存在（用于降级到下一部署方式）。
    NotFound { program: String },
    /// spawn / wait 等系统调用失败。
    Failed { message: String },
}

impl ProcessOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, ProcessOutcome::Exited { code: Some(0), .. })
    }

    pub fn stdout(&self) -> &str {
        match self {
            ProcessOutcome::Exited { stdout, .. } | ProcessOutcome::TimedOut { stdout, .. } => {
                stdout
            }
            ProcessOutcome::NotFound { .. } | ProcessOutcome::Failed { .. } => "",
        }
    }

    pub fn stderr(&self) -> &str {
        match self {
            ProcessOutcome::Exited { stderr, .. } | ProcessOutcome::TimedOut { stderr, .. } => {
                stderr
            }
            ProcessOutcome::NotFound { .. } | ProcessOutcome::Failed { .. } => "",
        }
    }
}

/// 子进程执行器。实现必须保证在 `timeout` 内返回（到点杀进程）。
pub trait ProcessRunner: Send + Sync {
    fn run(&self, command: &ProcessCommand, timeout: Duration) -> ProcessOutcome;
}

/// 真实执行器。
#[derive(Debug, Default)]
pub struct SystemProcessRunner;

impl SystemProcessRunner {
    pub fn new() -> Self {
        Self
    }
}

impl ProcessRunner for SystemProcessRunner {
    fn run(&self, command: &ProcessCommand, timeout: Duration) -> ProcessOutcome {
        let mut cmd = Command::new(&command.program);
        cmd.args(&command.args)
            // 固定英文输出，`GDBus.Error:` 等关键字解析不随系统 locale 漂移。
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &command.env {
            cmd.env(key, value);
        }

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return ProcessOutcome::NotFound {
                    program: command.program.clone(),
                };
            }
            Err(err) => {
                return ProcessOutcome::Failed {
                    message: format!("启动 {} 失败：{err}", command.program),
                };
            }
        };

        let stdout_reader = child.stdout.take().map(spawn_reader);
        let stderr_reader = child.stderr.take().map(spawn_reader);

        let deadline = Instant::now() + timeout;
        let mut timed_out = false;
        let exit_code: Option<i32> = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.code(),
                Ok(None) => {
                    let now = Instant::now();
                    if now >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        timed_out = true;
                        break None;
                    }
                    thread::sleep(
                        deadline
                            .saturating_duration_since(now)
                            .min(Duration::from_millis(10)),
                    );
                }
                Err(err) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return ProcessOutcome::Failed {
                        message: format!("等待子进程失败：{err}"),
                    };
                }
            }
        };

        // 注意：超时只 kill 直接子进程。若它还有孙进程继承了管道，读线程可能暂时阻塞；
        // 因此这里的收集有上限，宁可丢弃尾部输出也不能卡住调用方（部署超时必须准时返回）。
        let collect_window = Duration::from_millis(250);
        let stdout = stdout_reader
            .map(|rx| collect_reader(rx, collect_window))
            .unwrap_or_default();
        let stderr = stderr_reader
            .map(|rx| collect_reader(rx, collect_window))
            .unwrap_or_default();
        if timed_out {
            ProcessOutcome::TimedOut { stdout, stderr }
        } else {
            ProcessOutcome::Exited {
                code: exit_code,
                stdout,
                stderr,
            }
        }
    }
}

fn spawn_reader<R: Read + Send + 'static>(reader: R) -> Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(read_capped(reader));
    });
    receiver
}

fn read_capped<R: Read>(mut reader: R) -> String {
    let mut captured: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if captured.len() < MAX_CAPTURED_OUTPUT {
                    let room = MAX_CAPTURED_OUTPUT - captured.len();
                    captured.extend_from_slice(&chunk[..n.min(room)]);
                }
                // 超出上限后继续读空，避免管道写满导致子进程阻塞。
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&captured).into_owned()
}

fn collect_reader(receiver: Receiver<String>, wait: Duration) -> String {
    receiver.recv_timeout(wait).unwrap_or_default()
}

/// 在 PATH（及额外目录）中查找可执行文件。用于探测 `gdbus` / `rime_deployer`。
pub trait BinaryLocator: Send + Sync {
    fn locate(&self, name: &str) -> Option<PathBuf>;
}

/// 基于 PATH 与固定系统目录的查找器。
#[derive(Debug, Clone)]
pub struct PathBinaryLocator {
    extra_dirs: Vec<PathBuf>,
    search_path: bool,
}

impl Default for PathBinaryLocator {
    fn default() -> Self {
        Self {
            extra_dirs: vec![
                PathBuf::from("/usr/bin"),
                PathBuf::from("/usr/local/bin"),
                PathBuf::from("/bin"),
            ],
            search_path: true,
        }
    }
}

impl PathBinaryLocator {
    /// 只在这些目录里找（测试用，结果与宿主 PATH 无关）。
    pub fn with_extra_dirs<I, P>(dirs: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        Self {
            extra_dirs: dirs.into_iter().map(Into::into).collect(),
            search_path: false,
        }
    }

    pub fn locate_in_path(name: &str) -> Option<PathBuf> {
        let locator = PathBinaryLocator::default();
        <Self as BinaryLocator>::locate(&locator, name)
    }
}

impl BinaryLocator for PathBinaryLocator {
    fn locate(&self, name: &str) -> Option<PathBuf> {
        if name.contains(std::path::MAIN_SEPARATOR) {
            let path = PathBuf::from(name);
            return is_executable(&path).then_some(path);
        }
        let mut dirs = self.extra_dirs.clone();
        if self.search_path {
            if let Some(path_var) = std::env::var_os("PATH") {
                dirs.extend(std::env::split_paths(&path_var));
            }
        }
        for dir in dirs {
            let candidate = dir.join(name);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
        None
    }
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_stdout_and_exit_code() {
        let runner = SystemProcessRunner::new();
        let outcome = runner.run(
            &ProcessCommand::new("/bin/sh").args(["-c", "printf hello; printf oops >&2"]),
            Duration::from_secs(5),
        );
        match outcome {
            ProcessOutcome::Exited {
                code,
                stdout,
                stderr,
            } => {
                assert_eq!(code, Some(0));
                assert_eq!(stdout, "hello");
                assert_eq!(stderr, "oops");
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn nonzero_exit_is_not_success() {
        let runner = SystemProcessRunner::new();
        let outcome = runner.run(
            &ProcessCommand::new("/bin/sh").args(["-c", "exit 3"]),
            Duration::from_secs(5),
        );
        assert!(!outcome.is_success());
        assert!(matches!(
            outcome,
            ProcessOutcome::Exited { code: Some(3), .. }
        ));
    }

    #[test]
    fn kills_process_on_timeout() {
        let runner = SystemProcessRunner::new();
        let started = Instant::now();
        let outcome = runner.run(
            &ProcessCommand::new("/bin/sh").args(["-c", "sleep 5"]),
            Duration::from_millis(60),
        );
        assert!(
            matches!(outcome, ProcessOutcome::TimedOut { .. }),
            "got {outcome:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "超时未被强制打断"
        );
    }

    #[test]
    fn missing_program_reports_not_found() {
        let runner = SystemProcessRunner::new();
        let outcome = runner.run(
            &ProcessCommand::new("/nonexistent/rime-deploy-probe"),
            Duration::from_secs(1),
        );
        assert!(
            matches!(outcome, ProcessOutcome::NotFound { .. }),
            "got {outcome:?}"
        );
    }

    #[test]
    fn locator_finds_executable_in_extra_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fake-deployer");
        std::fs::write(&path, "#!/bin/sh\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        let locator = PathBinaryLocator::with_extra_dirs([dir.path().to_path_buf()]);
        assert_eq!(locator.locate("fake-deployer"), Some(path));
        assert_eq!(locator.locate("no-such-deployer"), None);
    }

    #[test]
    fn locator_ignores_non_executable_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("not-executable");
        std::fs::write(&path, "plain text").expect("write");
        let locator = PathBinaryLocator::with_extra_dirs([dir.path().to_path_buf()]);
        #[cfg(unix)]
        assert_eq!(locator.locate("not-executable"), None);
    }
}
