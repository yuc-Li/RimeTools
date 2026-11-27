/**
 * 设置页（M1 / PRD FR-8；T1.9c）。
 *
 * 数据来源（`docs/ipc.md`）：
 * - §3.1 `env_detect`：前端、版本、用户/共享目录、部署能力（环境区）
 * - §3.1 `env_list_schemes`：方案 + 受管文件映射（方案区）
 * - §3.1 `env_set_user_dir`：手动指定用户目录（用户目录区）
 * - §3.7 `settings_get` / `settings_update` / `settings_reset`：设置读取、部分更新、恢复默认
 *
 * 交互模型：**显式保存**。所有开关/选项先改本地草稿并标记 dirty，底部保存条统一
 * 调用 `settings_update`（只传变化字段，字段名 camelCase）；保存失败在页面顶部回显
 * `AppError.message/hint/detail`。用户目录是独立的环境动作（`env_set_user_dir`），
 * 有自己的「应用目录」按钮与错误回显。
 *
 * 不负责：路由与主题实际应用（App.tsx 由 lead 接线）。本组件默认导出、无必填 props。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toAppError } from "../../ipc/client";
import type { AppError, AppSettings, BackupSettings, SchemeInfo, Theme } from "../../ipc/types";
import { Badge, ErrorAlert, Spinner } from "../../components/Feedback";
import { isMockMode, settingsApi, type EnvDetectResult } from "./api";
import { SchemeConfigSection } from "./SchemeConfigSection";
import {
  AppearanceSection,
  BackupSection,
  EnvironmentSection,
  ResetSection,
  SaveDeploySection,
  SchemesSection,
  Section,
  UserDirSection,
} from "./sections";
import "./settings.css";

/** 页面可编辑的设置子集（语言 M2 前只读，不进入草稿） */
interface SettingsDraft {
  autoDeployAfterSave: boolean;
  backup: BackupSettings;
  theme: Theme;
}

const DIRTY_LABEL: Record<string, string> = {
  autoDeployAfterSave: "保存后自动部署",
  theme: "主题",
  backup: "备份策略",
};

function draftFromSettings(settings: AppSettings): SettingsDraft {
  return {
    autoDeployAfterSave: settings.autoDeployAfterSave,
    backup: { ...settings.backup },
    theme: settings.theme,
  };
}

function sameBackup(a: BackupSettings, b: BackupSettings): boolean {
  return (
    a.enabled === b.enabled &&
    a.keepCount === b.keepCount &&
    (a.dir ?? null) === (b.dir ?? null) &&
    a.scheduledEnabled === b.scheduledEnabled &&
    a.scheduledIntervalHours === b.scheduledIntervalHours
  );
}

/** 只构造变化字段，避免部分更新时误覆盖（真实后端与 mock 都按 patch 语义处理） */
function buildPatch(settings: AppSettings, draft: SettingsDraft): Partial<AppSettings> {
  const patch: Partial<AppSettings> = {};
  if (draft.autoDeployAfterSave !== settings.autoDeployAfterSave) {
    patch.autoDeployAfterSave = draft.autoDeployAfterSave;
  }
  if (draft.theme !== settings.theme) {
    patch.theme = draft.theme;
  }
  if (!sameBackup(draft.backup, settings.backup)) {
    // 传完整 backup：真实后端按子字段合并，mock 是整对象替换，完整对象两边都安全
    patch.backup = { ...draft.backup };
  }
  return patch;
}

function isAbsolutePath(path: string): boolean {
  return path.startsWith("/") || /^[A-Za-z]:[\\/]/.test(path) || path.startsWith("\\\\");
}

