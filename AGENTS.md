# AGENTS.md

本文件面向在本仓库工作的 AI 代理，同时记录项目的 AI 辅助开发情况与资源消耗。

## 项目速览

**RimeTools** —— 跨平台 Rime 个人词库管理工具，Tauri 2 + Rust + React。
当前仅实测 Linux（fcitx5-rime），Windows 适配层已实现但未验证。

```bash
# 开发（前端热更新，需要 Vite）
./scripts/dev.sh

# 独立运行（前端打进二进制）
./scripts/dev.sh --prod

# 测试
(cd src-tauri && cargo test --workspace)   # Rust，269 个
npm run typecheck && npm test              # 前端

# 打包（deb + 便携 tarball → dist-pkg/）
./scripts/package.sh

# 发版（改版本号 + 测试 + 打包 + 打 tag + 推送）
./scripts/release.sh 0.2.0
```

代码结构见 README 的「项目结构」一节；接口契约见 `docs/ipc.md`。

## 协作约定

- **提交信息用中文**，说明动机而不只是改了什么
- **CHANGELOG 只在发版时更新**，日常提交不写
- **测试夹具是合成数据**：`tests/fixtures/` 里的用户码表由脚本生成，不要换成真实配置
- **不要读写用户真实的 Rime 目录**：测试一律用临时目录或夹具
- **版本号三处同步**：`package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`，
  用 `scripts/release.sh` 就不会漏
- 提交前跑 `cargo fmt --all`；clippy 目前有少量风格警告，不阻断
- **不要每次改动都跑全量测试**：日常用 `cargo check` / `npx tsc --noEmit` 快速验证即可，
  全量 `cargo test --workspace` 与 `npm test` 留到**准备推送或发布**时再跑

## AI 辅助开发

### 开发方式

本项目从零到 v0.1.0 由**单人 + AI 代理**完成：

- **1 个主会话**贯穿全程（需求文档 → 技术验证 → 实现 → 测试 → 打包 → 发布）
- 主会话按需派生 **9 个子代理**并行处理独立任务（模块实现、验证报告等），
  每个子代理只跑一轮，按不相交的写作用域划分，避免并发冲突
- 上下文过长时使用自动压缩（checkpoint）续接，因此单个会话可跨越两天

### 使用的模型

会话中途切换过一次模型：

| 模型 | 提供方 | 使用区间 | 承担的工作 |
| --- | --- | --- | --- |
| `qwen4-max` | qwen-token-plan-cn | 2026-11-26 14:40 ~ 15:26 | 项目起步：需求文档 v0.1–v0.3、开发计划表 |
| `deepseek-v4.1-flash-1017` | deepseek | 2026-11-26 15:26 起 | 其余全部：M0 技术验证、六个 crate、前端界面、打包、发布流水线 |

子代理与主会话使用同一模型，未单独指定其他路由。

### Token 用量

<!-- ai-usage:start -->
统计时点：**2026-11-27 11:09**（会话仍在继续，重跑本脚本可刷新）

**按模型**

| 模型 | 步数 | input | output | cacheRead | total |
| --- | ---: | ---: | ---: | ---: | ---: |
| `deepseek-v4.1-flash-1017` | 1,226 | 3,392,365 | 1,341,050 | 254,982,272 | 259,715,687 |
| `qwen4-max` | 47 | 217,484 | 94,113 | 4,723,968 | 5,035,565 |
| **合计** | **1,273** | **3,609,849** | **1,435,163** | **259,706,240** | **264,751,252** |

**按角色**

| 角色 | 会话数 | input | output | cacheRead | total |
| --- | ---: | ---: | ---: | ---: | ---: |
| 主会话 | 1 | 2,065,552 | 506,464 | 182,808,192 | 185,380,208 |
| 子代理 | 9 | 1,544,297 | 928,699 | 76,898,048 | 79,371,044 |
| 合计 | 10 | 3,609,849 | 1,435,163 | 259,706,240 | 264,751,252 |
<!-- ai-usage:end -->

### 统计口径

- **数据来源**：本地 DSH 会话日志（`~/.dsh/sessions/` 下按工作目录分目录）
  （每步请求返回的 `usage` 字段，按会话累加）
- **`input` / `output`**：实际计费的输入与输出 token
- **`cacheRead`**：命中前缀缓存的部分，单价远低于 input
- **`total` = input + output + cacheRead**，即含缓存命中的全部 token 吞吐量
- **不含**：图片/附件单独计费的部分，以及本地工具执行（不消耗 token）
- **费用未计算**：各家定价与折扣策略会变，按需自行换算

### 更新方式

数字会随会话增长。刷新上方表格（写入 `ai-usage` 标记块）：

```bash
python3 scripts/ai-usage.py --update AGENTS.md
```

只在终端查看、不改文件：

```bash
python3 scripts/ai-usage.py
```

脚本按当前仓库路径自动定位会话目录（可加 `--dir` 手动指定），
只统计工作目录为本仓库的会话。