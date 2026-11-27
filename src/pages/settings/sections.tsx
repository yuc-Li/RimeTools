/**
 * 设置页的展示分区（纯展示 + 少量本地交互状态，所有持久化动作由 `Settings.tsx` 负责）。
 *
 * 每个分区标题下都标注了对应的 `docs/ipc.md` 命令，便于接线时对照。
 */

import { useEffect, useState, type ReactNode } from "react";
import { Badge, ErrorAlert, Spinner } from "../../components/Feedback";
import { Modal } from "../../components/Modal";
import type {
  AppError,
  BackupSettings,
  ManagedFileRef,
  SchemeInfo,
  Theme,
} from "../../ipc/types";
import type { EnvDetectResult } from "./api";
import {
  FRONTEND_LABEL,
  FORMAT_LABEL,
  ROLE_LABEL,
  THEME_OPTIONS,
  TRANSLATOR_LABEL,
  baseName,
  columnStyle,
  deployFallbackNote,
  deployMethodLabel,
  groupManagedFiles,
  roleTone,
  roleWriteNote,
} from "./labels";

// ───────────────────────── 通用外壳 ─────────────────────────

export function Section({
  title,
  description,
  aside,
  children,
}: {
  title: string;
  description?: ReactNode;
  aside?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="stg-section">
      <header className="stg-section__head">
        <div className="stg-section__title">
          <h2>{title}</h2>
          {description ? <p>{description}</p> : null}
        </div>
        {aside ? <div className="stg-section__aside">{aside}</div> : null}
      </header>
      <div className="stg-section__body">{children}</div>
    </section>
  );
}

export function Banner({
  tone,
  title,
  children,
}: {
  tone: "warn" | "info" | "ok" | "danger";
  title?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className={`stg-banner stg-banner--${tone}`}>
      {title ? <strong className="stg-banner__title">{title}</strong> : null}
      {children ? <div className="stg-banner__body">{children}</div> : null}
    </div>
  );
}

export function Switch({
  checked,
  onChange,
  disabled,
  label,
  hint,
}: {
  checked: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
  label: string;
  hint?: ReactNode;
}) {
  return (
    <label className={`stg-switch-row${disabled ? " is-disabled" : ""}`}>
      <input
        type="checkbox"
        role="switch"
        className="stg-switch"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span className="stg-switch-row__text">
        <span className="stg-switch-row__label">{label}</span>
        {hint ? <span className="stg-switch-row__hint">{hint}</span> : null}
      </span>
    </label>
  );
}

// ───────────────────────── 环境（env_detect） ─────────────────────────

export function EnvironmentSection({
  env,
  loading,
  error,
  onRetry,
}: {
  env: EnvDetectResult | null;
  loading: boolean;
  error: AppError | null;
  onRetry: () => void;
}) {
  return (
    <Section
      title="环境"
      description={
        <>
          自动探测到的 Rime 前端、目录与部署能力（<code>docs/ipc.md</code> §3.1{" "}
          <code>env_detect</code>）。
        </>
      }
      aside={
        <button type="button" className="ui-btn ui-btn--sm" onClick={onRetry} disabled={loading}>
          {loading ? "检测中…" : "重新检测"}
        </button>
      }
    >
      {loading && !env ? (
        <Spinner label="正在检测 Rime 环境…" />
      ) : error ? (
        <ErrorAlert error={error} onRetry={onRetry} retryLabel="重新检测" />
      ) : env ? (
        <EnvironmentBody env={env} />
      ) : (
        <Banner tone="info">尚未检测到环境。</Banner>
      )}
    </Section>
  );
}

