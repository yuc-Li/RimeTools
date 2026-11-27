/**
 * 输入法配置章节（M2 设置增强）：
 * 快速上屏（`speller/auto_select`）、最大码长（`speller/max_code_length`）、
 * 词表挂载（主词典 `import_tables` 的启用/停用）。
 *
 * 写入位置与安全性：
 * - speller 配置写 `<方案>.custom.yaml` 的 `patch:`，不直接改上游方案文件；
 * - 词表挂载只能改主词典（Rime 没有词典补丁机制），仅切换行首 `# ` 前缀；
 * - 后端两条路径都有写前备份与指纹校验，文件被外部改动时拒绝覆盖。
 *
 * 当前只对小鹤音形等「码表类」方案实测；其他方案也能读，但保存前给出提示。
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { toAppError } from "../../ipc/client";
import type {
  AppError,
  MountToggle,
  SchemeConfigSnapshot,
  SchemeInfo,
} from "../../ipc/types";
import { settingsApi } from "./api";
import { Badge } from "../../components/Feedback";
import { Banner, Section, Switch } from "./sections";

export function SchemeConfigSection({ schemes }: { schemes: SchemeInfo[] | null }) {
  const [schemaId, setSchemaId] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<SchemeConfigSnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  // 草稿（与快照对照判断是否有改动）
  const [autoSelect, setAutoSelect] = useState(false);
  const [maxCodeLength, setMaxCodeLength] = useState<number>(4);
  const [mounts, setMounts] = useState<Record<string, boolean>>({});

  const load = useCallback(async (id: string | null) => {
    setLoading(true);
    setError(null);
    setNotice(null);
    try {
      const snap = await settingsApi.getSchemeConfig(id);
      setSnapshot(snap);
      setAutoSelect(snap.speller.autoSelect ?? false);
      setMaxCodeLength(snap.speller.maxCodeLength ?? 4);
      setMounts(Object.fromEntries(snap.mounts.map((m) => [m.name, m.enabled])));
    } catch (e) {
      setError(toAppError(e));
      setSnapshot(null);
    } finally {
      setLoading(false);
    }
  }, []);

  // 首次进入：用当前激活方案
  useEffect(() => {
    void load(null);
  }, [load]);

  const changedMounts = useMemo<MountToggle[]>(() => {
    if (!snapshot) return [];
    return snapshot.mounts
      .filter((m) => (mounts[m.name] ?? m.enabled) !== m.enabled)
      .map((m) => ({ name: m.name, enabled: mounts[m.name] ?? m.enabled }));
  }, [mounts, snapshot]);

  const spellerChanged =
    snapshot !== null &&
    ((snapshot.speller.autoSelect ?? false) !== autoSelect ||
      (snapshot.speller.maxCodeLength ?? 4) !== maxCodeLength);

  const dirty = spellerChanged || changedMounts.length > 0;

  const save = useCallback(
    async (deployAfter: boolean) => {
      if (!snapshot) return;
      setSaving(true);
      setError(null);
      setNotice(null);
      try {
        const result = await settingsApi.updateSchemeConfig({
          schemaId: snapshot.schemaId,
          ...(spellerChanged
            ? { autoSelect, maxCodeLength }
            : {}),
          ...(changedMounts.length > 0 ? { mounts: changedMounts } : {}),
          deployAfter,
        });
        setSnapshot(result.snapshot);
        setAutoSelect(result.snapshot.speller.autoSelect ?? false);
        setMaxCodeLength(result.snapshot.speller.maxCodeLength ?? 4);
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
    [autoSelect, changedMounts, maxCodeLength, snapshot, spellerChanged],
  );

  const activeScheme = schemes?.find((s) => s.schemaId === snapshot?.schemaId) ?? null;
  const supported = activeScheme ? activeScheme.translatorKind === "table" : true;

  return (
    <Section
      title="输入法配置"
      description={
        <>
          调整当前方案的输入行为：快速上屏（<code>speller/auto_select</code>）、
          最大码长（<code>speller/max_code_length</code>）、以及挂载哪些词表。改完需要重新部署。
        </>
      }
      aside={
        snapshot ? (
          <Badge tone={snapshot.spellerSource === "custom" ? "accent" : "muted"}>
            {snapshot.spellerSource === "custom" ? "已被 custom 覆盖" : "来自方案文件"}
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
          <Switch
            checked={autoSelect}
            onChange={setAutoSelect}
            label="快速上屏"
            hint={
              autoSelect
                ? "编码唯一时直接上屏，不再等选词（speller/auto_select: true）。"
                : "关闭后即使只剩一个候选也需要确认（speller/auto_select: false）。"
            }
          />

          <label className="stg-field stg-field--inline">
            <span className="stg-field__label">最大码长</span>
            <input
              className="stg-input stg-input--narrow"
              type="number"
              min={1}
              max={32}
              value={maxCodeLength}
              onChange={(e) => setMaxCodeLength(Number.parseInt(e.target.value, 10) || 1)}
            />
            <span className="stg-field__hint">
              超过这个长度不再接收编码；音形一般是 4。
            </span>
          </label>

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