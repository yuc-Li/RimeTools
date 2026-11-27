//! 应用共享状态。
//!
//! 设计约束（见 `docs/ipc.md` §6）：
//! - 受管文件由 `env_list_schemes` 注册，`dict_*` 命令按 `fileId` 查找，避免前端传路径。
//! - 每个打开的词库持有 `DictStore`（含指纹）；保存时用指纹做外部修改检测。
//! - 部署控制器全局唯一，保证并发请求能在同一状态机上合并。

use rime_core::store::DictStore;
use rime_core::types::{AppSettings, FileId, ManagedFileRef, SchemeInfo};
use rime_deploy::DeployController;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Inner {
    /// `env_list_schemes` 注册的受管文件（fileId → 描述）
    managed_files: HashMap<FileId, ManagedFileRef>,
    /// 已识别的方案
    schemes: Vec<SchemeInfo>,
    /// 已打开的词库
    stores: HashMap<FileId, DictStore>,
    settings: AppSettings,
    /// 当前激活方案 id（用于编码建议）
    active_schema: Option<String>,
}

pub struct AppState {
    inner: Mutex<Inner>,
    deploy: Mutex<Option<Arc<DeployController>>>,
    /// 本地字表拼音源缓存（首次生成编码建议时惰性加载，约 8000 字）
    pinyin: Mutex<Option<Arc<std::collections::HashMap<char, Vec<String>>>>>,
    /// 部署状态观察者（由 setup 注入，负责向 WebView emit `deploy://state`）
    deploy_observer: Mutex<Option<Arc<dyn Fn(&rime_core::types::DeployState) + Send + Sync>>>,
    /// 反查缓存（用户目录词库 + 符号表），首次使用时惰性构建
    lookup: Mutex<Option<crate::lookup_cache::SharedLookupCache>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        // 启动即恢复上次的设置（文件不存在/损坏 → 默认值，不阻断启动）
        let settings = crate::settings_file::load().unwrap_or_default();
        Self {
            inner: Mutex::new(Inner {
                settings,
                ..Inner::default()
            }),
            deploy: Mutex::new(None),
            pinyin: Mutex::new(None),
            deploy_observer: Mutex::new(None),
            lookup: Mutex::new(None),
        }
    }

    // ── 受管文件与方案 ──

    pub fn register_schemes(&self, schemes: &[SchemeInfo]) {
        let mut inner = self.lock();
        inner.schemes = schemes.to_vec();
        for s in schemes {
            for mf in &s.managed_files {
                inner.managed_files.insert(mf.id.clone(), mf.clone());
            }
        }
        if inner.active_schema.is_none() {
            inner.active_schema = schemes
                .iter()
                .find(|s| s.is_current)
                .or_else(|| schemes.iter().find(|s| s.is_active))
                .map(|s| s.schema_id.clone());
        }
    }

    pub fn managed_file(&self, file_id: &FileId) -> Option<ManagedFileRef> {
        self.lock().managed_files.get(file_id).cloned()
    }

    /// 全部受管文件路径（去重，备份/部署保护用）。
    pub fn managed_files(&self) -> Vec<std::path::PathBuf> {
        let inner = self.lock();
        let mut seen = std::collections::HashSet::new();
        inner
            .managed_files
            .values()
            .map(|f| f.path.clone())
            .filter(|p| seen.insert(p.clone()))
            .collect()
    }

    /// 外部写入（如从备份恢复）后，重新加载对应词库，避免 UI 仍显示旧内容。
    ///
    /// 这里重新 open 是正确的：数据来源是磁盘，分组等内存元数据会丢，
    /// 但恢复场景下用户是主动回滚，重新加载语义清晰。
    pub fn reload_store_after_external_write(&self, path: &std::path::Path) {
        let mut inner = self.lock();
        let ids: Vec<FileId> = inner
            .managed_files
            .iter()
            .filter(|(_, mf)| mf.path == path)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            let Some(mf) = inner.managed_files.get(&id).cloned() else {
                continue;
            };
            if let Ok(store) = DictStore::open(&mf.path, mf.format, mf.role) {
                inner.stores.insert(id, store);
            }
        }
    }

    pub fn scheme(&self, schema_id: &str) -> Option<SchemeInfo> {
        self.lock()
            .schemes
            .iter()
            .find(|s| s.schema_id == schema_id)
            .cloned()
    }

    pub fn active_scheme(&self) -> Option<SchemeInfo> {
        let inner = self.lock();
        let id = inner.active_schema.clone()?;
        inner.schemes.iter().find(|s| s.schema_id == id).cloned()
    }

    pub fn set_active_schema(&self, schema_id: impl Into<String>) {
        self.lock().active_schema = Some(schema_id.into());
    }

    // ── 词库 ──

    /// 把打开的词库放入状态。
    pub fn put_store(&self, file_id: FileId, store: DictStore) {
        self.lock().stores.insert(file_id, store);
    }

    pub fn has_store(&self, file_id: &FileId) -> bool {
        self.lock().stores.contains_key(file_id)
    }

    /// 对已打开词库执行只读操作；未打开返回 `None`。
    pub fn with_store<R>(&self, file_id: &FileId, f: impl FnOnce(&DictStore) -> R) -> Option<R> {
        self.lock().stores.get(file_id).map(f)
    }

    /// 对已打开词库执行可变操作。
    pub fn with_store_mut<R>(
        &self,
        file_id: &FileId,
        f: impl FnOnce(&mut DictStore) -> R,
    ) -> Option<R> {
        self.lock().stores.get_mut(file_id).map(f)
    }

    /// 保存成功后刷新状态（避免前端每次保存都重新 open）。
    pub fn replace_store(&self, file_id: FileId, store: DictStore) {
        self.lock().stores.insert(file_id, store);
    }

    // ── 设置 ──

    pub fn settings(&self) -> AppSettings {
        self.lock().settings.clone()
    }

    pub fn update_settings(&self, f: impl FnOnce(&mut AppSettings)) -> AppSettings {
        let updated = {
            let mut inner = self.lock();
            f(&mut inner.settings);
            inner.settings.clone()
        };
        // 落盘失败只告警：设置不持久化不该让当前操作失败（用户仍能继续用）
        if let Err(e) = crate::settings_file::save(&updated) {
            log::warn!("保存设置失败：{e}");
        }
        updated
    }

    // ── 部署 ──

    /// 取（或惰性创建）部署控制器。
    ///
    /// 惰性创建的原因：部署控制器需要用户目录，而用户目录在环境探测后才确定；
    /// 且不是所有操作都需要它（浏览词库时不该初始化 D-Bus 客户端）。
    pub fn deploy_controller(&self, user_dir: &std::path::Path) -> Arc<DeployController> {
        let mut guard = self.deploy.lock().expect("部署控制器锁中毒");
        if let Some(ctl) = guard.as_ref() {
            return Arc::clone(ctl);
        }
        let deployer: Arc<dyn rime_deploy::Deployer> =
            Arc::new(rime_deploy::Fcitx5Deployer::system());
        let config = rime_deploy::DeployConfig::new(user_dir);
        let ctl = DeployController::new(deployer, config);

        // 把状态变化转发到前端：契约 docs/ipc.md §4 的 deploy://state
        if let Some(observer) = self
            .deploy_observer
            .lock()
            .expect("观察者锁中毒")
            .as_ref()
            .cloned()
        {
            ctl.set_observer(Some(Arc::new(
                move |state: &rime_core::types::DeployState| observer(state),
            )));
        }

        *guard = Some(Arc::clone(&ctl));
        ctl
    }

    // ── 备份目录（唯一事实源） ──

    /// 备份目录：
    /// 1. 用户显式配置的 `backup.dir` 优先；
    /// 2. 否则 `$XDG_DATA_HOME/rimetools/backups`（回退 `~/.local/share/rimetools/backups`）。
    ///
    /// 默认放在 **Rime 用户目录之外**：避免 `.bak` 文件混进输入法配置，
    /// 被 Rime 同步/部署逻辑误扫；同时保证「写前自动备份」与「备份页」看到同一批文件。
    pub fn backup_dir(&self) -> std::path::PathBuf {
        if let Some(dir) = self.settings().backup.dir {
            return dir;
        }
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(std::path::PathBuf::from)
                    .map(|h| h.join(".local/share"))
            })
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
        base.join("rimetools/backups")
    }

    // ── 拼音数据源 ──

    /// 取本地字表拼音源（惰性加载并缓存）。
    ///
    /// 优先用用户本机字表（覆盖 8105 字、含多音字），退化时返回 `None`，
    /// 由调用方回落到 `rime-suggest` 的内置常用字表。
    pub fn pinyin_source(
        &self,
        user_dir: &std::path::Path,
    ) -> Option<Arc<std::collections::HashMap<char, Vec<String>>>> {
        if let Some(cached) = self.pinyin.lock().expect("拼音缓存锁中毒").as_ref() {
            return Some(Arc::clone(cached));
        }
        let map = rime_platform::load_char_pinyin(user_dir);
        if map.is_empty() {
            return None;
        }
        let arc = Arc::new(map);
        *self.pinyin.lock().expect("拼音缓存锁中毒") = Some(Arc::clone(&arc));
        Some(arc)
    }

    /// 注入部署状态观察者（setup 阶段调用一次）。
    pub fn attach_deploy_observer(
        &self,
        observer: Arc<dyn Fn(&rime_core::types::DeployState) + Send + Sync>,
    ) {
        *self.deploy_observer.lock().expect("观察者锁中毒") = Some(observer);
    }

    pub fn existing_deploy_controller(&self) -> Option<Arc<DeployController>> {
        self.deploy.lock().expect("部署控制器锁中毒").clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl AppState {
    /// 取反查缓存（未构建时为 None）。
    pub fn lookup_cache(&self) -> Option<crate::lookup_cache::SharedLookupCache> {
        self.lookup.lock().ok().and_then(|guard| guard.clone())
    }

    /// 替换反查缓存。
    pub fn set_lookup_cache(&self, cache: crate::lookup_cache::SharedLookupCache) {
        if let Ok(mut guard) = self.lookup.lock() {
            *guard = Some(cache);
        }
    }
}
