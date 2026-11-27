//! 部署状态机与并发请求合并（PRD FR-5.5 / FR-5.6）。
//!
//! ## 状态机
//!
//! ```text
//! Idle ──request──▶ Preparing ──▶ Running ──▶ Verifying ──▶ Success
//!                                        └──────────────▶ Failed
//! ```
//!
//! * `Preparing`：记录起止时间、拍 `build/` 基线快照；
//! * `Running`：调用 [`Deployer`] 触发部署（D-Bus / 兜底进程）；
//! * `Verifying`：轮询 `build/`，只有产物（mtime + size）真的变化才 `output_verified=true`；
//! * 每次状态变化通过 [`DeployObserver`] 回调（Tauri 层据此 `emit` 事件）。
//!
//! ## 并发合并
//!
//! 同一时刻只跑一轮部署。`Running` 期间的新请求只覆盖一个 `pending` 标志
//! （按 手动 > 保存后 > 定时 合并），当前轮结束后最多补跑**一次**，
//! 绝不排队 N 次（FR-5.6）。
//!
//! ## 线程模型
//!
//! [`DeployController::request`] 立即返回并在后台线程执行，不阻塞 UI；
//! [`DeployController::deploy_blocking`] 供 CLI/测试等待结果。
//! 等待/通知使用 `Mutex + Condvar`，不引入异步运行时。

use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rime_core::types::{
    AppError, AppResult, DeployPhase, DeployResult, DeployState, DeployTrigger, ErrorCode,
};
use tracing::{debug, error, info, warn};

use crate::deployer::{DeployContext, DeployOutcome, Deployer};
use crate::error;
use crate::snapshot::{BuildDelta, BuildSnapshot};

/// 保留的最近部署结果条数（部署状态中心用）。
const MAX_RECENT_RESULTS: usize = 20;

/// 部署配置。
#[derive(Debug, Clone)]
pub struct DeployConfig {
    /// Rime 用户目录（fcitx5 通常为 `~/.local/share/fcitx5/rime`）。
    pub user_dir: PathBuf,
    /// 共享数据目录，兜底部署时传给 rime_deployer。
    pub shared_dir: PathBuf,
    /// 构建产物目录，默认 `user_dir/build`。
    pub build_dir: PathBuf,
    /// 单轮部署总超时（覆盖触发 + 产物校验）。
    pub timeout: Duration,
    /// 校验阶段轮询 `build/` 的间隔。
    pub poll_interval: Duration,
    /// 首次检测到产物变化后，需保持静止多久才认定部署完成。
    pub settle_window: Duration,
    /// 部署日志目录；`None` 表示不落盘（`DeployResult.log_path` 为 `None`）。
    pub log_dir: Option<PathBuf>,
    /// 「产物无变化」的提前收敛窗口。
    ///
    /// 真正的重新编译会在几秒内写 `build/`；超过这个窗口仍毫无变化，
    /// 基本可判定为「librime 认为无需重编译」，无需等满 `timeout` 再给结论。
    pub no_change_grace: Duration,
}

impl DeployConfig {
    pub fn new(user_dir: impl Into<PathBuf>) -> Self {
        let user_dir = user_dir.into();
        Self {
            build_dir: user_dir.join("build"),
            user_dir,
            shared_dir: PathBuf::from("/usr/share/rime-data"),
            timeout: Duration::from_secs(60),
            poll_interval: Duration::from_millis(200),
            settle_window: Duration::from_millis(600),
            log_dir: None,
            // 实测 fcitx5-rime 部署大词库时 build/ 首批写入在 1–3 秒内完成；
            // 8 秒足以区分「在编译」与「无需编译」，又不会让用户久等。
            no_change_grace: Duration::from_secs(8),
        }
    }

    pub fn with_build_dir(mut self, build_dir: impl Into<PathBuf>) -> Self {
        self.build_dir = build_dir.into();
        self
    }

