//! 部署状态机 / 并发合并 / 产物判定的集成测试。
//!
//! 全部使用 [`MockDeployer`]，**不会触发任何真实部署、不碰用户真实 Rime 目录**
//! （`build_dir` 指向 tempdir）。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rime_core::types::{DeployPhase, DeployState, DeployTrigger};
use rime_deploy::mock::{Gate, MockBehavior, MockDeployer};
use rime_deploy::{
    AppError, DeployConfig, DeployController, DeployPhase as ReexportedPhase, DeployResult,
    ErrorCode, RequestOutcome,
};

fn setup() -> (tempfile::TempDir, DeployConfig) {
    let temp = tempfile::tempdir().expect("tempdir");
    let user_dir = temp.path().join("rime");
    std::fs::create_dir_all(&user_dir).expect("创建临时用户目录");
    let config = DeployConfig::new(&user_dir)
        .with_timeout(Duration::from_secs(5))
        .with_poll_interval(Duration::from_millis(5))
        .with_settle_window(Duration::from_millis(20));
    (temp, config)
}

fn controller(config: DeployConfig, mock: &Arc<MockDeployer>) -> Arc<DeployController> {
    let deployer: Arc<dyn rime_deploy::Deployer> =
        Arc::clone(mock) as Arc<dyn rime_deploy::Deployer>;
    DeployController::new(deployer, config)
}

fn record_phases(controller: &Arc<DeployController>) -> Arc<Mutex<Vec<DeployPhase>>> {
    let phases = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&phases);
    controller.set_observer(Some(Arc::new(move |state: &DeployState| {
        recorder.lock().expect("phases lock").push(state.phase);
    })));
    phases
}

fn recorded_phases(phases: &Arc<Mutex<Vec<DeployPhase>>>) -> Vec<DeployPhase> {
    phases.lock().expect("phases lock").clone()
}

fn build_dir(config: &DeployConfig) -> &Path {
    &config.build_dir
}

#[test]
fn success_path_verifies_output_and_reports_full_phase_sequence() {
    let (_temp, config) = setup();
    let mock = Arc::new(MockDeployer::mutating());
    let controller = controller(config.clone(), &mock);
    let phases = record_phases(&controller);

    let result: DeployResult = controller
        .deploy_blocking(DeployTrigger::Manual)
        .expect("等待本轮部署");

    assert!(result.success, "result: {result:?}");
    assert!(
        result.output_verified,
        "build/ 产物变化必须被检测到：{result:?}"
    );
    assert_eq!(result.trigger, DeployTrigger::Manual);
    assert!(result.error.is_none());
    assert!(result.duration_ms < 5_000);
    assert!(build_dir(&config).join("mock-build.out").is_file());

    let state = controller.state();
    assert_eq!(state.phase, DeployPhase::Success);
    assert!(state.started_at.is_some());
    assert!(state.finished_at.is_some());
    assert!(state.last_result.as_ref().expect("last_result").success);
    assert!(state
        .message
        .as_deref()
        .unwrap_or_default()
        .contains("部署成功"));

    assert_eq!(
        recorded_phases(&phases),
        vec![
            DeployPhase::Preparing,
            DeployPhase::Running,
            DeployPhase::Verifying,
            DeployPhase::Success
        ],
        "状态序列必须严格按状态机推进"
    );
    assert!(!controller.is_running());

    let recent = controller.recent_results(5);
    assert_eq!(recent.len(), 1);
    assert!(recent[0].success);
}

#[test]
fn no_output_change_but_build_up_to_date_is_success() {
    // 关键场景（实机验证过）：librime 增量部署在词库未改动时会跳过重新编译，
    // build/ 一个字节都不会变，但部署确实成功了（用户能看到 Rime 的「部署完成」）。
    // 因此「产物无变化 + 产物已不比源文件旧」必须判为成功，不能误报超时/失败。
    let (temp, mut config) = setup();
    config.timeout = Duration::from_millis(120);
    config.poll_interval = Duration::from_millis(5);
    config.settle_window = Duration::from_millis(10);

    // 造一个「源文件很旧、产物更新」的目录：模拟“已是最新”
    let user_dir = temp.path().join("user");
    let build_dir = temp.path().join("build");
    std::fs::create_dir_all(&user_dir).unwrap();
    std::fs::create_dir_all(&build_dir).unwrap();
    std::fs::write(user_dir.join("custom_phrase_double.txt"), b"a\tb\n").unwrap();
    config.user_dir = user_dir;
    config.build_dir = build_dir.clone();
    // 产物写得比源文件晚
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(build_dir.join("xhup.table.bin"), b"bin").unwrap();

    let mock = Arc::new(MockDeployer::new()); // 命令成功但 build/ 无变化
    let controller = controller(config, &mock);

    let result = controller
        .deploy_blocking(DeployTrigger::Manual)
        .expect("等待本轮部署");

    assert!(result.success, "产物已是最新时必须判成功：{result:?}");
    assert!(result.error.is_none(), "不应带错误");
    assert_eq!(controller.state().phase, DeployPhase::Success);
}