function EnvironmentBody({ env }: { env: EnvDetectResult }) {
  const active = env.active;
  const frontend = active ?? env.frontends[0] ?? null;
  const capability = frontend?.deployCapability ?? null;
  const multiple = env.frontends.length > 1;

  if (!frontend) {
    return (
      <Banner tone="warn" title="没有检测到 Rime 前端">
        请确认已安装 fcitx5-rime（Linux）或小狼毫（Windows）；也可以在下方「用户目录」里手动指定路径。
      </Banner>
    );
  }

  return (
    <div className="stg-env">
      <div className="stg-env__title">
        <strong>{frontend.displayName}</strong>
        <Badge tone="accent">{FRONTEND_LABEL[frontend.kind]}</Badge>
        {frontend.version ? <span className="stg-muted">版本 {frontend.version}</span> : null}
        {active ? <Badge tone="ok">已识别</Badge> : <Badge tone="warn">多前端未唯一确定</Badge>}
      </div>

      <dl className="stg-kv">
        <dt>librime 版本</dt>
        <dd className="stg-mono">{frontend.librimeVersion ?? "未提供"}</dd>

        <dt>探测到的用户目录</dt>
        <dd className="stg-mono stg-break">{frontend.userDir || "未提供"}</dd>

        <dt>共享目录</dt>
        <dd className="stg-mono stg-break">{frontend.sharedDir ?? "未提供"}</dd>

        <dt>部署能力</dt>
        <dd className="stg-deploy">
          {capability?.supported ? (
            <>
              <Badge tone="ok">支持自动部署</Badge>
              <span>{deployMethodLabel(capability.method)}</span>
              {capability.note ? <span className="stg-muted">{capability.note}</span> : null}
            </>
          ) : (
            <>
              <Badge tone="warn">不支持自动部署</Badge>
              <span className="stg-muted">
                {capability?.method ? `方式：${deployMethodLabel(capability.method)}` : "未提供部署方式"}
              </span>
              <span className="stg-muted">{capability?.note ?? deployFallbackNote()}</span>
            </>
          )}
        </dd>
      </dl>

      {multiple ? (
        <Banner tone="warn" title={`检测到 ${env.frontends.length} 个前端，工具不静默猜测`}>
          <p>
            多前端共存时的选择器属于 FR-1.6（P1），当前以「用户目录」指向的环境为准。候选：
          </p>
          <ul className="stg-candidates">
            {env.frontends.map((item) => (
              <li key={`${item.kind}:${item.userDir}`}>
                <Badge
                  tone={
                    active?.kind === item.kind && active?.userDir === item.userDir
                      ? "accent"
                      : "muted"
                  }
                >
                  {active?.kind === item.kind && active?.userDir === item.userDir ? "当前" : "候选"}
                </Badge>
                <span>{item.displayName}</span>
                <span className="stg-mono stg-muted">{item.userDir}</span>
              </li>
            ))}
          </ul>
        </Banner>
      ) : null}
    </div>
  );
}

// ───────────────────────── 用户目录（env_set_user_dir） ─────────────────────────

export function UserDirSection({
  detectedUserDir,
  configuredUserDir,
  input,
  onInputChange,
  onApply,
  saving,
  error,
  notice,
}: {
  detectedUserDir: string | null;
  configuredUserDir: string | null;
  input: string;
  onInputChange: (value: string) => void;
  onApply: () => void;
  saving: boolean;
  error: AppError | null;
  notice: string | null;
}) {
  const effective = configuredUserDir ?? detectedUserDir;
  // 后端 env_detect 也会把探测值写进 settings.userDir，因此不能只看「非空」判断是否手动指定；
  // 只有与自动探测结果不一致时才算用户手动覆盖。
  const normalize = (value: string) => value.replace(/[\\/]+$/, "");
  const manualOverride =
    configuredUserDir != null &&
    detectedUserDir != null &&
    normalize(configuredUserDir) !== normalize(detectedUserDir);
  const dirBadge = manualOverride
    ? { tone: "accent" as const, label: "已手动指定" }
    : detectedUserDir
      ? { tone: "muted" as const, label: "自动探测" }
      : configuredUserDir
        ? { tone: "muted" as const, label: "已设置" }
        : { tone: "muted" as const, label: "未设置" };
  return (
    <Section
      title="用户目录"
      description={
        <>
          手动指定 Rime 用户目录（<code>docs/ipc.md</code> §3.1 <code>env_set_user_dir</code>，PRD FR-1.4）。共享目录只读，选错时后端会返回具体原因。
        </>
      }
      aside={<Badge tone={dirBadge.tone}>{dirBadge.label}</Badge>}
    >
      <div className="stg-current">
        <span className="stg-current__label">当前生效</span>
        <code className="stg-mono stg-break">{effective ?? "未检测到"}</code>
      </div>

      <form
        className="stg-dir-form"
        onSubmit={(event) => {
          event.preventDefault();
          onApply();
        }}
      >
        <label className="stg-field">
          <span className="stg-field__label">手动指定路径</span>
          <div className="stg-row">
            <input
              className="stg-input stg-input--mono"
              value={input}
              onChange={(event) => onInputChange(event.target.value)}
              placeholder="/home/你/.local/share/fcitx5/rime"
              spellCheck={false}
              autoComplete="off"
              disabled={saving}
            />
            <button
              type="submit"
              className="ui-btn ui-btn--primary"
              disabled={saving || input.trim().length === 0}
            >
              {saving ? "校验中…" : "应用目录"}
            </button>
          </div>
          <span className="stg-field__hint">
            只接受绝对路径，不支持 <code>~</code> 缩写或相对路径；目录须存在、可写且包含 Rime 方案文件。
          </span>
        </label>
      </form>

      {notice ? (
        <div className="stg-notice stg-notice--ok" role="status">
          {notice}
        </div>
      ) : null}
      {error ? <ErrorAlert error={error} compact /> : null}
    </Section>
  );
}