    pub fn with_shared_dir(mut self, shared_dir: impl Into<PathBuf>) -> Self {
        self.shared_dir = shared_dir.into();
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = poll_interval;
        self
    }

    pub fn with_settle_window(mut self, settle_window: Duration) -> Self {
        self.settle_window = settle_window;
        self
    }

    pub fn with_log_dir(mut self, log_dir: impl Into<PathBuf>) -> Self {
        self.log_dir = Some(log_dir.into());
        self
    }

    pub fn with_no_change_grace(mut self, grace: Duration) -> Self {
        self.no_change_grace = grace;
        self
    }
}

/// 一次部署请求的处置结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestOutcome {
    /// 当前空闲，已启动新一轮部署。
    Started,
    /// 已有部署在跑，本次请求合并进 pending（本轮最多补跑一次）。
    Merged,
}

/// 状态变化观察者（Tauri 层在此 `emit` 事件）。回调在部署线程中同步调用，
/// 实现应只做转发，避免阻塞。
pub trait DeployObserver: Send + Sync {
    fn on_state(&self, state: &DeployState);
}

impl<F> DeployObserver for F
where
    F: Fn(&DeployState) + Send + Sync + 'static,
{
    fn on_state(&self, state: &DeployState) {
        (self)(state);
    }
}

struct Inner {
    state: DeployState,
    running: bool,
    pending: Option<DeployTrigger>,
    completed_rounds: u64,
    results: VecDeque<(u64, DeployResult)>,
}

/// 部署编排器：状态机 + 并发合并 + 产物校验。
///
/// 以 [`Arc`] 形式持有（后台线程需要共享所有权）。
pub struct DeployController {
    deployer: Arc<dyn Deployer>,
    config: DeployConfig,
    observer: Mutex<Option<Arc<dyn DeployObserver>>>,
    inner: Mutex<Inner>,
    cv: Condvar,
}

impl DeployController {
    pub fn new(deployer: Arc<dyn Deployer>, config: DeployConfig) -> Arc<Self> {
        Arc::new(Self {
            deployer,
            config,
            observer: Mutex::new(None),
            inner: Mutex::new(Inner {
                state: DeployState::default(),
                running: false,
                pending: None,
                completed_rounds: 0,
                results: VecDeque::new(),
            }),
            cv: Condvar::new(),
        })
    }

    pub fn config(&self) -> &DeployConfig {
        &self.config
    }

    /// 设置/清除状态观察者。
    pub fn set_observer(&self, observer: Option<Arc<dyn DeployObserver>>) {
        *self
            .observer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = observer;
    }

    pub fn state(&self) -> DeployState {
        self.lock_inner().state.clone()
    }

    pub fn is_running(&self) -> bool {
        self.lock_inner().running
    }

    /// 最近 N 次部署结果（最新在前）。
    pub fn recent_results(&self, limit: usize) -> Vec<DeployResult> {
        let inner = self.lock_inner();
        inner
            .results
            .iter()
            .rev()
            .take(limit)
            .map(|(_, result)| result.clone())
            .collect()
    }

    /// 请求部署（非阻塞）。并发请求自动合并为至多一次补跑。
    pub fn request(self: &Arc<Self>, trigger: DeployTrigger) -> AppResult<RequestOutcome> {
        let mut inner = self.lock_inner();
        if inner.running {
            inner.pending = Some(merge_triggers(inner.pending, trigger));
            debug!(?trigger, "部署进行中，请求已合并为一次补跑");
            return Ok(RequestOutcome::Merged);
        }
        inner.running = true;
        inner.pending = Some(trigger);

        let controller = Arc::clone(self);
        drop(inner);
        let spawned = thread::Builder::new()
            .name("rime-deploy".to_owned())
            .spawn(move || controller.worker_loop());
        match spawned {
            Ok(_) => Ok(RequestOutcome::Started),
            Err(err) => {
                let mut inner = self.lock_inner();
                inner.running = false;
                inner.pending = None;
                Err(
                    AppError::new(ErrorCode::Internal, format!("无法启动部署线程：{err}"))
                        .with_hint("请重启应用后重试"),
                )
            }
        }
    }

