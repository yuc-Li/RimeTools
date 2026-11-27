# 在 Windows 上开发、测试与打包

> 本文面向「换一台 Windows 机器继续开发」的场景。
> 仓库当前的完成度按平台分了两档，**先看这张表再动手**，能省掉不少困惑。

## 一、当前状态（务必先读）

| 能力 | Linux | Windows | 说明 |
| --- | --- | --- | --- |
| 编译 / 启动 | ✅ 实测 | ⚠️ 理论上可编译，**未在真机验证** | 代码里平台分支集中在 `rime-platform`，其余 crate 与平台无关 |
| 输入法探测 | ✅ fcitx5 | ⚠️ 已实现 `%APPDATA%\Rime` 探测，未实测 | 见 `rime-platform::detect_frontends` |
| 词库读写 / 导入导出 / 反查 / 方案配置 | ✅ 实测 | ⚠️ 沿用同一套代码，未实测 | 文件路径全部走 `Path` API，无硬编码分隔符 |
| **部署（推到输入法）** | ✅ fcitx5 D-Bus | ❌ **未实现** | 见下方「待补事项」第 1 条 |
| 安装包 | ✅ deb + 便携包 | ⚠️ 已配好 NSIS 目标与脚本，未实测 | 见「五、打包」 |

也就是说：**Windows 上能开发、能跑、能读写词库，但「一键部署」还不可用**，
打包配置已就绪但没在真机跑过。下面的步骤按这个前提写。

## 二、环境准备

### 1. 基础工具