export default function Settings() {
  const mockMode = isMockMode();

  // ── 环境（env_detect） ──
  const [env, setEnv] = useState<EnvDetectResult | null>(null);
  const [envLoading, setEnvLoading] = useState(true);
  const [envError, setEnvError] = useState<AppError | null>(null);

  // ── 方案与受管文件（env_list_schemes） ──
  const [schemes, setSchemes] = useState<SchemeInfo[] | null>(null);
  const [schemesLoading, setSchemesLoading] = useState(true);
  const [schemesError, setSchemesError] = useState<AppError | null>(null);

  // ── 设置（settings_get / settings_update） ──
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [settingsLoading, setSettingsLoading] = useState(true);
  // 每次采用后端返回的完整设置时自增：让带本地输入缓冲的子组件（如保留份数）重新同步
  const [settingsRevision, setSettingsRevision] = useState(0);
  const [settingsError, setSettingsError] = useState<AppError | null>(null);
  const [draft, setDraft] = useState<SettingsDraft | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<AppError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  // ── 用户目录（env_set_user_dir） ──
  const [dirInput, setDirInput] = useState("");
  const [dirSaving, setDirSaving] = useState(false);
  const [dirError, setDirError] = useState<AppError | null>(null);
  const [dirNotice, setDirNotice] = useState<string | null>(null);
  const dirInitRef = useRef(false);

  // ── 恢复默认（settings_reset） ──
  const [resetOpen, setResetOpen] = useState(false);
  const [resetBusy, setResetBusy] = useState(false);
  const [resetError, setResetError] = useState<AppError | null>(null);

  // ───────────────────────── 加载 ─────────────────────────

  const adoptSettings = useCallback((next: AppSettings) => {
    setSettings(next);
    setSettingsRevision((value) => value + 1);
  }, []);

  const loadEnv = useCallback(async () => {
    setEnvLoading(true);
    setEnvError(null);
    try {
      setEnv(await settingsApi.detectEnv());
    } catch (error) {
      setEnvError(toAppError(error));
    } finally {
      setEnvLoading(false);
    }
  }, []);

  const loadSchemes = useCallback(async () => {
    setSchemesLoading(true);
    setSchemesError(null);
    try {
      setSchemes(await settingsApi.listSchemes());
    } catch (error) {
      setSchemesError(toAppError(error));
    } finally {
      setSchemesLoading(false);
    }
  }, []);

  const loadSettings = useCallback(async () => {
    setSettingsLoading(true);
    setSettingsError(null);
    try {
      const loaded = await settingsApi.getSettings();
      adoptSettings(loaded);
      // 首次加载才初始化草稿；后续刷新（如用户目录切换）不丢弃未保存改动
      setDraft((previous) => previous ?? draftFromSettings(loaded));
    } catch (error) {
      setSettingsError(toAppError(error));
    } finally {
      setSettingsLoading(false);
    }
  }, [adoptSettings]);

  useEffect(() => {
    void loadEnv();
    void loadSchemes();
    void loadSettings();
  }, [loadEnv, loadSchemes, loadSettings]);

  // 用户目录输入框的初值：优先手动指定的设置，其次自动探测结果
  useEffect(() => {
    const value =
      settings?.userDir ??
      env?.active?.userDir ??
      env?.frontends[0]?.userDir ??
      null;
    if (!dirInitRef.current && value) {
      setDirInput(value);
      dirInitRef.current = true;
    }
  }, [settings, env]);

  // 提示 6s 后自动消失（用户目录提示保留到下一次操作）
  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(null), 6000);
    return () => clearTimeout(timer);
  }, [notice]);

  const refreshAll = useCallback(() => {
    void loadEnv();
    void loadSchemes();
    void loadSettings();
  }, [loadEnv, loadSchemes, loadSettings]);

  // ───────────────────────── 草稿与保存 ─────────────────────────

  const dirtyFields = useMemo(() => {
    if (!settings || !draft) return [] as string[];
    const fields: string[] = [];
    if (draft.autoDeployAfterSave !== settings.autoDeployAfterSave) {
      fields.push("autoDeployAfterSave");
    }
    if (draft.theme !== settings.theme) fields.push("theme");
    if (!sameBackup(draft.backup, settings.backup)) fields.push("backup");
    return fields;
  }, [settings, draft]);

  const dirty = dirtyFields.length > 0;

  const setAutoDeploy = useCallback((value: boolean) => {
    setDraft((previous) => (previous ? { ...previous, autoDeployAfterSave: value } : previous));
    setSaveError(null);
  }, []);

  const setTheme = useCallback((value: Theme) => {
    setDraft((previous) => (previous ? { ...previous, theme: value } : previous));
    setSaveError(null);
  }, []);

  const setBackup = useCallback((patch: Partial<BackupSettings>) => {
    setDraft((previous) =>
      previous ? { ...previous, backup: { ...previous.backup, ...patch } } : previous,
    );
    setSaveError(null);
  }, []);

  const discard = useCallback(() => {
    if (!settings) return;
    setDraft(draftFromSettings(settings));
    setSaveError(null);
    setNotice("已放弃未保存的改动");
  }, [settings]);

  /** §3.7 settings_update：显式保存（部分更新） */
  const saveNow = useCallback(async () => {
    if (!settings || !draft || !dirty || saving) return;
    const patch = buildPatch(settings, draft);
    if (Object.keys(patch).length === 0) return;

    setSaving(true);
    setSaveError(null);
    setNotice(null);
    try {
      const updated = await settingsApi.updateSettings(patch);
      adoptSettings(updated);
      setDraft(draftFromSettings(updated));
      setNotice("设置已保存");
    } catch (error) {
      // 保存失败必须回显：AppError.message + hint + detail（折叠）
      setSaveError(toAppError(error));
    } finally {
      setSaving(false);
    }
  }, [settings, draft, dirty, saving, adoptSettings]);

  // ───────────────────────── 用户目录 ─────────────────────────

  const applyUserDir = useCallback(async () => {
    const path = dirInput.trim();
    setDirError(null);
    setDirNotice(null);

    if (!path) {
      setDirError({
        code: "userDirInvalid",
        message: "用户目录不能为空",
        hint: "请输入 Rime 用户目录的绝对路径，例如 /home/你/.local/share/fcitx5/rime。",
      });
      return;
    }
    if (!isAbsolutePath(path)) {
      setDirError({
        code: "userDirInvalid",
        message: `需要绝对路径：${path}`,
        hint: "v1 只接受绝对路径，不支持 ~ 缩写或相对路径。",
      });
      return;
    }

    setDirSaving(true);
    try {
      // §3.1 env_set_user_dir：真实后端会做存在/可写/含 Rime 文件特征校验
      const result = await settingsApi.setUserDir(path);
      let appliedDir = path;
      if (Array.isArray(result)) {
        // 当前 Rust 实现返回重新探测到的方案列表
        setSchemes(result);
        setSchemesError(null);
      } else if (result && typeof result === "object" && "userDir" in result) {
        // 契约写的返回值是 AppSettings，兼容处理
        adoptSettings(result);
        appliedDir = result.userDir ?? path;
      }

      // 目录已切换，同步一次设置（失败不影响切换结果本身）
      try {
        const fresh = await settingsApi.getSettings();
        adoptSettings(fresh);
        appliedDir = fresh.userDir ?? appliedDir;
      } catch {
        setSettings((previous) => (previous ? { ...previous, userDir: appliedDir } : previous));
      }

      setDirInput(appliedDir);
      setDirNotice(`已切换到 ${appliedDir}，受管文件已重新识别。`);
    } catch (error) {
      // FR-1.4：失败必须给具体原因 + 下一步（含「选错成共享目录」的 hint）
      setDirError(toAppError(error));
    } finally {
      setDirSaving(false);
    }
  }, [dirInput, adoptSettings]);

  // ───────────────────────── 恢复默认 ─────────────────────────

  const performReset = useCallback(async () => {
    if (resetBusy) return;
    setResetBusy(true);
    setResetError(null);
    try {
      const reset = await settingsApi.resetSettings();
      adoptSettings(reset);
      setDraft(draftFromSettings(reset));
      setSaveError(null);
      setResetOpen(false);
      setNotice("已恢复默认设置（词库与备份数据未受影响）");

      // 用户目录回到自动探测：输入框展示探测值，并重新识别方案
      const fallback = env?.active?.userDir ?? env?.frontends[0]?.userDir ?? "";
      setDirInput(reset.userDir ?? fallback);
      dirInitRef.current = true;
      void loadSchemes();
    } catch (error) {
      setResetError(toAppError(error));
    } finally {
      setResetBusy(false);
    }
  }, [resetBusy, env, loadSchemes, adoptSettings]);

  // ───────────────────────── 渲染 ─────────────────────────

  const detectedUserDir = env?.active?.userDir ?? env?.frontends[0]?.userDir ?? null;
  const configuredUserDir = settings?.userDir ?? null;

  return (
    <div className="settings">
      <header className="stg-header">
        <div>
          <h1>设置</h1>
          <p>Rime 环境、受管文件、部署与备份策略。设置只保存在本机，不上传任何数据。</p>
        </div>
        <div className="stg-header__badges">
          {mockMode ? (
            <Badge tone="warn" title="纯浏览器预览：数据来自 src/ipc/mock.ts">
              Mock 预览
            </Badge>
          ) : (
            <Badge tone="ok">后端已连接</Badge>
          )}
          <button
            type="button"
            className="ui-btn ui-btn--sm"
            onClick={refreshAll}
            disabled={envLoading || schemesLoading || settingsLoading}
          >
            刷新环境
          </button>
        </div>
      </header>

      {saveError ? (
        <ErrorAlert error={saveError} onRetry={() => void saveNow()} retryLabel="重试保存" />
      ) : null}

      {notice ? (
        <div className="stg-notice stg-notice--ok" role="status">
          {notice}
        </div>
      ) : null}

      {/* 1. 环境（§3.1 env_detect） */}
      <EnvironmentSection
        env={env}
        loading={envLoading}
        error={envError}
        onRetry={() => void loadEnv()}
      />

      {/* 2. 用户目录（§3.1 env_set_user_dir） */}
      <UserDirSection
        detectedUserDir={detectedUserDir}
        configuredUserDir={configuredUserDir}
        input={dirInput}
        onInputChange={(value) => {
          setDirInput(value);
          if (dirError) setDirError(null);
          if (dirNotice) setDirNotice(null);
        }}
        onApply={() => void applyUserDir()}
        saving={dirSaving}
        error={dirError}
        notice={dirNotice}
      />

      {/* 3. 方案与受管文件（§3.1 env_list_schemes） */}
      <SchemesSection
        schemes={schemes}
        loading={schemesLoading}
        error={schemesError}
        onRetry={() => void loadSchemes()}
      />

      {/* 4–7 依赖 settings_get */}
      {settingsLoading && !settings ? (
        <Section title="设置项">
          <Spinner label="正在读取设置…" />
        </Section>
      ) : null}

      {/* 刷新失败时保留上一次可用的设置界面，同时回显错误 */}
      {settingsError ? (
        <Section title="设置项">
          <ErrorAlert
            error={settingsError}
            onRetry={() => void loadSettings()}
            retryLabel="重新读取设置"
          />
        </Section>
      ) : null}

      {settings && draft ? (
        <>
          {/* 4. 保存与部署 */}
          <SchemeConfigSection schemes={schemes} />

          <SaveDeploySection autoDeploy={draft.autoDeployAfterSave} onToggle={setAutoDeploy} env={env} />

          {/* 5. 备份策略 */}
          <BackupSection key={settingsRevision} backup={draft.backup} onChange={setBackup} />

          {/* 6. 外观与语言 */}
          <AppearanceSection theme={draft.theme} language={settings.language} onThemeChange={setTheme} />

          {/* 7. 恢复默认 */}
          <ResetSection
            open={resetOpen}
            busy={resetBusy}
            error={resetError}
            onOpen={() => {
              setResetError(null);
              setResetOpen(true);
            }}
            onClose={() => setResetOpen(false)}
            onConfirm={() => void performReset()}
          />

          <p className="stg-footnote stg-page-foot">
            设置通过 <code>docs/ipc.md</code> §3.7 的 <code>settings_update</code> 部分更新保存
            （字段 camelCase）。M1 阶段后端设置为内存态，持久化插件随 T1.9 接线，重启应用后可能回到默认值。
          </p>
        </>
      ) : null}

      {/* 底部保存条：显式保存模型 */}
      {dirty ? (
        <div className="stg-savebar" role="region" aria-label="未保存的设置">
          <div className="stg-savebar__info">
            <strong>{dirtyFields.length} 项未保存</strong>
            <span className="stg-muted">
              {dirtyFields.map((field) => DIRTY_LABEL[field] ?? field).join("、")}
            </span>
          </div>
          <div className="stg-savebar__actions">
            <button type="button" className="ui-btn" onClick={discard} disabled={saving}>
              放弃
            </button>
            <button
              type="button"
              className="ui-btn ui-btn--primary"
              onClick={() => void saveNow()}
              disabled={saving}
            >
              {saving ? "保存中…" : "保存设置"}
            </button>
          </div>
        </div>
      ) : null}
    </div>
  );
}