    /// 请求部署并等待至少一轮结束（CLI/测试用；不要从 UI 线程调用）。
    ///
    /// 若已有部署在跑，本次请求按合并规则并入 pending，并返回当前在跑那一轮的结果
    /// （不会重复排队）；若空闲则返回本轮结果。
    pub fn deploy_blocking(self: &Arc<Self>, trigger: DeployTrigger) -> AppResult<DeployResult> {
        let target_round = self.lock_inner().completed_rounds + 1;
        self.request(trigger)?;
        self.wait_for_round(target_round, self.config.timeout + Duration::from_secs(5))
            .ok_or_else(|| {
                AppError::new(ErrorCode::Internal, "等待部署结果超时")
                    .with_hint("部署可能仍在后台执行，请稍后查看部署状态")
            })
    }

    /// 等待部署线程空闲。
    pub fn wait_until_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut inner = self.lock_inner();
        while inner.running {
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
        true
    }

    fn worker_loop(self: Arc<Self>) {
        loop {
            let trigger = {
                let mut inner = self.lock_inner();
                match inner.pending.take() {
                    Some(trigger) => trigger,
                    None => {
                        inner.running = false;
                        self.cv.notify_all();
                        return;
                    }
                }
            };
            let before = self.lock_inner().completed_rounds;
            let panicked =
                std::panic::catch_unwind(AssertUnwindSafe(|| self.run_round(trigger))).is_err();
            if panicked && self.lock_inner().completed_rounds == before {
                // 兜底：任何未预料的 panic 都不能让控制器永久卡在 running 状态。
                let err = AppError::new(ErrorCode::Internal, "部署任务发生内部错误")
                    .with_hint("请重试；若持续失败请查看应用日志");
                self.finish_failure(trigger, Instant::now(), err, false, None);
            }
        }
    }

