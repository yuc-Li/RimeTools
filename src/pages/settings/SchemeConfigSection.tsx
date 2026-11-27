/**
 * 输入法配置章节（M2 设置增强）：表驱动的方案配置编辑。
 *
 * 配置项目录由后端 `OPTION_SPECS` 提供（字段依据 Rime 官方文档：
 * RimeWithSchemata、Schema.yaml 詳解、rime-prelude/default.yaml），
 * 前端只负责按类型渲染控件，不硬编码字段名。
 *
 * 写入位置与安全性：
 * - 所有项都写 `<方案>.custom.yaml` 的 `patch:`（Rime 官方覆盖机制），
 *   不直接改上游方案文件；「恢复默认」= 从 patch 里删除该键；
 * - 词表挂载只能改主词典（Rime 没有词典补丁机制），仅切换行首 `# ` 前缀；
 * - 两条路径都有写前备份与指纹校验，文件被外部改动时拒绝覆盖。
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { toAppError } from "../../ipc/client";
import type {
  AppError,
  MountToggle,
  OptionState,
  OptionUpdate,
  OptionValue,
  SchemeConfigSnapshot,
  SchemeInfo,
} from "../../ipc/types";
import { Badge } from "../../components/Feedback";
import { settingsApi } from "./api";
import { Banner, Section } from "./sections";

export function SchemeConfigSection({ schemes }: { schemes: SchemeInfo[] | null }) {
  const [schemaId, setSchemaId] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<SchemeConfigSnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  /** 草稿：只记录被改过的项；`null` 表示「恢复默认」 */
  const [draft, setDraft] = useState<Record<string, OptionValue | null>>({});
  const [mounts, setMounts] = useState<Record<string, boolean>>({});

  const load = useCallback(async (id: string | null) => {
    setLoading(true);
    setError(null);
    setNotice(null);
    setDraft({});
    try {
      const snap = await settingsApi.getSchemeConfig(id);
      setSnapshot(snap);
      setMounts(Object.fromEntries(snap.mounts.map((m) => [m.name, m.enabled])));
    } catch (e) {
      setError(toAppError(e));
      setSnapshot(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load(null);
  }, [load]);

  /** 当前值：草稿优先，否则用快照值 */
  const valueOf = useCallback(
    (opt: OptionState): OptionValue | null =>
      opt.key in draft ? draft[opt.key] : opt.value,
    [draft],
  );

  const optionUpdates = useMemo<OptionUpdate[]>(() => {
    if (!snapshot) return [];
    const out: OptionUpdate[] = [];
    for (const [key, v] of Object.entries(draft)) {
      const opt = snapshot.options.find((o) => o.key === key);
      if (!opt) continue;
      // 与当前生效值相同则不算改动
      if (v === opt.value) continue;
      out.push({ key, value: v });
    }
    return out;
  }, [draft, snapshot]);

  const changedMounts = useMemo<MountToggle[]>(() => {
    if (!snapshot) return [];
    return snapshot.mounts
      .filter((m) => (mounts[m.name] ?? m.enabled) !== m.enabled)
      .map((m) => ({ name: m.name, enabled: mounts[m.name] ?? m.enabled }));
  }, [mounts, snapshot]);

  const dirty = optionUpdates.length > 0 || changedMounts.length > 0;

  const setValue = (key: string, value: OptionValue | null) => {
    setDraft((prev) => ({ ...prev, [key]: value }));
  };

  const resetOption = (opt: OptionState) => {
    // 恢复默认 = 从 patch 删除该键
    setDraft((prev) => ({ ...prev, [opt.key]: null }));
  };

  const save = useCallback(
    async (deployAfter: boolean) => {
      if (!snapshot) return;
      setSaving(true);
      setError(null);
      setNotice(null);
      try {
        const result = await settingsApi.updateSchemeConfig({
          schemaId: snapshot.schemaId,
          ...(optionUpdates.length > 0 ? { options: optionUpdates } : {}),
          ...(changedMounts.length > 0 ? { mounts: changedMounts } : {}),
          deployAfter,
        });
        setSnapshot(result.snapshot);
        setDraft({});
        setMounts(Object.fromEntries(result.snapshot.mounts.map((m) => [m.name, m.enabled])));

        if (result.written.length === 0) {
          setNotice("没有需要写入的改动。");
        } else if (result.deploy) {
          setNotice(
            result.deploy.success
              ? `已写入 ${result.written.length} 个文件，并已部署成功。`
              : `已写入 ${result.written.length} 个文件，但部署失败：${
                  result.deploy.error?.message ?? "请在「部署」页查看日志"
                }`,
          );
        } else {
          setNotice("已保存。配置要重新部署后才会生效（可在「部署」页一键部署）。");
        }
      } catch (e) {
        setError(toAppError(e));
      } finally {
        setSaving(false);
      }
    },
    [changedMounts, optionUpdates, snapshot],
  );

  const activeScheme = schemes?.find((s) => s.schemaId === snapshot?.schemaId) ?? null;
  const supported = activeScheme ? activeScheme.translatorKind === "table" : true;

  // 按分组渲染
  const groups = useMemo(() => {
    if (!snapshot) return [] as { name: string; items: OptionState[] }[];
    const map = new Map<string, OptionState[]>();
    for (const opt of snapshot.options) {
      const list = map.get(opt.group) ?? [];
      list.push(opt);
      map.set(opt.group, list);
    }
    return [...map.entries()].map(([name, items]) => ({ name, items }));
  }, [snapshot]);

  const overriddenCount = snapshot?.options.filter((o) => o.overridden).length ?? 0;

  return (
    <Section
      title="输入法配置"
      description={
        <>
          调整当前方案的输入行为。字段与取值依据 Rime 官方文档；
          所有改动写进 <code>&lt;方案&gt;.custom.yaml</code> 的 <code>patch:</code>，
          方案文件升级不会被冲掉。改完需要重新部署。
        </>
      }
      aside={
        snapshot ? (
          <Badge tone={overriddenCount > 0 ? "accent" : "muted"}>
            {overriddenCount > 0 ? `已覆盖 ${overriddenCount} 项` : "全部使用方案默认值"}
          </Badge>
        ) : null
      }
    >
      <label className="stg-field stg-field--inline">
        <span className="stg-field__label">方案</span>
        <select
          className="stg-input"
          value={schemaId ?? snapshot?.schemaId ?? ""}
          disabled={loading || saving}
          onChange={(e) => {
            setSchemaId(e.target.value);
            void load(e.target.value);
          }}
        >
          {(schemes ?? []).map((s) => (
            <option key={s.schemaId} value={s.schemaId}>
              {s.name}（{s.schemaId}）
            </option>
          ))}
        </select>
      </label>

      {error ? (
        <Banner tone="danger" title="读取或保存失败">
          <p>{error.message}</p>
          {error.hint ? <p className="stg-footnote">{error.hint}</p> : null}
        </Banner>
      ) : null}

      {!supported ? (
        <Banner tone="warn" title="该方案尚未实测">
          <p>
            这里只对码表类方案（如小鹤音形）做过验证。其他方案的配置项名称可能不同，
            保存前请确认文件内容；写前仍会自动备份，可随时还原。
          </p>
        </Banner>
      ) : null}

      {loading && !snapshot ? <p className="stg-footnote">正在读取方案配置…</p> : null}

      {snapshot ? (
        <>
          {groups.map((group) => (
            <div key={group.name} className="stg-optgroup">
              <h3 className="stg-optgroup__title">{group.name}</h3>
              <div className="stg-options">
                {group.items.map((opt) => (
                  <OptionRow
                    key={opt.key}
                    opt={opt}
                    value={valueOf(opt)}
                    pending={opt.key in draft}
                    onChange={(v) => setValue(opt.key, v)}
                    onReset={() => resetOption(opt)}
                  />
                ))}
              </div>
            </div>
          ))}

          <div className="stg-mounts">
            <div className="stg-mounts__head">
              <h3>挂载的词表</h3>
              <p className="stg-footnote">
                来自主词典的 <code>import_tables</code>。取消勾选会把这行注释掉，词库本身不删除。
              </p>
            </div>

            {snapshot.mounts.length === 0 ? (
              <p className="stg-footnote">
                该方案的主词典没有声明 <code>import_tables</code>，没有可调节的挂载项。
              </p>
            ) : (
              <ul className="stg-mounts__list">
                {snapshot.mounts.map((m) => {
                  const checked = mounts[m.name] ?? m.enabled;
                  return (
                    <li key={m.name} className="stg-mounts__item">
                      <label>
                        <input
                          type="checkbox"
                          checked={checked}
                          onChange={(e) =>
                            setMounts((prev) => ({ ...prev, [m.name]: e.target.checked }))
                          }
                        />
                        <span className="stg-mounts__name">{m.name}</span>
                      </label>
                      {m.isUserTable ? <Badge tone="accent">用户码表</Badge> : null}
                      {m.entryCount != null ? (
                        <span className="stg-mounts__count">{m.entryCount.toLocaleString()} 条</span>
                      ) : null}
                      {m.path == null ? <Badge tone="warn">文件缺失</Badge> : null}
                      {m.comment ? <span className="stg-mounts__note">{m.comment}</span> : null}
                    </li>
                  );
                })}
              </ul>
            )}
          </div>

          <div className="stg-actions">
            <button
              type="button"
              className="ui-btn ui-btn--primary"
              disabled={!dirty || saving}
              onClick={() => void save(false)}
            >
              {saving ? "保存中…" : "保存配置"}
            </button>
            <button
              type="button"
              className="ui-btn"
              disabled={!dirty || saving}
              onClick={() => void save(true)}
            >
              保存并部署
            </button>
            {dirty ? <span className="stg-footnote">有未保存的改动</span> : null}
          </div>

          {notice ? (
            <Banner tone="info" title="已保存">
              <p>{notice}</p>
            </Banner>
          ) : null}

          <details className="stg-details">
            <summary>写入位置</summary>
            <ul className="stg-footnote">
              <li>
                行为配置：<code>{snapshot.customPath}</code>
                {snapshot.customExists ? "" : "（保存时创建）"}
              </li>
              <li>
                词表挂载：<code>{snapshot.dictPath ?? "（未找到主词典）"}</code>
              </li>
              <li>方案文件（只读）：<code>{snapshot.schemaPath}</code></li>
            </ul>
          </details>
        </>
      ) : null}
    </Section>
  );
}

/** 单个配置项：按类型渲染控件，并提供「恢复默认」。 */
function OptionRow({
  opt,
  value,
  pending,
  onChange,
  onReset,
}: {
  opt: OptionState;
  value: OptionValue | null;
  pending: boolean;
  onChange: (v: OptionValue | null) => void;
  onReset: () => void;
}) {
  const willReset = pending && value === null;
  const canReset = opt.overridden && !willReset;

  return (
    <div className={`stg-option ${pending ? "stg-option--pending" : ""}`}>
      <div className="stg-option__text">
        <div className="stg-option__label">
          {opt.label}
          {opt.overridden && !willReset ? <Badge tone="accent">已覆盖</Badge> : null}
          {willReset ? <Badge tone="warn">将恢复默认</Badge> : null}
        </div>
        <p className="stg-option__desc">{opt.description}</p>
        <code className="stg-option__key">{opt.key}</code>
      </div>

      <div className="stg-option__control">
        {opt.kind === "bool" ? (
          <label className="stg-option__switch">
            <input
              type="checkbox"
              checked={value === true}
              onChange={(e) => onChange(e.target.checked)}
            />
            <span>{value === true ? "开" : "关"}</span>
          </label>
        ) : null}

        {opt.kind === "int" || opt.kind === "float" ? (
          <input
            className="stg-input stg-input--narrow"
            type="number"
            min={opt.min ?? undefined}
            max={opt.max ?? undefined}
            step={opt.kind === "float" ? 0.05 : 1}
            value={typeof value === "number" ? value : ""}
            onChange={(e) => {
              const raw = e.target.value.trim();
              if (raw === "") {
                onChange(null);
                return;
              }
              const n = Number(raw);
              if (Number.isFinite(n)) onChange(n);
            }}
          />
        ) : null}

        {opt.kind === "enum" ? (
          <select
            className="stg-input"
            value={typeof value === "string" ? value : ""}
            onChange={(e) => onChange(e.target.value)}
          >
            {opt.choices.map((c) => (
              <option key={c.value} value={c.value}>
                {c.label}
              </option>
            ))}
          </select>
        ) : null}

        {opt.kind === "text" ? (
          <input
            className="stg-input"
            type="text"
            value={typeof value === "string" ? value : ""}
            placeholder={opt.default != null ? String(opt.default) : "（未设置）"}
            onChange={(e) => onChange(e.target.value)}
          />
        ) : null}

        <button
          type="button"
          className="ui-btn ui-btn--sm"
          disabled={!canReset}
          onClick={onReset}
          title="从 custom.yaml 删除该键，回到方案文件里的值"
        >
          恢复默认
        </button>
      </div>
    </div>
  );
}