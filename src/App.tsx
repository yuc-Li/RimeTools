import { useCallback, useEffect, useState } from "react";
import Workbench from "./pages/workbench/Workbench";
import Deploy from "./pages/deploy/Deploy";
import Backup from "./pages/backup/Backup";
import Settings from "./pages/settings/Settings";
import Lookup from "./pages/lookup/Lookup";
import SchemeConfig from "./pages/scheme/SchemeConfig";
import { call, isTauri, onSettingsChanged } from "./ipc/client";
import type { AppSettings, Theme } from "./ipc/types";

/**
 * 应用外壳：导航 + 页面容器 + 全局主题。
 *
 * 设计要点：
 * - 事件订阅（`deploy://state` 等）都在 `src/ipc/events.ts` 的模块级单例里，
 *   页面切换不会丢订阅（契约 §6.4）。
 * - 主题由 `settings.theme` 驱动，通过 `<html data-theme>` 覆盖 CSS 变量；
 *   设置页保存后经 `onSettingsChanged()` 通知，立即生效、无需刷新。
 */

type Tab = "workbench" | "lookup" | "scheme" | "deploy" | "backup" | "settings";

const TABS: { id: Tab; label: string }[] = [
  { id: "workbench", label: "词库工作台" },
  { id: "lookup", label: "反查" },
  { id: "scheme", label: "输入法" },
  { id: "deploy", label: "部署" },
  { id: "backup", label: "备份" },
  { id: "settings", label: "设置" },
];

/** 从 `?tab=` 读取初始页（便于深链与自动化验证）；非法值回落工作台。 */
function initialTab(): Tab {
  if (typeof window === "undefined") return "workbench";
  const raw = new URLSearchParams(window.location.search).get("tab");
  return TABS.some((t) => t.id === raw) ? (raw as Tab) : "workbench";
}

export default function App() {
  const [tab, setTab] = useState<Tab>(initialTab);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const inTauri = isTauri();

  const refreshSettings = useCallback(async () => {
    if (!inTauri) return;
    try {
      setSettings(await call<AppSettings>("settings_get"));
    } catch {
      // 设置读取失败不影响使用：退回默认（跟随系统主题）
    }
  }, [inTauri]);

  // 首屏 + 设置变更时刷新
  useEffect(() => {
    void refreshSettings();
    return onSettingsChanged(() => {
      void refreshSettings();
    });
  }, [refreshSettings]);

  // 应用主题：system → 移除属性，交给 prefers-color-scheme
  useEffect(() => {
    const theme: Theme = settings?.theme ?? "system";
    const root = document.documentElement;
    if (theme === "system") {
      delete root.dataset.theme;
    } else {
      root.dataset.theme = theme;
    }
  }, [settings?.theme]);

  return (
    <div className="app">
      <nav className="app__bar" aria-label="主导航">
        <span className="app__brand">RimeTools</span>
        <div className="app__nav">
          {TABS.map((t) => (
            <button
              key={t.id}
              type="button"
              className="app__nav-btn"
              aria-current={tab === t.id ? "page" : undefined}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </div>
        <span className="app__spacer" />
        <span
          className={`app__status ${inTauri ? "app__status--ok" : "app__status--warn"}`}
          title={inTauri ? "已连接到 Rust 后端" : "纯浏览器模式：数据来自内置 mock"}
        >
          {inTauri ? "后端已连接" : "Mock 预览"}
        </span>
      </nav>

      {!inTauri && (
        <div className="app__banner">
          当前在浏览器中运行，数据为模拟数据。用 <code>./scripts/dev.sh</code> 启动可连接真实 Rime 配置。
        </div>
      )}

      <div className="app__body">
        {tab === "workbench" && <Workbench />}
        {tab === "lookup" && <Lookup />}
        {tab === "scheme" && <SchemeConfig />}
        {tab === "deploy" && <Deploy />}
        {tab === "backup" && <Backup />}
        {tab === "settings" && <Settings />}
      </div>
    </div>
  );
}