/**
 * 部署页核心判定的回归测试（纯函数 + mock 命令链路，不渲染 DOM）。
 *
 * 重点守住 PRD FR-5.5：`success && !outputVerified` 必须判为警告；
 * 以及 `deploy_start` 的并发合并语义（merged 不是失败）。
 */
import { describe, expect, it } from "vitest";
import { mockInvoke } from "../../ipc/mock";
import type { DeployResult, DeployState } from "../../ipc/types";
import {
  classifyDeployResult,
  deployElapsedMs,
  formatDuration,
  inferHistoryFinishedAt,
  manualDeployGuide,
  resultKey,
} from "./format";

function makeResult(patch: Partial<DeployResult> = {}): DeployResult {
  return {
    trigger: "manual",
    success: true,
    durationMs: 1200,
    outputVerified: true,
    ...patch,
  };
}

const IDLE: DeployState = { phase: "idle", startedAt: null, finishedAt: null };

describe("FR-5.5 结果判定", () => {
  it("success && !outputVerified 必须是警告，不能当成功", () => {
    const verdict = classifyDeployResult(makeResult({ outputVerified: false }));
    expect(verdict.tone).toBe("warn");
    expect(verdict.unverified).toBe(true);
    expect(verdict.title).toContain("build/");
    expect(verdict.title).toContain("改动可能未生效");
  });

  it("success && outputVerified 是成功", () => {
    const verdict = classifyDeployResult(makeResult());
    expect(verdict.tone).toBe("ok");
    expect(verdict.unverified).toBe(false);
  });

  it("失败带 error.message", () => {
    const verdict = classifyDeployResult(
      makeResult({
        success: false,
        outputVerified: false,
        error: { code: "deployTimeout", message: "部署超时", hint: "检查 fcitx5" },
      }),
    );
    expect(verdict.tone).toBe("danger");
    expect(verdict.detail).toBe("部署超时");
  });

  it("无结果时 muted", () => {
    expect(classifyDeployResult(undefined).tone).toBe("muted");
    expect(classifyDeployResult(null).tone).toBe("muted");
  });
});

describe("时间/耗时", () => {
  it("部署中按 startedAt 实时计算", () => {
    expect(deployElapsedMs({ phase: "running", startedAt: 1000, finishedAt: null }, 3500)).toBe(2500);
  });

  it("结束后用 durationMs", () => {
    expect(
      deployElapsedMs(
        {
          phase: "success",
          startedAt: 1000,
          finishedAt: 5000,
          lastResult: makeResult({ durationMs: 4000 }),
        },
        9999,
      ),
    ).toBe(4000);
  });

  it("历史时间可从日志文件名反推", () => {
    const item = makeResult({ logPath: "/home/u/.local/state/rimetools/logs/deploy-1764000000000.log" });
    expect(inferHistoryFinishedAt(item, 3, IDLE)).toBe(1764000000000);
  });

  it("最新一条与 lastResult 一致时用 finishedAt", () => {
    const item = makeResult();
    const state: DeployState = {
      phase: "success",
      startedAt: 1000,
      finishedAt: 2200,
      lastResult: item,
    };
    expect(inferHistoryFinishedAt(item, 0, state)).toBe(2200);
    expect(inferHistoryFinishedAt(item, 1, state)).toBeNull();
  });

  it("会话内观察到的时间可按结果指纹回填任意历史条目", () => {
    const item = makeResult({ trigger: "afterSave", durationMs: 900 });
    const observed = { [resultKey(item)]: 1_764_000_123_000 };
    expect(inferHistoryFinishedAt(item, 5, IDLE, observed)).toBe(1_764_000_123_000);
  });

  it("耗时格式化", () => {
    expect(formatDuration(950)).toBe("950 毫秒");
    expect(formatDuration(1200)).toBe("1.2 秒");
    expect(formatDuration(65_000)).toBe("1 分 5 秒");
    expect(formatDuration(null)).toBe("—");
  });
});

describe("手动部署指引", () => {
  it("fcitx5 给出 rime_deployer 兜底命令", () => {
    const guide = manualDeployGuide({
      kind: "fcitx5Rime",
      displayName: "fcitx5-rime",
      userDir: "/home/u/.local/share/fcitx5/rime",
      sharedDir: "/usr/share/rime-data",
      deployCapability: { supported: false, method: "none" },
    });
    expect(guide.steps.length).toBeGreaterThan(0);
    expect(guide.command).toContain("rime_deployer --build");
    expect(guide.command).toContain("/home/u/.local/share/fcitx5/rime");
  });

  it("小狼毫提示 WeaselDeployer", () => {
    const guide = manualDeployGuide({
      kind: "weasel",
      displayName: "小狼毫",
      userDir: "C:/Users/u/AppData/Roaming/Rime",
      deployCapability: { supported: false, method: "none" },
    });
    expect(guide.steps.join(" ")).toContain("WeaselDeployer");
  });
});

describe("mock 部署链路", () => {
  it("deploy_start 推进状态并写入历史；并发请求合并而非失败", async () => {
    const first = await mockInvoke<{ accepted: boolean; merged: boolean }>("deploy_start", {
      trigger: "manual",
    });
    expect(first).toEqual({ accepted: true, merged: false });

    const second = await mockInvoke<{ accepted: boolean; merged: boolean }>("deploy_start", {
      trigger: "manual",
    });
    expect(second).toEqual({ accepted: false, merged: true });

    expect(await mockInvoke<boolean>("deploy_wait", { timeoutMs: 5000 })).toBe(true);

    const history = await mockInvoke<DeployResult[]>("deploy_history", { limit: 10 });
    expect(history.length).toBeGreaterThanOrEqual(1);
    expect(history[0].trigger).toBe("manual");
    expect(history[0].success).toBe(true);
    expect(history[0].outputVerified).toBe(true);
    expect(history[0].durationMs).toBeGreaterThan(0);
  }, 15_000);
});