// ───────────────────────── 方案与受管文件（env_list_schemes） ─────────────────────────

export function SchemesSection({
  schemes,
  loading,
  error,
  onRetry,
}: {
  schemes: SchemeInfo[] | null;
  loading: boolean;
  error: AppError | null;
  onRetry: () => void;
}) {
  return (
    <Section
      title="方案与受管文件"
      description={
        <>
          方案来自 <code>docs/ipc.md</code> §3.1 <code>env_list_schemes</code>；受管文件按方案自动识别（FR-3.1），只有挂了个人词条入口的方案才会被写入。
        </>
      }
      aside={
        schemes ? (
          <Badge tone="info">
            {schemes.length} 个方案 · {groupManagedFiles(schemes).length} 个写入目标
          </Badge>
        ) : null
      }
    >
      {loading && !schemes ? (
        <Spinner label="正在读取方案与受管文件…" />
      ) : error ? (
        <ErrorAlert error={error} onRetry={onRetry} retryLabel="重新读取" />
      ) : schemes && schemes.length > 0 ? (
        <div className="stg-schemes">
          <ManagedFilesOverview schemes={schemes} />
          <div className="stg-scheme-list">
            {schemes.map((scheme) => (
              <SchemeCard key={scheme.schemaId} scheme={scheme} />
            ))}
          </div>
        </div>
      ) : schemes ? (
        <Banner tone="warn" title="没有识别到方案">
          请检查用户目录下是否存在 <code>*.schema.yaml</code>，或在「用户目录」中重新指定路径。
        </Banner>
      ) : null}
    </Section>
  );
}