#[test]
fn no_output_change_and_build_older_than_source_is_failure() {
    // 反面：产物比词库源文件旧 → 改动确实没编译进去，是真失败
    let (temp, mut config) = setup();
    config.timeout = Duration::from_millis(120);
    config.poll_interval = Duration::from_millis(5);
    config.settle_window = Duration::from_millis(10);

    let user_dir = temp.path().join("user2");
    let build_dir = temp.path().join("build2");
    std::fs::create_dir_all(&user_dir).unwrap();
    std::fs::create_dir_all(&build_dir).unwrap();
    std::fs::write(build_dir.join("xhup.table.bin"), b"bin").unwrap();
    std::thread::sleep(Duration::from_millis(20));
    // 源文件更新（模拟刚保存过的改动）
    std::fs::write(user_dir.join("custom_phrase_double.txt"), b"a\tb\n").unwrap();
    config.user_dir = user_dir;
    config.build_dir = build_dir;

    let mock = Arc::new(MockDeployer::new());
    let controller = controller(config, &mock);

    let result = controller
        .deploy_blocking(DeployTrigger::AfterSave)
        .expect("等待本轮部署");

    assert!(!result.success, "产物落后于源文件不能判成功：{result:?}");
    assert_eq!(
        result.error.as_ref().expect("应带错误").code,
        ErrorCode::DeployFailed,
        "应报 DeployFailed（未生效），而不是 DeployTimeout（超时）"
    );
    assert_eq!(controller.state().phase, DeployPhase::Failed);
}

#[test]
fn deployer_error_marks_failed_with_deploy_failed_code() {
    let (_temp, config) = setup();
    let mock = Arc::new(MockDeployer::new().with_behavior(MockBehavior {
        error: Some(AppError::new(
            ErrorCode::DeployFailed,
            "fcitx5 拒绝本次部署",
        )),
        ..MockBehavior::default()
    }));
    let controller = controller(config, &mock);
    let phases = record_phases(&controller);

    let result = controller
        .deploy_blocking(DeployTrigger::Manual)
        .expect("等待本轮部署");
    assert!(!result.success);
    assert!(!result.output_verified);
    let error = result.error.expect("应带错误");
    assert_eq!(error.code, ErrorCode::DeployFailed);
    assert!(error.message.contains("拒绝本次部署"));
    assert_eq!(controller.state().phase, DeployPhase::Failed);
    assert_eq!(
        controller.state().message.as_deref(),
        Some("fcitx5 拒绝本次部署")
    );
    assert_eq!(
        recorded_phases(&phases),
        vec![
            DeployPhase::Preparing,
            DeployPhase::Running,
            DeployPhase::Failed
        ]
    );
}

#[test]
fn method_overrun_is_deploy_timeout() {
    let (_temp, mut config) = setup();
    config.timeout = Duration::from_millis(60);
    let mock = Arc::new(MockDeployer::new().with_behavior(MockBehavior {
        delay: Some(Duration::from_millis(200)),
        ..MockBehavior::default()
    }));
    let controller = controller(config, &mock);

    let started = Instant::now();
    let result = controller
        .deploy_blocking(DeployTrigger::Manual)
        .expect("等待本轮部署");
    assert!(!result.success);
    assert_eq!(
        result.error.expect("应带错误").code,
        ErrorCode::DeployTimeout
    );
    assert!(started.elapsed() >= Duration::from_millis(190));
    assert_eq!(controller.state().phase, DeployPhase::Failed);
}

#[test]
fn concurrent_requests_merge_into_single_supplementary_run() {
    let (_temp, config) = setup();
    let gate = Gate::new();
    let mock = Arc::new(MockDeployer::mutating().with_gate(Arc::clone(&gate)));
    let controller = controller(config, &mock);

    assert_eq!(
        controller
            .request(DeployTrigger::AfterSave)
            .expect("启动首轮"),
        RequestOutcome::Started
    );
    assert!(
        mock.wait_started(1, Duration::from_secs(2)),
        "首轮应进入 Running 并被闸门卡住"
    );

    // Running 期间连发 5 次（混合触发来源，手动优先级最高）。
    for trigger in [
        DeployTrigger::Scheduled,
        DeployTrigger::AfterSave,
        DeployTrigger::Scheduled,
        DeployTrigger::Manual,
        DeployTrigger::Scheduled,
    ] {
        assert_eq!(
            controller.request(trigger).expect("合并请求"),
            RequestOutcome::Merged
        );
    }
    assert_eq!(mock.calls(), 1, "Running 期间不得并发执行第二轮");
    assert!(controller.is_running());

    gate.open();
    assert!(
        controller.wait_until_idle(Duration::from_secs(5)),
        "部署线程应在放行后结束"
    );

    assert_eq!(mock.calls(), 2, "5 次合并请求只应补跑 1 次（不是 6 次）");
    assert_eq!(
        mock.triggers(),
        vec![DeployTrigger::AfterSave, DeployTrigger::Manual],
        "pending 应合并为优先级最高的 Manual"
    );
    assert_eq!(controller.state().phase, DeployPhase::Success);
    assert_eq!(controller.recent_results(10).len(), 2);
    assert_eq!(mock.finished_count(), 2);
}

