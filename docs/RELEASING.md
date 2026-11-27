# 发布流程

本项目的版本发布是**手动触发、自动化构建**：
本地用脚本改版本并打 tag，推 tag 后由 GitHub Actions 构建产物并创建 Release。

## 一、发布前准备

1. 确认 `main` 上的改动都已合入、CI 通过
2. 在 `CHANGELOG.md` 顶部把「未发布」的内容整理成新版本小节：

```markdown
## [0.2.0] - 2026-12-01

### 新增
- …
```

> CHANGELOG 只在发布时更新，日常提交不写。

## 二、一条命令发布

```bash
./scripts/release.sh 0.2.0
```

脚本会依次完成：

| 步骤 | 说明 |
| --- | --- |
| 前置检查 | 分支为 `main`、工作区干净、与 `origin/main` 同步、tag 未占用、CHANGELOG 有该版本小节 |
| 同步版本号 | `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json` 一起改 |
| 跑测试 | `cargo test --workspace` |
| 构建打包 | 前端 + release 二进制 + deb/便携包（产物在 `dist-pkg/`） |
| 提交打 tag | 提交 `release: v0.2.0`，创建附注 tag |
| 推送 | 推 `main` 与 tag（会二次确认） |

只想先检查条件、不改任何文件：

```bash
./scripts/release.sh 0.2.0 --dry-run
```

## 三、自动化构建

推送 `v*` tag 后，`.github/workflows/release.yml` 会：

1. 安装系统依赖与工具链
2. 校验 tag 与三处版本号、CHANGELOG 小节一致（不一致直接失败）
3. 跑前端与 Rust 测试
4. 构建 release 二进制，打包 deb 与便携 tarball
5. 从 `CHANGELOG.md` 提取本版说明作为发布正文
6. 创建 GitHub Release 并附上产物

也可以在 GitHub 页面手动触发（Actions → Release → Run workflow），填入已存在的 tag。

带后缀的 tag（如 `v0.2.0-beta.1`）会自动标记为 **Pre-release**；常规版本号（如 `v0.1.0`）为正式 Release。

## 四、发布后检查

- Release 页面是否挂上了 `.deb` 与 `.tar.gz`
- 发布说明是否正常（来自 CHANGELOG 对应小节）
- 下载 deb 验证安装：`sudo dpkg -i rimetools_x.y.z_amd64.deb`

## 五、常见问题

**CI 报版本号不一致**
tag、`package.json`、`Cargo.toml`、`tauri.conf.json` 四处必须相同。用 `release.sh` 发版就不会出现。

**CI 报 CHANGELOG 缺少小节**
发布正文来自 `CHANGELOG.md` 的 `## [x.y.z]` 小节，没有就失败——避免发出空说明。

**想撤销一次发布**
删除远端 Release 与 tag，然后本地：

```bash
git tag -d v0.2.0
git push origin :refs/tags/v0.2.0
```

再把 `CHANGELOG.md` 的该小节改回「未发布」，重新走一遍流程。