    fn run_round(&self, trigger: DeployTrigger) {
        let started_at = now_ms();
        let round_start = Instant::now();
        let deadline = round_start + self.config.timeout;
        self.begin_round(started_at);

        if !self.config.user_dir.is_dir() {
            let err = error::invalid_user_dir(&self.config.user_dir);
            self.finish_failure(trigger, round_start, err, false, None);
            return;
        }

        // 部署前产物基线（PRD FR-5.5：不以退出码判定成功）。
        let before = BuildSnapshot::capture(&self.config.build_dir);
        debug!(
            build_dir = %self.config.build_dir.display(),
            files = before.len(),
            "部署前 build/ 快照完成"
        );

        self.set_phase(
            DeployPhase::Running,
            Some(format!("正在触发部署（{}）…", trigger_label(trigger))),
        );
        let context = DeployContext {
            trigger,
            user_dir: self.config.user_dir.clone(),
            shared_dir: self.config.shared_dir.clone(),
            build_dir: self.config.build_dir.clone(),
            deadline,
        };

        let method_result =
            match std::panic::catch_unwind(AssertUnwindSafe(|| self.deployer.deploy(&context))) {
                Ok(result) => result,
                Err(_) => {
                    let err = AppError::new(
                        ErrorCode::Internal,
                        format!("部署方式 {} 发生内部错误", self.deployer.method_name()),
                    )
                    .with_hint("请反馈日志以便排查；可先手动执行 rime_deployer 兜底部署");
                    self.finish_failure(trigger, round_start, err, false, None);
                    return;
                }
            };

        let outcome = match method_result {
            Err(err) => {
                self.finish_failure(trigger, round_start, err, false, None);
                return;
            }
            Ok(outcome) => outcome,
        };

        if Instant::now() >= deadline {
            let err = error::timed_out(format!(
                "部署超时：触发部署后 {} 秒内未完成",
                self.config.timeout.as_secs_f32()
            ));
            self.finish_failure(trigger, round_start, err, false, None);
            return;
        }

        self.set_phase(
            DeployPhase::Verifying,
            Some("正在校验 build/ 产物…".to_owned()),
        );
        match self.wait_for_output(&before, deadline) {
            VerifyOutcome::Verified(delta) => {
                self.finish_success(trigger, round_start, outcome, true, &delta, None);
            }
            VerifyOutcome::Unsettled(delta) => {
                let note = format!(
                    "已检测到 build/ 产物更新（{}），部署可能仍在收尾",
                    delta.summary()
                );
                self.finish_success(trigger, round_start, outcome, true, &delta, Some(note));
            }
            VerifyOutcome::NoChange => {
                // 「产物没变」≠「部署失败」：librime 的部署是增量的，
                // 词库没改动时会直接跳过重新编译，此时 build/ 完全不会发生变化
                // （实测：点部署能看到 Rime 自己的「部署完成」提示，但 build/ 一个字节没动）。
                // 因此这里要用「构建产物是否已比词库源文件更新」来判断是否生效。
                match build_is_up_to_date(&self.config.user_dir, &self.config.build_dir) {
                    Ok(true) => {
                        let note = format!(
                            "部署完成：词库内容未变化，构建产物已是最新（未触发重新编译）。耗时 {} 秒内。",
                            self.config.timeout.as_secs_f32()
                        );
                        let delta = BuildDelta::default();
                        self.finish_success(
                            trigger,
                            round_start,
                            outcome,
                            true,
                            &delta,
                            Some(note),
                        );
                    }
                    Ok(false) => {
                        // 产物比词库旧 → 改动确实没生效，这是真失败
                        let err = error::deploy_failed(
                            "部署命令已执行，但构建产物早于词库文件，改动未生效",
                        )
                        .with_hint("请尝试重启 fcitx5 后重新部署，或检查 rime 日志");
                        self.finish_failure(trigger, round_start, err, false, None);
                    }
                    Err(e) => {
                        // 无法判定时按「成功但未验证」处理，不谎报失败
                        let note = format!(
                            "部署命令已执行；未能读取产物时间戳以确认生效（{e}），请实际输入验证。"
                        );
                        let delta = BuildDelta::default();
                        self.finish_success(
                            trigger,
                            round_start,
                            outcome,
                            false,
                            &delta,
                            Some(note),
                        );
                    }
                }
            }
        }
    }

    /// 轮询 `build/`，等待产物变化并静止。
    fn wait_for_output(&self, before: &BuildSnapshot, deadline: Instant) -> VerifyOutcome {
        let poll_interval = self.config.poll_interval.max(Duration::from_millis(1));
        let settle_window = self.config.settle_window;
        let mut previous = before.clone();
        let mut changed = false;
        let mut last_change_at: Option<Instant> = None;

        // 「无变化」的提前收敛窗口：真正的重新编译几秒内就会写 build/，
        // 若这么久了还一点没动，基本就是 librime 判断无需重编译。
        // 不提前收敛的话用户要干等满 timeout（实测体验很差）。
        let grace_deadline = Instant::now()
            + self
                .config
                .no_change_grace
                .min(deadline.saturating_duration_since(Instant::now()));

        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            // 还没看到任何变化，且已过收敛窗口 → 直接判定「无变化」
            if !changed && now >= grace_deadline {
                break;
            }
            let snapshot = BuildSnapshot::capture(&self.config.build_dir);
            if snapshot != previous {
                changed = true;
                last_change_at = Some(now);
                previous = snapshot;
            } else if changed {
                if let Some(since) = last_change_at {
                    if now.duration_since(since) >= settle_window {
                        break;
                    }
                }
            }

            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let sleep_for = poll_interval.min(deadline.saturating_duration_since(now));
            if sleep_for.is_zero() {
                break;
            }
            thread::sleep(sleep_for);
        }

