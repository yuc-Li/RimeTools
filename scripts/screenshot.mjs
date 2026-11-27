/**
 * 用 headless Chrome + CDP 为 README 截图。
 *
 * 为什么用 mock 模式：截图会公开，绝不能包含用户真实词库/路径。
 * 页面在无 Tauri 宿主时会自动走 `src/ipc/mock.ts` 的合成数据。
 * 截图前会隐藏「Mock 预览」「mock 数据」「mock 调试」等仅供开发时看的标识，
 * 让文档截图保持干净。
 *
 * 用法：node scripts/screenshot.mjs   （需要先起 vite：npm run dev）
 */

import { writeFileSync, mkdirSync } from "node:fs";
import { execFileSync } from "node:child_process";

const BASE = process.env.SHOT_BASE ?? "http://127.0.0.1:1421";
const DEBUG_PORT = process.env.SHOT_PORT ?? "9223";
const OUT_DIR = "docs/screenshots";
const VIEWPORT = { width: 1400, height: 880 };
const DSF = 2; // 2 倍渲染，再降采样，文字更锐利

const TABS = [
  { id: "workbench", file: "workbench", wait: 1200 },
  {
    id: "deploy",
    file: "deploy",
    wait: 900,
    // 先跑一次部署，截图展示「成功 + 产物已确认」与历史记录，比空状态更直观
    action: `[...document.querySelectorAll('button')]
      .find(b => b.textContent?.includes('立即部署'))?.click()`,
    afterActionWait: 2400,
  },
  { id: "backup", file: "backup", wait: 900 },
];

const HIDE_MOCK_UX = `(() => {
  const hide = (sel) => document.querySelectorAll(sel).forEach(el => { el.style.display = 'none'; });
  hide('.app__banner');
  hide('.app__status');
  hide('.wb-devbar');
  // 工作台头部的「mock 数据（无 Tauri 后端）」徽标
  document.querySelectorAll('.ui-badge').forEach(el => {
    if (/mock/i.test(el.textContent || '')) el.style.display = 'none';
  });
  return true;
})()`;

function connect(wsUrl) {
  const ws = new WebSocket(wsUrl);
  let id = 0;
  const pending = new Map();
  ws.onmessage = (e) => {
    const m = JSON.parse(e.data);
    if (m.id && pending.has(m.id)) {
      pending.get(m.id)(m.result);
      pending.delete(m.id);
    }
  };
  const ready = new Promise((r) => (ws.onopen = r));
  const send = (method, params) =>
    new Promise((res) => {
      const i = ++id;
      pending.set(i, res);
      ws.send(JSON.stringify({ id: i, method, params }));
    });
  return { ws, ready, send };
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  const list = await (await fetch(`http://127.0.0.1:${DEBUG_PORT}/json`)).json();
  const page = list.find((t) => t.type === "page" && t.url.includes("1421"));
  if (!page) throw new Error("找不到调试目标页面（vite 是否已启动？）");

  const { ws, ready, send } = connect(page.webSocketDebuggerUrl);
  await ready;

  await send("Page.enable", {});
  await send("Emulation.setDeviceMetricsOverride", {
    width: VIEWPORT.width,
    height: VIEWPORT.height,
    deviceScaleFactor: DSF,
    mobile: false,
  });

  mkdirSync(OUT_DIR, { recursive: true });

  for (const tab of TABS) {
    await send("Page.navigate", { url: `${BASE}/?tab=${tab.id}` });
    await sleep(tab.wait);
    await send("Runtime.evaluate", { expression: HIDE_MOCK_UX, returnByValue: true });

    if (tab.action) {
      await send("Runtime.evaluate", { expression: tab.action, returnByValue: true });
      await sleep(tab.afterActionWait ?? 1000);
      // 部署完成后状态区会重绘，再隐藏一次（避免 mock 标识在重绘后重现）
      await send("Runtime.evaluate", { expression: HIDE_MOCK_UX, returnByValue: true });
    }
    await sleep(350);

    const shot = await send("Page.captureScreenshot", { format: "png", captureBeyondViewport: false });
    const raw = `${OUT_DIR}/${tab.file}.png`;
    writeFileSync(raw, Buffer.from(shot.data, "base64"));
    console.log(`  ${tab.file}.png  ${VIEWPORT.width * DSF}x${VIEWPORT.height * DSF}`);
  }

  ws.close();

  // 降采样到 1400 宽并转 WebP（体积小很多，GitHub 正常渲染）
  execFileSync("python3", ["scripts/optimize_screens.py"], { stdio: "inherit" });
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});