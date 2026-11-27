//! 词库读写命令（契约 `docs/ipc.md` §3.2）。
//!
//! 保存流程严格遵循 §5.1：
//! `取 store → 暂存 → 校验 → 序列化 → 写前备份+原子写（rime-safety）→ 刷新状态 → 可选部署`
//!
//! 数据安全红线：指纹不一致必须拒绝写入，绝不覆盖。

use crate::state::AppState;
use rime_core::store::DictStore;
use rime_core::types::{
    AppError, EntryPage, EntryQuery, ErrorCode, FileId, Fingerprint, GroupCount, ManagedFileRef,
    SaveRequest, SaveResult, ValidationIssue,
};
use rime_safety::{save_file, WriteSafetyOptions};
use serde::Serialize;
use tauri::State;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictOpenResult {
    #[serde(rename = "ref")]
    pub file_ref: ManagedFileRef,
    pub total: u32,
    pub fingerprint: Fingerprint,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidateResult {
    pub ok: bool,
    pub errors: Vec<ValidationIssue>,
}

/// 打开词库：解析文件、建立内存索引、记录指纹。
#[tauri::command]
pub fn dict_open(file_id: FileId, state: State<'_, AppState>) -> Result<DictOpenResult, AppError> {
    let file_ref = state.managed_file(&file_id).ok_or_else(|| {
        AppError::new(ErrorCode::NotFound, format!("未注册的受管文件：{file_id}"))
            .with_hint("请先调用 env_list_schemes 完成环境探测")
    })?;

    let store = DictStore::open(&file_ref.path, file_ref.format, file_ref.role.clone())?;
    let result = DictOpenResult {
        total: store.total() as u32,
        fingerprint: store.fingerprint().clone(),
        file_ref: file_ref.clone(),
    };
    state.put_store(file_id, store);
    Ok(result)
}

/// 分页查询。
///
/// 注意 fileId 的两种表示：
/// - 前端持有的是**受管文件 id**（`cp:<路径>` / `dict:<路径>`，见 `env_list_schemes`）；
/// - `DictStore` 内部以**文件路径**标识自己。
///
/// 两者并不相等，若把前端的 id 直接透传给 `store.query()` 做过滤，会一条都查不到
/// （实机踩过：列表空白、总数为 0）。这里已按 id 路由到具体 store，
/// store 层不需要再按 file_id 过滤，返回时再把 id 改回前端认识的那个。
#[tauri::command]
pub fn dict_query(query: EntryQuery, state: State<'_, AppState>) -> Result<EntryPage, AppError> {
    let file_id = query
        .file_id
        .clone()
        .ok_or_else(|| AppError::new(ErrorCode::NotFound, "查询缺少 fileId"))?;

    let mut store_query = query.clone();
    store_query.file_id = None; // 已按 id 路由，store 内是单文件，无需二次过滤

    let mut page = state
        .with_store(&file_id, |store| store.query(&store_query))
        .ok_or_else(|| not_opened(&file_id))?;

    // 把 fileId 统一回前端使用的受管文件 id
    for entry in &mut page.items {
        entry.key.file_id = file_id.clone();
    }
    Ok(page)
}

/// 分组计数（侧栏）。
#[tauri::command]
pub fn dict_groups(
    file_id: FileId,
    state: State<'_, AppState>,
) -> Result<Vec<GroupCount>, AppError> {
    state
        .with_store(&file_id, |store| store.group_counts())
        .ok_or_else(|| not_opened(&file_id))
}

/// 保存前校验：返回逐条问题供 UI 定位，不写盘。
#[tauri::command]
pub fn dict_validate(
    request: SaveRequest,
    state: State<'_, AppState>,
) -> Result<ValidateResult, AppError> {
    let file_id = request.file_id.clone();
    let (alphabet, max_code_length) = scheme_limits(&state, &file_id);

    let errors = state
        .with_store_mut(&file_id, |store| {
            // 校验在暂存的副本上进行，不影响现有状态
            let mut probe = store.clone();
            probe.stage(&request)?;
            Ok::<_, AppError>(probe.validate(&alphabet, max_code_length))
        })
        .ok_or_else(|| not_opened(&file_id))??;

    Ok(ValidateResult {
        ok: errors.is_empty(),
        errors,
    })
}

/// 保存：暂存 → 校验 → 备份+原子写 → 刷新 → 可选部署。
#[tauri::command]
pub fn dict_save(request: SaveRequest, state: State<'_, AppState>) -> Result<SaveResult, AppError> {
    let file_id = request.file_id.clone();
    let file_ref = state
        .managed_file(&file_id)
        .ok_or_else(|| not_opened(&file_id))?;

    let (alphabet, max_code_length) = scheme_limits(&state, &file_id);
    let settings = state.settings();

    // 1) 暂存 + 校验 + 序列化
    //
    // 校验失败时必须在真实 store 上 `discard_staged()`：stage 会写入暂存态，
    // 若残留会让 UI 重查时看到"已保存但其实没保存"的假象。
    let prepared = state
        .with_store_mut(&file_id, |store| {
            store.stage(&request)?;
            let issues = store.validate(&alphabet, max_code_length);
            if !issues.is_empty() {
                store.discard_staged();
                return Err(AppError::new(
                    ErrorCode::ValidationFailed,
                    format!("校验未通过（{} 项问题），已取消保存", issues.len()),
                )
                .with_detail(render_issues(&issues)));
            }
            store.serialize()
        })
        .ok_or_else(|| not_opened(&file_id))??;

    let (content, _opts) = prepared;

    // 2) 写前备份 + 原子写；指纹不一致会在此被拒绝
    let safety_options = WriteSafetyOptions {
        // 与备份页共用同一个目录（state.backup_dir），否则写前自动备份
        // 会落在源文件旁边、在备份页里看不到
        backup_dir: Some(state.backup_dir()),
        fingerprint_policy: Default::default(),
        validate_staged_utf8: true,
        prune_keep_count: settings
            .backup
            .enabled
            .then_some(settings.backup.keep_count),
    };

    let outcome = save_file(
        &file_ref.path,
        &request.expected_fingerprint,
        content.as_bytes(),
        &safety_options,
    )
    .map_err(|e| {
        // 写入失败（含 ExternalModified）：清掉暂存，保持 store 与磁盘一致
        state.with_store_mut(&file_id, |store| store.discard_staged());
        e
    })?;

    // 记录写前自动备份的原因，保证备份页显示「写前自动」而不是未知
    crate::backup_index::record(&state.backup_dir(), std::slice::from_ref(&outcome.backup));

    // 3) 固化暂存态并刷新指纹。
    //    这里**不能**重新 DictStore::open：group 是工具级内存元数据，
    //    重开会让用户刚设置的分组消失。
    let fingerprint = outcome.fingerprint.clone();
    state.with_store_mut(&file_id, |store| store.mark_saved(fingerprint));

    // 4) 可选部署
    let deploy = if request.deploy_after && settings.auto_deploy_after_save {
        Some(trigger_deploy(
            &state,
            rime_core::types::DeployTrigger::AfterSave,
        )?)
    } else {
        None
    };

    Ok(SaveResult {
        written: outcome.bytes_written as u32,
        backup: Some(outcome.backup),
        fingerprint: outcome.fingerprint,
        deploy,
    })
}

/// 触发部署并等待结果（保存后自动部署场景需要结果反馈）。
pub(crate) fn trigger_deploy(
    state: &AppState,
    trigger: rime_core::types::DeployTrigger,
) -> Result<rime_core::types::DeployResult, AppError> {
    let user_dir = state
        .settings()
        .user_dir
        .or_else(|| {
            rime_platform::detect_frontends()
                .into_iter()
                .next()
                .map(|f| f.user_dir)
        })
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::DeployUnavailable,
                "未确定 Rime 用户目录，无法部署",
            )
        })?;

    let controller = state.deploy_controller(&user_dir);
    controller.deploy_blocking(trigger)
}