        let after = BuildSnapshot::capture(&self.config.build_dir);
        let delta = before.diff(&after);
        if delta.is_empty() {
            return VerifyOutcome::NoChange;
        }
        let settled = last_change_at
            .map(|since| Instant::now().duration_since(since) >= settle_window)
            .unwrap_or(true);
        if settled {
            VerifyOutcome::Verified(delta)
        } else {
            VerifyOutcome::Unsettled(delta)
        }
    }

    fn begin_round(&self, started_at: i64) {
        let snapshot = {
            let mut inner = self.lock_inner();
            inner.state.phase = DeployPhase::Preparing;
            inner.state.started_at = Some(started_at);
            inner.state.finished_at = None;
            inner.state.message = Some("正在准备部署…".to_owned());
            inner.state.clone()
        };
        self.notify(&snapshot);
    }

    fn set_phase(&self, phase: DeployPhase, message: Option<String>) {
        let snapshot = {
            let mut inner = self.lock_inner();
            inner.state.phase = phase;
            inner.state.message = message;
            inner.state.clone()
        };
        self.notify(&snapshot);
    }

    fn finish_success(
        &self,
        trigger: DeployTrigger,
        round_start: Instant,
        outcome: DeployOutcome,
        output_verified: bool,
        delta: &BuildDelta,
        verification_note: Option<String>,
    ) {
        let mut message = format!("部署成功（{}）", outcome.method);
        let mut notes: Vec<String> = Vec::new();
        if let Some(note) = outcome.note {
            notes.push(note);
        }
        if let Some(note) = verification_note {
            notes.push(note);
        }
        if !notes.is_empty() {
            message.push('；');
            message.push_str(&notes.join("；"));
        }
        let log_path = self.write_log(trigger, true, delta, &message, None);
        let result = DeployResult {
            trigger,
            success: true,
            finished_at: Some(now_ms()),
            duration_ms: elapsed_ms(round_start),
            output_verified,
            error: None,
            log_path,
        };
        self.finish(result, message);
    }

    fn finish_failure(
        &self,
        trigger: DeployTrigger,
        round_start: Instant,
        err: AppError,
        output_verified: bool,
        delta: Option<&BuildDelta>,
    ) {
        let empty = BuildDelta::default();
        let delta = delta.unwrap_or(&empty);
        let message = err.message.clone();
        let log_path = self.write_log(trigger, false, delta, &message, err.detail.as_deref());
        let result = DeployResult {
            trigger,
            success: false,
            finished_at: Some(now_ms()),
            duration_ms: elapsed_ms(round_start),
            output_verified,
            error: Some(err),
            log_path,
        };
        self.finish(result, message);
    }

    fn finish(&self, result: DeployResult, message: String) {
        // 顺序保证：先写状态 → 回调观察者 → 再标记本轮完成并唤醒等待者。
        // 这样 `deploy_blocking` 的调用方被唤醒时，观察者一定已经收到终态事件
        // （否则「状态序列」在并发下可能缺最后一帧）。
        let (state_snapshot, round) = {
            let mut inner = self.lock_inner();
            let round = inner.completed_rounds + 1;
            inner.state.phase = if result.success {
                DeployPhase::Success
            } else {
                DeployPhase::Failed
            };
            inner.state.finished_at = Some(now_ms());
            inner.state.message = Some(message);
            inner.state.last_result = Some(result.clone());
            inner.results.push_back((round, result.clone()));
            while inner.results.len() > MAX_RECENT_RESULTS {
                inner.results.pop_front();
            }
            (inner.state.clone(), round)
        };
        self.notify(&state_snapshot);
        {
            let mut inner = self.lock_inner();
            inner.completed_rounds = round;
            self.cv.notify_all();
        }
        info!(
            success = result.success,
            output_verified = result.output_verified,
            duration_ms = result.duration_ms,
            "部署轮次结束"
        );
    }

    fn write_log(
        &self,
        trigger: DeployTrigger,
        success: bool,
        delta: &BuildDelta,
        message: &str,
        detail: Option<&str>,
    ) -> Option<PathBuf> {
        let log_dir = self.config.log_dir.as_ref()?;
        if let Err(err) = std::fs::create_dir_all(log_dir) {
            warn!(dir = %log_dir.display(), %err, "创建部署日志目录失败");
            return None;
        }
        let stamp = now_ms();
        let path = log_dir.join(format!("deploy-{stamp}.log"));
        let content = format!(
            "time={stamp}\ntrigger={trigger:?}\nsuccess={success}\noutput_changes={}\nbuild_dir={}\nmessage={message}\ndetail={}\n",
            delta.summary(),
            self.config.build_dir.display(),
            detail.unwrap_or(""),
        );
        match std::fs::write(&path, content) {
            Ok(()) => Some(path),
            Err(err) => {
                warn!(path = %path.display(), %err, "写部署日志失败");
                None
            }
        }
    }

    fn notify(&self, state: &DeployState) {
        let observer = self
            .observer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(observer) = observer {
            if std::panic::catch_unwind(AssertUnwindSafe(|| observer.on_state(state))).is_err() {
                error!("部署状态观察者 panic，已忽略（不影响部署流程）");
            }
        }
    }

    fn lock_inner(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn wait_for_round(&self, round: u64, timeout: Duration) -> Option<DeployResult> {
        let deadline = Instant::now() + timeout;
        let mut inner = self.lock_inner();
        loop {
            if inner.completed_rounds >= round {
                return inner
                    .results
                    .iter()
                    .find(|(number, _)| *number == round)
                    .map(|(_, result)| result.clone())
                    .or_else(|| inner.state.last_result.clone());
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            let (guard, _) = self
                .cv
                .wait_timeout(inner, deadline.saturating_duration_since(now))
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            inner = guard;
        }
    }
}

enum VerifyOutcome {
    /// 产物有变化且已静止（部署完成）。
    Verified(BuildDelta),
    /// 产物有变化，但到 deadline 时仍在变化（部署可能仍在收尾）。
    Unsettled(BuildDelta),
    /// 产物没有变化：不能判定部署生效。
    NoChange,
}

/// 判断 `build/` 产物是否已不比词库源文件旧。
///
/// 用于「部署命令成功但 build/ 无变化」的场景：librime 增量部署在词库未改动时
/// 会跳过编译，此时产物没变化是**正常且已生效**的；反之若产物早于源文件，
/// 说明这次部署没有把改动编译进去，才是真失败。
pub fn build_is_up_to_date(
    user_dir: &std::path::Path,
    build_dir: &std::path::Path,
) -> Result<bool, std::io::Error> {
    let Some(newest_source) = newest_mtime(user_dir, Some(build_dir), 3, true)? else {
        return Ok(true); // 没有源文件可比对，不做否定判定
    };
    // 注意：产物是 `.bin`，不能用「编译输入」过滤器，否则会扫不到任何产物
    let Some(newest_build) = newest_mtime(build_dir, None, 2, false)? else {
        return Ok(false); // 有源文件但没有任何产物
    };
    Ok(newest_build >= newest_source)
}

/// 目录内（递归、限深）最新修改时间；`skip` 用于跳过 `build/` 自身。
///
/// 只统计**会影响编译产物**的文件（见 [`is_compile_input`]）。
fn newest_mtime(
    dir: &std::path::Path,
    skip: Option<&std::path::Path>,
    max_depth: usize,
    only_compile_inputs: bool,
) -> Result<Option<SystemTime>, std::io::Error> {
    let mut newest: Option<SystemTime> = None;
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    let mut visited = 0usize;

    while let Some((current, depth)) = stack.pop() {
        if visited > 20_000 {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            let path = entry.path();
            if let Some(skip) = skip {
                if path == skip {
                    continue;
                }
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                // 扫产物目录时不排除 build 内的子目录结构，只限制深度
                let descend = if only_compile_inputs {
                    is_compile_dir(&path)
                } else {
                    true
                };
                if depth < max_depth && descend {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            if only_compile_inputs && !is_compile_input(&path) {
                continue;
            }
            if let Ok(mtime) = meta.modified() {
                newest = Some(match newest {
                    Some(prev) if prev >= mtime => prev,
                    _ => mtime,
                });
            }
        }
    }
    Ok(newest)
}

/// 目录是否可能包含编译输入。
///
/// 排除运行时数据：`build/`（产物自身）、`sync/`（同步快照）、`*.userdb/`
/// （用户词频数据库，**每打一个字就变**，实机验证过它会污染新鲜度判断）、
/// 以及随包附带的二进制目录。
fn is_compile_dir(path: &std::path::Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    !(name == "build"
        || name == "sync"
        || name.ends_with(".userdb")
        || name.starts_with("weasel-")
        || name == "others"
        || name.starts_with('.'))
}

/// 文件是否是会影响 `build/` 产物的编译输入。
///
/// 关键排除项（都实测会误判）：
/// - `user.yaml` / `installation.yaml`：部署与安装时写入的运行时状态，
///   部署完反而比产物新，若计入会把「刚部署成功」误判成「产物落后」。
/// - `*.userdb` 内容、`sync/`：同上。
fn is_compile_input(path: &std::path::Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if matches!(name, "user.yaml" | "installation.yaml") {
        return false;
    }
    if name.ends_with(".userdb") || name.ends_with(".ldb") || name.ends_with(".log") {
        return false;
    }
    name.ends_with(".dict.yaml")
        || name.ends_with(".schema.yaml")
        || name.ends_with(".yaml")
        || name.ends_with(".txt")
        || name.ends_with(".gram")
        || name.ends_with(".json")
        || name.ends_with(".lua")
}

fn trigger_priority(trigger: DeployTrigger) -> u8 {
    match trigger {
        DeployTrigger::Manual => 3,
        DeployTrigger::AfterSave => 2,
        DeployTrigger::Scheduled => 1,
    }
}

fn merge_triggers(current: Option<DeployTrigger>, next: DeployTrigger) -> DeployTrigger {
    match current {
        Some(current) if trigger_priority(current) >= trigger_priority(next) => current,
        _ => next,
    }
}

fn trigger_label(trigger: DeployTrigger) -> &'static str {
    match trigger {
        DeployTrigger::Manual => "手动",
        DeployTrigger::AfterSave => "保存后自动",
        DeployTrigger::Scheduled => "定时",
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn elapsed_ms(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_merge_prefers_manual() {
        assert_eq!(
            merge_triggers(Some(DeployTrigger::Scheduled), DeployTrigger::AfterSave),
            DeployTrigger::AfterSave
        );
        assert_eq!(
            merge_triggers(Some(DeployTrigger::AfterSave), DeployTrigger::Scheduled),
            DeployTrigger::AfterSave
        );
        assert_eq!(
            merge_triggers(Some(DeployTrigger::Scheduled), DeployTrigger::Manual),
            DeployTrigger::Manual
        );
        assert_eq!(
            merge_triggers(Some(DeployTrigger::Manual), DeployTrigger::AfterSave),
            DeployTrigger::Manual
        );
        assert_eq!(
            merge_triggers(None, DeployTrigger::Scheduled),
            DeployTrigger::Scheduled
        );
    }

    #[test]
    fn initial_state_is_idle() {
        let deployer: Arc<dyn Deployer> = Arc::new(crate::mock::MockDeployer::new());
        let controller = DeployController::new(deployer, DeployConfig::new("/tmp/rime-test"));
        let state = controller.state();
        assert_eq!(state.phase, DeployPhase::Idle);
        assert!(state.started_at.is_none());
        assert!(state.last_result.is_none());
        assert!(!controller.is_running());
        assert!(controller.recent_results(5).is_empty());
    }
}