#[test]
fn sequential_requests_after_idle_start_new_round() {
    let (_temp, config) = setup();
    let mock = Arc::new(MockDeployer::mutating());
    let controller = controller(config, &mock);

    assert!(
        controller
            .deploy_blocking(DeployTrigger::Manual)
            .expect("首轮")
            .success
    );
    assert!(
        controller
            .deploy_blocking(DeployTrigger::AfterSave)
            .expect("次轮")
            .success
    );
    assert_eq!(
        mock.calls(),
        2,
        "空闲后的新请求应开启新轮，而不是沿用合并逻辑"
    );
    assert_eq!(controller.recent_results(10).len(), 2);
}

#[test]
fn invalid_user_dir_does_not_trigger_deployer() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config = DeployConfig::new(temp.path().join("missing-rime-dir"));
    let mock = Arc::new(MockDeployer::mutating());
    let controller = controller(config, &mock);

    let result = controller
        .deploy_blocking(DeployTrigger::Manual)
        .expect("等待本轮部署");
    assert!(!result.success);
    assert_eq!(
        result.error.expect("应带错误").code,
        ErrorCode::UserDirInvalid
    );
    assert_eq!(mock.calls(), 0, "用户目录无效时绝不能调用部署方式");
    assert_eq!(controller.state().phase, DeployPhase::Failed);
}

#[test]
fn observer_panic_does_not_break_deployment() {
    let (_temp, config) = setup();
    let mock = Arc::new(MockDeployer::mutating());
    let controller = controller(config, &mock);
    controller.set_observer(Some(Arc::new(|_: &DeployState| panic!("观察者故意 panic"))));

    let result = controller
        .deploy_blocking(DeployTrigger::Manual)
        .expect("等待本轮部署");
    assert!(result.success, "回调 panic 不应中断部署：{result:?}");
}

#[test]
fn writes_log_when_log_dir_configured() {
    let (temp, config) = setup();
    let config = config.with_log_dir(temp.path().join("logs"));
    let mock = Arc::new(MockDeployer::mutating());
    let controller = controller(config, &mock);

    let result = controller
        .deploy_blocking(DeployTrigger::Manual)
        .expect("等待本轮部署");
    let log_path: PathBuf = result.log_path.expect("应写部署日志");
    assert!(log_path.is_file(), "日志文件不存在：{}", log_path.display());
    let content = std::fs::read_to_string(&log_path).expect("读日志");
    assert!(content.contains("success=true"));
    assert!(content.contains("mock-build.out") || content.contains("新增 1"));
}

#[test]
fn public_reexports_match_contract_names() {
    // 编译期断言：上层只依赖 rime_deploy 时也能拿到契约类型。
    let _ = ReexportedPhase::Idle;
}

#[test]
fn no_change_converges_early_instead_of_waiting_full_timeout() {
    // 「产物无变化」不应该让用户干等满 timeout：
    // 真正的重新编译几秒内就会写 build/，超过 no_change_grace 仍毫无变化
    // 说明 librime 判断无需重编译，应立即给结论。
    let (temp, mut config) = setup();
    config.timeout = Duration::from_secs(10); // 完整超时很长
    config.poll_interval = Duration::from_millis(10);
    config.settle_window = Duration::from_millis(10);
    config.no_change_grace = Duration::from_millis(150); // 但收敛窗口很短

    let user_dir = temp.path().join("u");
    let build_dir = temp.path().join("b");
    std::fs::create_dir_all(&user_dir).unwrap();
    std::fs::create_dir_all(&build_dir).unwrap();
    std::fs::write(user_dir.join("x.txt"), b"a").unwrap();
    std::thread::sleep(Duration::from_millis(10));
    std::fs::write(build_dir.join("x.table.bin"), b"bin").unwrap(); // 产物更新 → 已是最新
    config.user_dir = user_dir;
    config.build_dir = build_dir;

    let mock = Arc::new(MockDeployer::new());
    let controller = controller(config, &mock);

    let started = Instant::now();
    let result = controller
        .deploy_blocking(DeployTrigger::Manual)
        .expect("等待本轮部署");
    let elapsed = started.elapsed();

    assert!(result.success, "产物已是最新应判成功：{result:?}");
    assert!(
        elapsed < Duration::from_secs(3),
        "应在收敛窗口内给出结论，实际耗时 {elapsed:?}（不应等满 10s 超时）"
    );
}