| 工具 | 版本 | 备注 |
| --- | --- | --- |
| Windows | 10 1809+ / 11，64 位 | |
| **Rust** | stable | 用 [rustup](https://rustup.rs/) 安装，**必须选 MSVC 工具链** |
| **Visual Studio Build Tools** | 2022 | 勾选「使用 C++ 的桌面开发」+ Windows SDK |
| **Node.js** | 20 LTS+ | 前端构建 |
| **WebView2 Runtime** | 最新 | Win11 自带；Win10 若缺失，装 [Evergreen Bootstrapper](https://developer.microsoft.com/microsoft-edge/webview2/) |
| Git | 最新 | 建议把 `core.autocrlf` 设为 `false`（见「六、坑」） |

安装 Rust 时执行：

```powershell
rustup default stable-msvc
rustup show          # 确认 default host 是 x86_64-pc-windows-msvc
```

### 2. 测试用的输入法

装 [小狼毫 Weasel](https://rime.im/download/)（Rime 的 Windows 前端），
并至少安装一个方案（本项目的首要目标是**小鹤音形**）。

安装后确认用户目录存在：

```powershell
Test-Path "$env:APPDATA\Rime"     # 应为 True
```

### 3. 可选：打安装包才需要

```powershell
cargo install tauri-cli --version "^2"    # 提供 cargo tauri 子命令
```

## 三、拉代码与依赖

```powershell
git clone <repo-url> RimeTools
cd RimeTools

# 前端依赖（按 package-lock.json 精确安装）
npm ci

# 预热 Rust 依赖
cd src-tauri
cargo fetch
```

> 仓库根目录的 `.gitattributes` 已声明 `tests/fixtures/** -text`，
> 保证夹具里的 CRLF 不被换行转换改写。**不要删这条规则**，
> 否则字节级往返测试会失败。

## 四、开发与运行

### 方式 A：热更新开发（改前端立即生效）

开两个终端：

```powershell
# 终端 1：Vite（监听 1420）
npm run dev

# 终端 2：Tauri 开发构建（会连 127.0.0.1:1420）
.\scripts\dev.ps1
```

原理：Tauri 的 `is_dev() = !cfg!(feature="custom-protocol")`。
未启用该 feature 时程序去连 `devUrl`（1420），所以**必须先起 Vite**。

### 方式 B：独立运行（前端打进二进制，启动快）

```powershell
.\scripts\dev.ps1 -Prod
```

等价于手写命令：

```powershell
npm run build
cd src-tauri
cargo run -p rimetools --features custom-protocol
```

> 如果你习惯 Git Bash，也可以直接用 Linux 那套 `./scripts/dev.sh`。

## 五、测试

### 日常快速校验（每次改完就做）

```powershell
npx tsc --noEmit                                   # 前端类型
cd src-tauri; cargo check --workspace              # Rust 编译检查
```

### 全量测试（提交前 / 推送前）

```powershell
cd src-tauri
cargo test --workspace
```

```powershell
npm run typecheck
npm test
```

**预期结果**：Linux 上是 330 个 Rust 测试全绿。Windows 上绝大多数用例与平台无关，
但请在第一次跑完后留意这几类：

| 用例类型 | 说明 |
| --- | --- |
| `rime-deploy` 的单元测试 | 用 mock 的 D-Bus runner，应当通过 |
| `end_to_end` / `m2_*` | 只操作临时目录里的夹具，应当通过 |
| 名字带 `real_` / `live_` 的 | 标了 `#[ignore]`，需要真实环境，默认不跑 |

若有失败，先看是不是**路径分隔符**或**换行**假设导致的，再判断是不是真 bug。

### 手动运行需要真实环境的探针

```powershell
cd src-tauri

# 只读：用真实 Rime 目录跑一遍加载链路
cargo test -p rimetools --test real_load_probe -- --ignored --nocapture

# 只读：反查 + 符号表
cargo test -p rimetools --test real_load_probe real_lookup_and_symbols -- --ignored --nocapture

# 只读：方案配置补丁（会读取 xhup.custom.yaml / xhup.dict.yaml，不写入）
cargo test -p rimetools --test real_load_probe real_scheme_config -- --ignored --nocapture
```

## 六、真机验证清单（第一次在 Windows 上跑，按顺序过一遍）

- [ ] 应用能启动，托盘图标出现
- [ ] 「设置 → 环境」能识别出**小狼毫**，用户目录为 `%APPDATA%\Rime`
- [ ] 「设置 → 方案与受管文件」能列出方案与受管文件
      （双拼类应指向 `custom_phrase*.txt`；音形类指向 `*.user.dict.yaml`）
- [ ] 「词库工作台」能打开、搜索、编辑、保存；保存后备份出现在「备份」页
- [ ] 「导入导出」能导出 txt/csv/dict.yaml，再导回来（用副本测试，别拿真词库试）
- [ ] 「反查」能按方案列出词表并查到编码
- [ ] 「输入法」页能读写 `<方案>.custom.yaml`，改动落盘
- [ ] **部署**：目前按钮会显示不支持——在 Windows 上请改用托盘右键「重新部署」

## 七、打包

### 便携版（一定能跑）

```powershell
.\scripts\package-windows.ps1
```

产物在 `dist-pkg\`：

```
rimetools-<版本>-x64.zip    便携包（内含 exe + ico + README.txt）
```

### NSIS 安装包（需要 tauri-cli）

```powershell
.\scripts\package-windows.ps1 -Installer
```

等价于：

```powershell
npm run build
cd src-tauri
cargo tauri build --bundles nsis
# 产物：src-tauri\target\release\bundle\nsis\*_x64-setup.exe
```

`tauri.conf.json` 里已经配好：

```json
"bundle": {
  "icon": [..., "icons/icon.ico"],
  "targets": ["deb", "appimage", "nsis"],
  "windows": {
    "webviewInstallMode": { "type": "downloadBootstrapper" },
    "nsis": { "installMode": "currentUser", "languages": ["SimpChinese", "English"] }
  }
}
```

- `downloadBootstrapper`：目标机缺 WebView2 时，安装包会先下载安装它
- `currentUser`：装到当前用户，不要求管理员

> 图标的 `.ico` 由 `python3 scripts/gen-icons.py` 生成（内含 16/24/32/48/64/128/256 七档）。
> 改图标后重跑一次即可。

## 八、待补事项（把 Windows 做到「可用」需要做的事）

按优先级排列，建议在 Windows 机器上逐项完成：

### 1. 实现 Weasel 部署（阻塞「一键部署」）

现状：`rime-platform::detect_frontends` 已能识别小狼毫，但
`deploy_capability.supported` 被设为 `false`（诚实标记），没有任何代码调用部署器。

需要做：

- 定位 `WeaselDeployer.exe`（安装目录通常在
  `%ProgramFiles(x86)%\Rime\weasel-<版本>\`，或从注册表
  `HKCU\Software\Rime\Weasel` / `HKLM\SOFTWARE\WOW6432Node\Rime\Weasel` 取）
- 执行 `WeaselDeployer.exe /deploy`，等待进程结束
- **不要只看退出码**：沿用 Linux 侧思路，以 `%APPDATA%\Rime\build\` 里编译产物的
  mtime 是否更新来判断是否真的生效（`rime-deploy` 已有 `build_is_up_to_date` /
  `newest_mtime`，可直接复用）
- 注意部署会重启 `WeaselServer.exe`，期间输入法可能短暂不可用
- 完成后把 `detect_frontends` 里的 `supported` 改回 `true`

### 2. 打包配置在真机上验证

- 跑一次 `.\scripts\package-windows.ps1 -Installer`，确认 NSIS 产物能装、能启动
- 检查开始菜单快捷方式、卸载项、图标是否正确

### 3. CI（可选）

GitHub Actions 加一个 `windows-latest` job 跑 `cargo test --workspace` + `npm test`，
避免 Windows 侧的回归只能靠本机发现。

### 4. 其它平台相关点

- `rime-deploy` 的 D-Bus 路径（`dbus.rs`）在 Windows 上不可用，
  调用会走"探测不到 gdbus"的降级分支；确保用户看到的是清晰提示而不是超时
- 托盘菜单、单实例、开机自启（若后续要做）都需要 Windows 侧验证

## 九、常见坑

| 现象 | 原因 / 处理 |
| --- | --- |
| `cargo build` 报找不到 `link.exe` | 没装 VS Build Tools，或没用 MSVC 工具链 |
| 应用启动白屏 | 缺 WebView2 运行时，或 Vite 没起（dev 模式） |
| 夹具往返测试失败、多出 `\r` | `core.autocrlf` 把 CRLF 改写了；设 `git config core.autocrlf false` 后重新 checkout |
| 中文路径报错 | 已在用 `Path`/`PathBuf`，若仍出错请报具体路径；注意 `%APPDATA%` 可能含中文用户名 |
| 保存被拒（`externalModified`） | 小狼毫部署或其它程序改了同一个文件；重新加载后再改（这是设计行为） |
| 杀软拦截写入 | 把 Rime 用户目录加入白名单 |
| 托盘图标不显示 | Windows 会隐藏新图标，点任务栏「^」展开查看 |
| 部署后输入法没变化 | 检查 `%APPDATA%\Rime\build\` 是否有新产物；小狼毫需重启服务 |

## 十、相关文档

- [README](../README.md)：功能与项目结构
- [docs/ipc.md](ipc.md)：前后端接口契约
- [docs/RELEASING.md](RELEASING.md)：发版流程（Linux 产物）
- [AGENTS.md](../AGENTS.md)：给 AI 代理的协作约定