/** 「工具会改哪些文件」的总览：按物理路径去重，避免同一文件被多方案重复列出 */
function ManagedFilesOverview({ schemes }: { schemes: SchemeInfo[] }) {
  const groups = groupManagedFiles(schemes);

  if (groups.length === 0) {
    return (
      <Banner tone="warn" title="当前没有任何可写的个人词条文件">
        <p>
          受管方案都没有挂载 <code>custom_phrase</code> 或 <code>import_tables</code> 用户码表，工具不会写入
          任何文件（FR-3.1 ③）。需要在方案中挂载个人词条入口后重新检测；工具不会自动改方案文件。
        </p>
      </Banner>
    );
  }

  return (
    <div className="stg-overview">
      <div className="stg-overview__head">
        <strong>工具只会写入以下 {groups.length} 个文件</strong>
        <span className="stg-muted">其余方案文件与系统词典只读</span>
      </div>
      <ul className="stg-overview__list">
        {groups.map(({ file, schemes: owners }) => (
          <li key={file.path} className="stg-overview__item">
            <Badge tone={roleTone(file.role)}>{ROLE_LABEL[file.role]}</Badge>
            <code className="stg-overview__name">{baseName(file.path)}</code>
            <span className="stg-overview__cols">{columnStyle(file.columns)}</span>
            <span className="stg-overview__owners">
              属于 {owners.map((scheme) => scheme.name).join(" / ")}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function SchemeCard({ scheme }: { scheme: SchemeInfo }) {
  const [open, setOpen] = useState(scheme.managedFiles.length > 0);
  const hasFiles = scheme.managedFiles.length > 0;
  // 只展示该方案字母表里真实存在的符号键（xhup 为 ; '，flypy 为 `），避免误报
  const symbolKeys = [";", "'", "`"].filter((symbol) => scheme.alphabet.includes(symbol));

  return (
    <div className={`stg-scheme${scheme.isCurrent ? " is-current" : ""}`}>
      <button
        type="button"
        className="stg-scheme__head"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        <span className={`stg-caret${open ? " is-open" : ""}`} aria-hidden="true">
          ▸
        </span>
        <span className="stg-scheme__name">{scheme.name}</span>
        <code className="stg-scheme__id">{scheme.schemaId}</code>
        {scheme.isCurrent ? <Badge tone="accent">当前方案</Badge> : null}
        <Badge tone={scheme.isActive ? "ok" : "muted"}>
          {scheme.isActive ? "已启用" : "未在 schema_list"}
        </Badge>
        <span className="stg-scheme__kind">{TRANSLATOR_LABEL[scheme.translatorKind]}</span>
        {scheme.isDoublePinyin ? <Badge tone="info">双拼</Badge> : null}
        <span className="stg-spacer" />
        <span className={`stg-scheme__count${hasFiles ? "" : " is-empty"}`}>
          {hasFiles ? `${scheme.managedFiles.length} 个受管文件` : "未挂载个人词条入口"}
        </span>
      </button>

      {open ? (
        <div className="stg-scheme__body">
          <dl className="stg-meta">
            <dt>版本</dt>
            <dd>{scheme.version ?? "未提供"}</dd>
            <dt>字母表</dt>
            <dd>
              <code className="stg-mono stg-break">{scheme.alphabet || "未提供"}</code>
              {symbolKeys.length > 0 ? (
                <span className="stg-muted stg-inline-note">
                  含 <code>{symbolKeys.join(" ")}</code> 符号键——保存校验会按此字符集判断编码合法性。
                </span>
              ) : null}
            </dd>
            <dt>码长上限</dt>
            <dd>
              {scheme.maxCodeLength != null
                ? `${scheme.maxCodeLength} 键（超出会在保存时提示 CodeTooLong）`
                : "未限制"}
            </dd>
            <dt>翻译器</dt>
            <dd>{TRANSLATOR_LABEL[scheme.translatorKind]}</dd>
          </dl>

          {hasFiles ? (
            <div className="stg-files">
              {scheme.managedFiles.map((file) => (
                <ManagedFileCard key={file.id} file={file} />
              ))}
            </div>
          ) : (
            <Banner tone="warn" title="该方案未挂载个人词条入口">
              <p>
                工具不会写入该方案。若希望维护它的个人词条，需要在方案中挂载{" "}
                <code>custom_phrase</code> 的 <code>user_dict</code>，或在 <code>import_tables</code>{" "}
                中加入用户码表（<code>*.user.dict.yaml</code>）；工具不会自动改方案文件（FR-3.1）。
              </p>
            </Banner>
          )}
        </div>
      ) : null}
    </div>
  );
}

function ManagedFileCard({ file }: { file: ManagedFileRef }) {
  return (
    <article className="stg-file">
      <div className="stg-file__head">
        <Badge tone={roleTone(file.role)}>{ROLE_LABEL[file.role]}</Badge>
        <code className="stg-file__name">{baseName(file.path)}</code>
        <Badge tone="muted">{FORMAT_LABEL[file.format]}</Badge>
        <span className="stg-spacer" />
        <span className="stg-file__cols">{columnStyle(file.columns)}</span>
      </div>
      <code className="stg-file__path stg-break" title={file.path}>
        {file.path}
      </code>
      <p className="stg-file__note">{roleWriteNote(file)}</p>
    </article>
  );
}

// ───────────────────────── 保存与部署（settings_update） ─────────────────────────

export function SaveDeploySection({
  autoDeploy,
  onToggle,
  env,
}: {
  autoDeploy: boolean;
  onToggle: (value: boolean) => void;
  env: EnvDetectResult | null;
}) {
  const frontend = env?.active ?? env?.frontends[0] ?? null;
  const capability = frontend?.deployCapability ?? null;

  return (
    <Section
      title="保存与部署"
      description={
        <>
          写入成功后是否自动重新部署（<code>settings.autoDeployAfterSave</code>，FR-5.2）。
        </>
      }
      aside={<Badge tone={autoDeploy ? "accent" : "muted"}>{autoDeploy ? "自动部署：开" : "自动部署：关"}</Badge>}
    >
      <Switch
        checked={autoDeploy}
        onChange={onToggle}
        label="保存后自动部署"
        hint={
          autoDeploy
            ? "保存成功后会立即触发部署（trigger = afterSave），输入法较短时间后即可读到新词。"
            : "关闭后保存只落盘：需要到「部署」页手动点「立即部署」，否则输入法读不到新词。"
        }
      />

      {capability && !capability.supported ? (
        <Banner tone="warn" title="当前前端不支持自动部署">
          <p>
            {capability.note ?? deployFallbackNote()}
            {autoDeploy ? " 开关虽为开启，也不会生效；请改用「部署」页手动部署。" : ""}
          </p>
        </Banner>
      ) : null}

      <p className="stg-footnote">
        自动部署只在保存请求携带 <code>deployAfter: true</code> 且此开关开启时触发；部署结果与产物校验见「部署」页。
      </p>
    </Section>
  );
}

// ───────────────────────── 备份策略（settings_update → backup.*） ─────────────────────────

export function BackupSection({
  backup,
  onChange,
}: {
  backup: BackupSettings;
  onChange: (patch: Partial<BackupSettings>) => void;
}) {
  return (
    <Section
      title="备份策略"
      description={
        <>
          对应 <code>settings.backup.*</code>（FR-6.1 / FR-6.4）。写前备份是安全网，不会被关闭。
        </>
      }
    >
      <Switch
        checked={backup.enabled}
        onChange={(value) => onChange({ enabled: value })}
        label="自动清理旧备份"
        hint="开启后，每次写入按「保留份数」清理更旧的 .bak；关闭后不再自动清理，备份会持续累积。写入前的备份始终执行。"
      />

      <div className="stg-field-row">
        <KeepCountField
          value={backup.keepCount}
          disabled={!backup.enabled}
          onCommit={(keepCount) => onChange({ keepCount })}
        />
        <div className="stg-field stg-field--grow">
          <span className="stg-field__label">备份目录</span>
          <code className="stg-mono stg-break">{backup.dir ?? "默认：与词库文件同目录"}</code>
          <span className="stg-field__hint">目录选择与「打开备份目录」在备份页 / M2 提供，本页只读展示。</span>
        </div>
      </div>

      <div className="stg-inert-block">
        <div className="stg-inert-block__head">
          <strong>定期备份</strong>
          <Badge tone="warn">M2 支持</Badge>
        </div>
        <p className="stg-muted">
          定期备份属于 P1（FR-6.4），M2 才实现；当前不会按周期自动备份。下方配置暂不可修改，也不会生效。
        </p>
        <div className="stg-field-row">
          <Switch
            checked={backup.scheduledEnabled}
            onChange={() => undefined}
            disabled
            label="启用定期备份"
            hint="M2 支持"
          />
          <label className="stg-field">
            <span className="stg-field__label">备份间隔（小时）</span>
            <input
              className="stg-input stg-input--number"
              type="number"
              value={backup.scheduledIntervalHours}
              readOnly
              disabled
            />
          </label>
        </div>
        {backup.scheduledEnabled ? (
          <Banner tone="warn" title="当前配置为开启，但 M2 前不会执行">
            <p>该值只是被保存下来；在定期备份实现前，请用备份页的「立即备份」或依赖写前自动备份。</p>
          </Banner>
        ) : null}
      </div>
    </Section>
  );
}

function KeepCountField({
  value,
  disabled,
  onCommit,
}: {
  value: number;
  disabled: boolean;
  onCommit: (value: number) => void;
}) {
  const [text, setText] = useState(String(value));
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setText(String(value));
    setError(null);
  }, [value]);

  const isValid = (raw: string) => {
    const parsed = Number(raw);
    return Number.isInteger(parsed) && parsed >= 1 && parsed <= 999;
  };

  // 输入过程中合法值立即写入草稿（仍要显式点「保存设置」），
  // 非法/半成品文本只留在本地，失焦时提示，不污染草稿。
  const handleChange = (raw: string) => {
    setText(raw);
    setError(null);
    if (isValid(raw)) {
      const parsed = Number(raw);
      if (parsed !== value) onCommit(parsed);
    }
  };

  const commit = () => {
    if (!isValid(text)) {
      setError("保留份数必须是 1–999 的整数");
      return;
    }
    setError(null);
    const parsed = Number(text);
    if (parsed !== value) onCommit(parsed);
  };

  return (
    <label className="stg-field">
      <span className="stg-field__label">保留份数</span>
      <input
        className="stg-input stg-input--number"
        type="number"
        min={1}
        max={999}
        step={1}
        value={text}
        disabled={disabled}
        onChange={(event) => handleChange(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            commit();
          }
        }}
      />
      {error ? (
        <span className="stg-field__error">{error}</span>
      ) : (
        <span className="stg-field__hint">超出份数的旧备份在下次写入后清理（1–999）。</span>
      )}
    </label>
  );
}

// ───────────────────────── 外观（theme / language） ─────────────────────────

export function AppearanceSection({
  theme,
  language,
  onThemeChange,
}: {
  theme: Theme;
  language: string;
  onThemeChange: (theme: Theme) => void;
}) {
  return (
    <Section
      title="外观与语言"
      description={
        <>
          主题 <code>settings.theme</code>（FR-7.3）；界面语言 <code>settings.language</code>（FR-7.4）。
        </>
      }
    >
      <div className="stg-field">
        <span className="stg-field__label">主题</span>
        <div className="stg-seg" role="radiogroup" aria-label="主题">
          {THEME_OPTIONS.map((option) => (
            <button
              key={option.value}
              type="button"
              role="radio"
              aria-checked={theme === option.value}
              className={`stg-seg__item${theme === option.value ? " is-active" : ""}`}
              onClick={() => onThemeChange(option.value)}
            >
              <span className="stg-seg__label">{option.label}</span>
              <span className="stg-seg__desc">{option.description}</span>
            </button>
          ))}
        </div>
        <span className="stg-field__hint">
          主题会保存到设置；界面配色由应用外壳应用，当前版本仍需刷新/重启窗口后完全生效。
        </span>
      </div>

      <div className="stg-field">
        <span className="stg-field__label">界面语言</span>
        <div className="stg-row">
          <select className="stg-input" defaultValue="zh-CN" disabled>
            <option value="zh-CN">简体中文（zh-CN）</option>
            <option value="en-US">English（M2 支持）</option>
          </select>
          <Badge tone="warn">英文 M2 支持</Badge>
        </div>
        <span className="stg-field__hint">
          当前仅提供简体中文；英文界面（FR-7.4，P1）在 M2 提供，i18n 框架已在 P0 预留。
          {language !== "zh-CN" ? ` 已保存的语言值：${language}（尚未实现）。` : null}
        </span>
      </div>
    </Section>
  );
}

// ───────────────────────── 恢复默认（settings_reset） ─────────────────────────

export function ResetSection({
  open,
  busy,
  error,
  onOpen,
  onClose,
  onConfirm,
}: {
  open: boolean;
  busy: boolean;
  error: AppError | null;
  onOpen: () => void;
  onClose: () => void;
  onConfirm: () => void;
}) {
  return (
    <Section
      title="恢复默认"
      description={
        <>
          <code>docs/ipc.md</code> §3.7 <code>settings_reset</code>：把设置恢复为初始值。
        </>
      }
    >
      <div className="stg-danger-row">
        <div>
          <strong>恢复默认设置</strong>
          <p className="stg-muted">
            重置主题、自动部署、备份策略等配置。不会删除词库文件、个人词条或已有备份数据。
          </p>
        </div>
        <button type="button" className="ui-btn ui-btn--danger" onClick={onOpen}>
          恢复默认…
        </button>
      </div>

      <Modal
        open={open}
        title="恢复默认设置？"
        tone="danger"
        dismissOnBackdrop={false}
        onClose={busy ? () => undefined : onClose}
        width={520}
        footer={
          <>
            <button type="button" className="ui-btn" onClick={onClose} disabled={busy}>
              取消
            </button>
            <button type="button" className="ui-btn ui-btn--danger" onClick={onConfirm} disabled={busy}>
              {busy ? "恢复中…" : "确认恢复默认"}
            </button>
          </>
        }
      >
        <p className="stg-confirm-text">
          将把设置恢复为默认值：主题跟随系统、保存后自动部署开启、备份保留 30 份并自动清理。
        </p>
        <p className="stg-confirm-text">
          <strong>不会删除</strong>：词库文件、个人词条、备份数据、修改历史。
        </p>
        <p className="stg-confirm-text stg-muted">
          恢复后，用户目录会回到「自动探测」状态；如果有未保存的设置改动，也会一并丢弃。
        </p>
        {error ? <ErrorAlert error={error} compact /> : null}
      </Modal>
    </Section>
  );
}