fn not_opened(file_id: &str) -> AppError {
    AppError::new(ErrorCode::NotFound, format!("词库尚未打开：{file_id}"))
        .with_hint("请先调用 dict_open")
}

/// 从受管文件所属方案取出编码约束（字符集与最大码长）。
fn scheme_limits(state: &AppState, file_id: &FileId) -> (String, Option<u32>) {
    let managed = state.managed_file(file_id);
    if let Some(mf) = managed {
        // 找到挂载该文件的方案
        if let Some(scheme) = state
            .active_scheme()
            .filter(|s| s.managed_files.iter().any(|f| f.path == mf.path))
        {
            return (scheme.alphabet.clone(), scheme.max_code_length);
        }
    }
    // 兜底：不做字符集限制（空 alphabet 表示全部允许）
    (String::new(), None)
}

fn render_issues(issues: &[ValidationIssue]) -> String {
    issues
        .iter()
        .take(20)
        .map(|i| {
            let where_ = i
                .line_no
                .map(|n| format!("第 {n} 行"))
                .unwrap_or_else(|| "—".to_owned());
            let entry = match (&i.text, &i.entry_code) {
                (Some(t), Some(c)) => format!("{t}/{c}"),
                _ => String::new(),
            };
            format!("{where_} {entry}：{}", i.message)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
