/**
 * 反查页（M2）：只读查询其他配置文件里的词库与符号表。
 *
 * 解决的问题：
 * - 「某个词有没有打法」——按词查，看它出现在哪些词典、编码是什么；
 * - 「这个编码下有什么」——按编码查候选；
 * - 「符号怎么打」——按符号或按键查 punctuator 表。
 *
 * 约束：
 * - 全程只读，不改任何文件；索引由后端惰性构建，可手动重建；
 * - 只索引用户目录（系统预设不纳入，避免几十 MB 常驻内存）。
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { toAppError } from "../../ipc/client";
import type {
  LookupMode,
  LookupResult,
  LookupSourcesResult,
  SourceOrigin,
  SymbolHit,
} from "../../ipc/types";
import { describeSources, lookupApi } from "./api";
import "./lookup.css";

type View = "dict" | "symbol";

const ORIGIN_LABEL: Record<SourceOrigin, string> = {
  user: "用户",
  system: "系统",
};

export default function Lookup() {
  const [view, setView] = useState<View>("dict");
  const [mode, setMode] = useState<LookupMode>("text");
  const [keyword, setKeyword] = useState("");
  const [exact, setExact] = useState(false);

  const [sources, setSources] = useState<LookupSourcesResult | null>(null);
  const [result, setResult] = useState<LookupResult | null>(null);
  const [symbols, setSymbols] = useState<SymbolHit[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadSources = useCallback(async () => {
    try {
      setSources(await lookupApi.sources());
    } catch (e) {
      setError(toAppError(e).message);
    }
  }, []);

  useEffect(() => {
    void loadSources();
  }, [loadSources]);

  const runSearch = useCallback(async () => {
    const kw = keyword.trim();
    if (!kw) return;
    setBusy(true);
    setError(null);
    try {
      if (view === "dict") {
        setResult(
          await lookupApi.query({
            mode,
            keyword: kw,
            exact,
            sourceIds: [],
            limit: 200,
          }),
        );
      } else {
        setSymbols(await lookupApi.symbol(kw));
      }
    } catch (e) {
      setError(toAppError(e).message);
    } finally {
      setBusy(false);
    }
  }, [keyword, mode, exact, view]);

  const rebuild = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await lookupApi.rebuild();
      await loadSources();
      setResult(null);
      setSymbols(null);
    } catch (e) {
      setError(toAppError(e).message);
    } finally {
      setBusy(false);
    }
  }, [loadSources]);

  const stats = useMemo(() => {
    if (!sources) return null;
    const totalEntries = sources.sources.reduce((sum, s) => sum + s.entryCount, 0);
    return { files: sources.sources.length, entries: totalEntries, failed: sources.failed };
  }, [sources]);

  return (
    <div className="lookup">
      <header className="lk-header">
        <div>
          <h1 className="lk-title">反查</h1>
          <p className="lk-sub">
            只读检索其他配置文件里的词库与符号表，不改动任何文件。
            {stats ? (
              <>
                {" "}
                当前索引 {stats.files} 个文件、{stats.entries.toLocaleString()} 条词
                {stats.failed > 0 ? `（${stats.failed} 个解析失败）` : ""}。
              </>
            ) : null}
          </p>
        </div>
        <button type="button" className="ui-btn" onClick={() => void rebuild()} disabled={busy}>
          重建索引
        </button>
      </header>

      <div className="lk-tabs" role="tablist">
        <button
          type="button"
          role="tab"
          aria-selected={view === "dict"}
          className={`lk-tab ${view === "dict" ? "lk-tab--active" : ""}`}
          onClick={() => setView("dict")}
        >
          词典反查
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={view === "symbol"}
          className={`lk-tab ${view === "symbol" ? "lk-tab--active" : ""}`}
          onClick={() => setView("symbol")}
        >
          符号打法
        </button>
      </div>

      <div className="lk-toolbar">
        <input
          className="lk-input"
          type="search"
          placeholder={
            view === "dict"
              ? mode === "text"
                ? "输入词，查它的编码（如 你好）"
                : "输入编码，查对应的词（如 nihc）"
              : "输入符号或按键（如 《 或 <）"
          }
          value={keyword}
          onChange={(e) => setKeyword(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void runSearch();
          }}
        />

        {view === "dict" ? (
          <>
            <select
              className="lk-select"
              value={mode}
              onChange={(e) => setMode(e.target.value as LookupMode)}
            >
              <option value="text">按词查编码</option>
              <option value="code">按编码查词</option>
            </select>
            <label className="lk-check">
              <input
                type="checkbox"
                checked={exact}
                onChange={(e) => setExact(e.target.checked)}
              />
              完全匹配
            </label>
          </>
        ) : null}

        <button
          type="button"
          className="ui-btn ui-btn--primary"
          onClick={() => void runSearch()}
          disabled={busy || keyword.trim().length === 0}
        >
          {busy ? "查询中…" : "查询"}
        </button>
      </div>

      {error ? <div className="ui-alert">{error}</div> : null}

      {view === "dict" ? (
        <DictResults result={result} />
      ) : (
        <SymbolResults symbols={symbols} />
      )}

      {sources && sources.sources.length > 0 ? (
        <footer className="lk-footer">
          索引来源：{sources.userDir || "（未设置用户目录）"}
          <span className="lk-sources">
            {sources.sources.slice(0, 6).map((s) => (
              <span key={s.id} className="ui-badge ui-badge--muted" title={s.id}>
                {s.name} · {s.entryCount}
              </span>
            ))}
            {sources.sources.length > 6 ? ` 等 ${sources.sources.length} 个` : ""}
          </span>
        </footer>
      ) : null}
    </div>
  );
}

function DictResults({ result }: { result: LookupResult | null }) {
  if (!result) {
    return <p className="lk-hint">输入关键词后回车或点「查询」。</p>;
  }
  if (result.hits.length === 0) {
    return (
      <p className="lk-hint">
        没有命中。反查只索引用户目录下的词库文件（{result.scannedSources} 个），
        系统预设不在范围内。
      </p>
    );
  }
  return (
    <div className="lk-results">
      <div className="lk-count">
        命中 {result.total} 条
        {result.truncated ? `（仅显示前 ${result.hits.length} 条）` : ""}
      </div>
      <table className="lk-table">
        <thead>
          <tr>
            <th>词</th>
            <th>编码</th>
            <th>来源</th>
          </tr>
        </thead>
        <tbody>
          {result.hits.map((hit) => (
            <tr key={`${hit.text}\u0000${hit.code}`}>
              <td className="lk-cell-text">{hit.text}</td>
              <td>
                <code className="lk-code">{hit.code}</code>
              </td>
              <td className="lk-cell-src" title={describeSources(hit)}>
                {hit.sources.map((s, i) => (
                  <span key={`${s.id}-${s.lineNo}`} className="lk-src">
                    {i > 0 ? "、" : ""}
                    {s.name}
                    <span className="lk-src-origin">{ORIGIN_LABEL[s.origin]}</span>
                  </span>
                ))}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function SymbolResults({ symbols }: { symbols: SymbolHit[] | null }) {
  if (!symbols) {
    return <p className="lk-hint">输入符号本身（如 《）或它的按键（如 &lt;）后查询。</p>;
  }
  if (symbols.length === 0) {
    return <p className="lk-hint">符号表里没有匹配项。</p>;
  }
  return (
    <div className="lk-results">
      <div className="lk-count">命中 {symbols.length} 条</div>
      <table className="lk-table">
        <thead>
          <tr>
            <th>按键</th>
            <th>可打出的符号</th>
            <th>形态</th>
            <th>输入方式</th>
            <th>来源</th>
          </tr>
        </thead>
        <tbody>
          {symbols.map((s) => (
            <tr key={`${s.key}-${s.shape}-${s.scope}`}>
              <td>
                <code className="lk-code">{s.key}</code>
              </td>
              <td className="lk-cell-symbols">{s.symbols.join(" ")}</td>
              <td>{s.shape === "full" ? "全角" : "半角"}</td>
              <td className="lk-cell-mode">
                {s.mode === "commit"
                  ? "直接输入即出字"
                  : `输入 /${s.key} 后选候选`}
              </td>
              <td className="lk-cell-src">{s.scope}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}