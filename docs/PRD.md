# RimeTools 产品需求文档（PRD）

| 项目 | 内容 |
| --- | --- |
| 文档版本 | v0.5（草稿，待评审） |
| 日期 | 2026-11-26 |
| 项目代号 | RimeTools（暂定名） |
| 参考产品 | [RIME 配置小工具（rime-config-tool）](https://rime.yougaoji.com/rime-config-tool-gui-setup-tool-intro/) |
| 已确认决策 | 平台：**Windows + Ubuntu（fcitx5-rime）**；范围：**词库维护为核心 + 基础配置管理**；技术栈：**Tauri 2 + Rust + Web 前端** |
| 范围调整（2026-11-26） | **划词快速采集（FR-2 整体）与跨方案导入（FR-3.19）暂缓**；编码建议与候选预览能力移入词库工作台（FR-3.23/3.24）。实现计划见项目里程碑（§7） |
| 首要适配目标 | **凇鹤拼音（[rime-crane](https://github.com/kchen0x/rime-crane)）**：小鹤双拼 `double_pinyin_flypy` + 小鹤音形 `xhup`。用户实机环境已实测，基线见 §2.4 |

> 优先级定义：**P0** = MVP 必须（M1）；**P1** = v1.0 必须（M2）；**P2** = v1.x 及以后。
> 本文档中所有外部技术结论均附来源；标注「M0 验证」的条目需在技术验证里程碑中实测确认。

---

## 1. 背景与动机

### 1.1 Rime 的价值与痛点

Rime（中州韵）是本地离线输入法引擎：轻量、无广告、词库与配置完全由用户掌控，前端覆盖 Windows（小狼毫 Weasel）、macOS（鼠须管）、Linux（ibus-rime / fcitx5-rime）、Android（同文）等。其代价是：

1. **维护门槛高**：词库与配置是 YAML / Tab 分隔文本，日常「加一个词、调一个候选顺序」需要手工编辑文件、理解 `custom.yaml` patch 机制、手动触发重新部署。
2. **缺少图形化日常维护工具**：现有工具要么平台单一，要么偏重一次性安装部署，缺少「长期维护个人词库」的跨平台 GUI。
3. **个人词库是长期资产**：专业术语、项目名称、人名地名、固定短语值得持续沉淀；商业输入法的云词库不可控，Rime 用户需要一个安全、可回滚、可迁移的个人词库工作台。

### 1.2 参考产品

功能上参考 [rime-config-tool](https://rime.yougaoji.com/rime-config-tool-gui-setup-tool-intro/)，
它把划词采集、词库编辑、候选预览、备份恢复串成闭环，验证了「个人词库工作台」这条路可行。
本项目借用这个思路，v1 只做**词库日常维护 + 基础配置**，并把平台扩到 Linux（fcitx5-rime）。

生态里另有 RimeVision（可视化配置）、WeaselSettings（小狼毫配置）、plum（方案安装）等工具，
各自解决不同环节，与本项目互补而非替代。

本项目定位：面向 Windows（小狼毫）与 Ubuntu（fcitx5-rime）的**跨平台个人 Rime 词库工作台**。

### 1.3 目标用户与画像

| 画像 | 描述 | 核心诉求 |
| --- | --- | --- |
| A. 双机开发者 | 工作用 Windows + 开发用 Ubuntu，两边都用 Rime | 两端一致的维护体验；个人词库可迁移（v1 导入导出，后续同步） |
| B. Rime 进阶用户 | 使用雾凇拼音/万象等方案，手工维护 `custom_phrase.txt` | 图形化批量维护、重码治理、备份回滚、不破坏现有配置 |
| C. Rime 新手 | 不理解 YAML 与部署机制 | 加词、调候选顺序「点几下就好」，出错可恢复 |

### 1.4 设计原则

1. **本地优先、隐私零妥协**：所有数据留在本机；无遥测；联网仅限用户显式触发的可选更新检查（P2）。
2. **Rime 原生格式是唯一事实源**：工具直接读写 Rime 兼容文件（UTF-8 无 BOM、Tab 分隔），不引入私有词库格式；工具元数据（分组、历史、索引）与词库内容分离。
3. **非破坏性**：只写受管文件；`*.custom.yaml` 只增改本工具声明的 patch 键，其余内容原样保留；系统词典只读。
4. **一切可回滚**：关键写入前自动备份；保存是事务性的（暂存 → 校验 → 原子写入 → 部署）。
5. **跨平台一致，平台差异显式降级**：同一功能在两端行为一致；平台能力受限（如 Wayland 全局热键）时明确告知并提供替代路径，不做静默失败。

### 1.5 成功指标（北极星与护栏）

| 指标 | 目标 |
| --- | --- |
| 新增到生效 | 工作台内 ≤ 3 步操作；保存后自动部署典型耗时 ≤ 10s（小词库） |
| 数据安全 | 0 起因工具导致的词库/配置丢失事故（写前备份 + 原子写 + 外部修改检测兜底） |
| 大词库体验 | 10 万词条下搜索响应 ≤ 200ms，列表滚动不卡顿 |
| 资源占用 | 空闲内存 ≤ 150MB；安装包 ≤ 25MB |
| 双端一致性 | 同一词库文件在 Win/Ubuntu 两端维护结果字节级一致（换行符策略除外，见 §6.4） |

---

## 2. 范围

### 2.1 平台范围（v1）

| 平台 | 输入法前端 | 支持级别 |
| --- | --- | --- |
| Windows 10/11 x64 | 小狼毫 Weasel（用户实机 0.17.4 / librime 1.13.1；0.15+ 兼容，M0 验证版本矩阵） | 完整支持 |
| Ubuntu 26.04 x64（**实测基线**：GNOME 50.1 / Wayland） | fcitx5-rime 5.1.13 / librime 1.16.1 | 完整支持。v1 为纯 GUI 应用（采集已暂缓），**不依赖全局热键/划词**，Wayland 限制基本无影响；portal 热键实测结论留作后续采集功能使用 |
| Ubuntu 24.04 x64（X11 / Wayland） | fcitx5-rime | X11 完整支持；Wayland 视 GNOME 版本走 portal 或降级剪贴板模式（M0 验证） |
| Ubuntu 22.04 x64 | fcitx5-rime | 尽力兼容（webkit2gtk-4.0；旧 GNOME 无 GlobalShortcuts portal → 剪贴板模式） |

**明确不在 v1 范围**：macOS（鼠须管）、Linux 其他发行版与 ibus-rime、Android/iOS/鸿蒙。架构上预留前端适配层（§6.2），后续扩展不改核心。

**开发验证策略（2026-11-26 用户决策）**：开发机为 Linux（Ubuntu 26.04 本机）。v1 的开发与测试**以 Linux 本机通过为准**；Windows 为**设计级兼容**——前端适配层隔离平台差异、代码保持可移植、打包配置预留，但不做 Win 构建门禁与真机测试，Win 实测与发布延后至有 Win 环境时。

### 2.2 v1.0 功能范围总览

| 模块 | P0（MVP） | P1（v1.0） | P2（后续） |
| --- | --- | --- | --- |
| 环境检测 | 自动识别前端/用户目录、手动指定 | 多前端共存、版本兼容提示 | ibus/macOS 适配 |
| 词库工作台 | 浏览/搜索/编辑/分组/批量、多编码、**新增词条编码建议（双拼/音形）**、导入导出、保存校验、静态排序预览 | 重码管理、修改历史、词库检查、多词典管理、**librime 实时候选预览** | 系统词典只读索引浏览（词典中心） |
| 快速采集（划词/热键/采集窗口） | — | — | **暂缓**（2026-11-26 用户决策，后续版本重新评估；需求保留于 FR-2） |
| 跨方案导入 | — | — | **暂缓**（同上，需求保留于 FR-3.19） |
| 配置管理 | — | 方案启停、常用开关（候选数/中英切换键等）、patch 安全写入 | 符号表管理、输入习惯深度项 |
| 部署 | 手动/保存后自动部署、状态与结果提示 | 部署状态中心（进度/日志/并发合并）、失败回滚 | 定时部署、用户数据同步（userdb sync） |
| 备份恢复 | 写前自动备份、手动备份、从备份恢复 | 定期备份、保留份数、恢复前差异比较 | 备份加密/异地 |
| 系统集成 | 托盘、单实例、浅色/深色主题、中文界面 | 开机自启、英文界面 | 水墨主题、应用内更新 |

### 2.3 非目标（v1 明确不做）

- 皮肤/候选窗外观编辑器（生态已有 weasel-theme-editor 等）。
- 方案/插件市场与 plum 集成。
- 云同步与多设备实时同步（v1 用导入导出满足迁移；同步方案 P2 评审）。
- 直接编辑 `*.userdb`（LevelDB 用户词库）：只读也不做，避免与前端自学习冲突；引导用户使用前端「同步用户数据」。
- 输入流记录/按键诊断类监控功能（隐私敏感，参考工具也已剥离为按需独立程序）。
- 移动端。
- **划词快速采集**（全局热键、选区/剪贴板抓取、采集窗口、Wayland 剪贴板模式）——**暂缓**（2026-11-26 用户决策），后续版本重新评估；其中「编码建议」「候选预览」能力保留并移入词库工作台（FR-3.23/3.24）。
- **跨方案导入**——**暂缓**（同上）。

### 2.4 首要适配目标：凇鹤拼音（实机实测基线，2026-11-26）

> 来源：对用户实机 `~/.local/share/fcitx5/rime` 的逐文件读取与 D-Bus 只读查询。本节是 v1 的第一验收环境。

**环境事实**

| 项 | 实测值 |
| --- | --- |
| 配置发行版 | 凇鹤拼音 [kchen0x/rime-crane](https://github.com/kchen0x/rime-crane)（雾凇 rime-ice + 小鹤双拼/音形整合，**GPL-3.0**），复制安装（非 git 仓库） |
| 启用方案 | `xhup`（小鹤音形，D-Bus `GetCurrentSchema` 实测当前方案）+ `double_pinyin_flypy`（小鹤双拼） |
| Linux 端 | Ubuntu 26.04.1 LTS，GNOME Shell 50.1，**Wayland 会话**（XWayland `DISPLAY=:0`）；fcitx5-rime 5.1.13 / librime 1.16.1 |
| Wayland 热键能力 | `org.freedesktop.portal.GlobalShortcuts` 接口**已存在**（BindShortcuts / ConfigureShortcuts / ListShortcuts）→ portal 热键路线可行（**v1 不依赖**：划词采集已暂缓，此结论供后续版本使用） |
| Windows 端 | Weasel 0.17.4 / librime 1.13.1（`weasel.custom.yaml` 实测）→ **FFI 需兼容 1.13.1 与 1.16.1 两代 librime** |
| 本机工具链 | `librime.so.1`（/usr/lib/x86_64-linux-gnu）、`rime_deployer`、`rime_dict_manager` 均在；D-Bus `org.fcitx.Fcitx5` 在线 |
| 跨设备同步 | Rime 自带同步（`sync/<installation_id>/`）覆盖 `custom_phrase*.txt`、`*.userdb.txt` 快照与方案文件，但**不覆盖 `xhup_dicts/xhup.user*`**（音形个人码表需另行迁移） |

**两个方案的个人词条入口（工具受管文件的依据）**

| 方案 | 个人词条文件 | 机制与格式 | 关键约束 |
| --- | --- | --- | --- |
| 小鹤双拼 `double_pinyin_flypy` | `custom_phrase_double.txt` | `table_translator@custom_phrase`，`user_dict: custom_phrase_double`，stabledb，`initial_quality: 99`（置顶）；`文本\t编码\t权重` | 编码必须是**小鹤双拼码**或任意自定义码（不是全拼）；`enable_completion: false`；**完整拼音码会阻断自学习造词**（文件头注释明示）；主翻译器 `speller/algebra` 的 `abbrev` 简拼被注释禁用 |
| 小鹤音形 `xhup` | `xhup_dicts/xhup.user.dict.yaml`（用户码表）+ `xhup_dicts/xhup.user.top.dict.yaml`（用户置顶码表） | 已列入 `xhup.dict.yaml` 的 `import_tables`，**零配置接入**；`文本\t编码` 两列（无权重列），YAML 头 `sort: by_weight`；置顶靠 `.top` 表先导入 | `enable_user_dict: false` → **无自学习**，个人词只能进用户码表；`max_code_length: 4`、4 码自动上屏（`auto_select`）；编码含双拼+鹤形，字母表含 `;` `'` |
| （备选）造词词典 | `cn_dicts/mydict.dict.yaml` + 在 `rime_ice.dict.yaml` 的 `import_tables` 启用 | rime-ice 官方推荐的个人词库路径，**参与造句/自学习** | 需要修改方案主词典文件的 import 行（受控定点修改，见 FR-3.20） |

**凇鹤特有的数据安全风险（工具必须兜底）**

1. **更新覆盖风险**：rime-crane README 的更新方式是「手动覆盖 `xhup_dicts` `en_dicts` `opencc` `build` 四个文件夹」——用户个人码表 `xhup.user*.dict.yaml` 恰好在被覆盖目录内，一次方案更新即可清空个人词。
2. **同步缺口**：Rime 自带 sync 不覆盖 `xhup_dicts/`，音形个人词跨设备无保护。
3. 结论：FR-6 的写前备份/外部修改检测必须覆盖上述文件，并新增「方案更新保护」（FR-6.7）；音形用户码表的导出/同步优先级提高（FR-3.13 / Q4）。

**编码建议的数据来源（无需外部数据）**

- 双拼映射：**运行时解析当前方案 `speller/algebra`**（flypy 的 `xform/derive/xlit` 规则链），对合法全拼音节表求值即得「全拼→双拼」映射；不内置 GPL 仓库内容（许可边界见 §4）。
- 音形词码：多字词**只含双拼、不含形码**（实测码表：`中国 vsgo`、`数据 uuju`、`中国人 vgr`），规则 = 二字词各字双拼全拼拼接；三字词取各字双拼首键；四字及以上取前三字+末字首键。
- 音形单字形码：从本地 `xhup_dicts/xhup.primary` / `xhup.secondary` 码表反查（字→全码，去掉双拼前缀即鹤形），运行时读取，不随工具分发。

---

## 3. 功能需求

> 每条需求含验收标准（AC）。涉及平台差异处分别给出 Win / Linux 行为。

### FR-1 环境检测与初始化（P0）

**描述**：首次启动自动发现 Rime 环境，建立「前端 → 用户目录 → 受管文件」的映射；失败时引导手动配置。

| 编号 | 需求 | 优先级 |
| --- | --- | --- |
| FR-1.1 | Windows：自动定位小狼毫安装目录与用户目录 `%APPDATA%\Rime`；安装目录探测顺序：注册表/卸载信息 → 常见安装路径（`M0 验证`） | P0 |
| FR-1.2 | Ubuntu：自动识别 fcitx5-rime（进程/D-Bus 存在性 + 用户目录 `~/.local/share/fcitx5/rime`）；共享目录默认 `/usr/share/rime-data`，可配置 | P0 |
| FR-1.3 | 检测当前会话类型（X11 / Wayland）与 fcitx5 运行状态，在界面明示能力级别（完整/降级） | P0 |
| FR-1.4 | 手动指定用户目录；校验目录有效性（存在、可写、含 Rime 文件特征），无效时给出具体原因 | P0 |
| FR-1.5 | 识别用户目录中的方案列表（`default.yaml` / `default.custom.yaml` / `build/`）与当前启用方案，展示「正在维护哪个方案的哪个词库文件」 | P0 |
| FR-1.6 | 多前端共存（如同时装有 ibus-rime）时列出候选并让用户选择，不静默猜测 | P1 |
| FR-1.7 | librime / 前端版本兼容矩阵检查，不兼容时警告并说明影响（候选预览/部署） | P1 |
| FR-1.8 | **方案结构识别**：解析当前方案 YAML，识别翻译器类型（script 拼音/双拼 vs table 音形）、`custom_phrase/user_dict` 挂载名、`speller/algebra`（双拼映射）、`import_tables` 中的用户码表（`*.user` / `*.user.top`）、`speller/alphabet` 与 `max_code_length`，据此自动确定受管文件与编码建议策略（§2.4），不硬编码方案名 | P0 |

**AC**：在干净的 Win11 + Weasel 与 Ubuntu 24.04 + fcitx5-rime 环境首启，≤ 3s 完成检测并显示正确目录；目录被移动/删除后重启能给出可操作的错误提示而非崩溃。

### FR-2 快速采集（**暂缓**，2026-11-26 用户决策）

> **本模块整体移出 v1 范围**（含全局热键、划词/剪贴板采集、采集窗口、Wayland 剪贴板模式），后续版本重新评估立项。以下需求条目保留作为未来规格；其中「编码建议」（FR-2.6）与「候选预览」（FR-2.8）能力已移入词库工作台，见 FR-3.23 / FR-3.24。v1 的新增词条路径 = 工作台「新增词条」对话框（文本输入 → 编码建议 → 权重/分组 → 保存）。

| 编号 | 需求 | 优先级 |
| --- | --- | --- |
| FR-2.1 | 全局热键（默认 `Ctrl+Alt+R`，可修改、冲突检测）唤起采集窗口。Win/X11 走系统级热键；**Wayland 走 GlobalShortcuts portal**（Rust 侧 ashpd/zbus，用户实机 GNOME 50.1 已确认接口存在）；portal 不可用时降级为应用内快捷键 + 托盘入口并明示 | P0 |
| FR-2.2 | **Win**：抓取前台窗口选中文本。实现：受控模拟 `Ctrl+C` → 读剪贴板 → **恢复原剪贴板**；失败时一次受控重试（同类工具的常见做法）；不附加目标输入线程 | P0 |
| FR-2.3 | **Linux X11**：优先直接读 PRIMARY selection（不污染剪贴板）；失败回退受控复制 | P0 |
| FR-2.4 | **Linux Wayland（用户实机即此场景，为主路径而非边缘情况）**：Wayland 下无法读取其他应用选区，采用「复制 → 热键」流：portal 热键唤起采集窗口并**预填当前剪贴板文本**；可选开启剪贴板变化监听，复制后弹轻量确认条「采集 “xxx”？」；托盘菜单提供「采集剪贴板内容」；采集窗口内始终提供手动粘贴框。界面明示当前采集模式 | P0 |
| FR-2.5 | 采集窗口显示：文本（可编辑）、编码建议列表、权重输入、分组选择、候选预览（FR-2.8）；`Enter` 保存、`Esc` 取消，全程可键盘操作 | P0 |
| FR-2.6 | **编码建议按当前方案类型生成**（FR-1.8 识别，数据源见 §2.4）：① 全拼方案→全拼、严格简拼（各音节首字母）；② **双拼方案→双拼全码**（运行时解析 `speller/algebra` 得到音节→键位映射，小鹤 flypy 为首个验证目标）与首键简码（标注「仅自定义短语可用」，因 flypy 主翻译器 abbrev 已禁用）；③ **音形方案（xhup）→词码**（二字词=各字双拼拼接；三字词=各字双拼首键；四字及以上=前三+末字首键）与单字「双拼+鹤形」建议（形码从本地主码表反查）；④ 多音字/多音词给出多个候选编码供选择；⑤ 纯英文→小写/大写/原样，英文短语去空格与连字符生成连续编码；⑥ 所有建议均可手动修改，允许任意自定义码（custom_phrase/用户码表机制支持） | P0 |
| FR-2.7 | 支持同一文本一次勾选多个编码入库（多编码）；无有效编码时禁止保存并提示 | P0 |
| FR-2.8 | **候选预览**：用隔离 librime 会话按当前方案模拟输入编码，显示前 N 项候选（N = `menu/page_size`，xhup 实测为 5）与可读权重；当前编辑文本高亮；预览只读，不影响正在使用的输入法。**音形类方案需处理 4 码自动上屏**（xhup `auto_select`）：逐键模拟并即时读取 context，或在预览会话中临时关闭 auto_select，避免候选被顶屏清空。依赖 M0 的 FFI 验证；若 FFI 不可用则降级为「按词库权重静态排序预览」并明示 | P0 |
| FR-2.9 | 保存目标：当前受管个人词库文件（按方案自动确定，见 FR-3.1；小鹤双拼为 `custom_phrase_double.txt`，音形为用户码表）；保存即触发写前备份 + 原子写 + （若开启）自动部署 | P0 |
| FR-2.10 | 采集窗口显示目标方案与目标文件路径，避免「加错方案」 | P0 |
| FR-2.11 | 紧凑简拼/混剪简拼等进阶建议、采集失败诊断信息（前台进程、失败原因，不含敏感内容） | P1 |

**AC**：
- Win：在浏览器/编辑器/资源管理器选中中英文混合文本，热键后采集窗口文本正确率 ≥ 95%（M0 实测调优），原剪贴板内容（含图片/文件列表格式）完整恢复。
- X11：同上，且不改变剪贴板。
- Wayland：剪贴板模式下从复制到出现确认条 ≤ 1s；不申请任何不可用的全局权限。
- 保存后在输入法中输入对应编码，新词出现在预期候选位置（自动部署开启时 ≤ 10s）。

### FR-3 词库工作台（P0/P1）

**描述**：个人词库的长期维护主界面。

#### 3.1 受管文件与数据边界

| 编号 | 需求 | 优先级 |
| --- | --- | --- |
| FR-3.1 | **受管文件按当前方案自动确定**（FR-1.8 识别，不硬编码方案名）：① 挂载 `custom_phrase` 的拼音/双拼方案 → 其 `user_dict` 指向的文件（小鹤双拼实测为 `custom_phrase_double.txt`，全拼为 `custom_phrase.txt`）；② 音形类 table 方案 → `import_tables` 中的用户码表（xhup 实测为 `xhup_dicts/xhup.user.dict.yaml` + `xhup.user.top.dict.yaml`，已挂载、零配置接入）；③ 方案未挂载任何个人词入口时，提示并引导（说明所需 translator/import 配置，不自动改方案文件）。界面始终显示「当前方案 → 目标文件」映射 | P0 |
| FR-3.2 | 允许用户添加其他个人词库文件为受管对象：Rime 兼容 `*.dict.yaml`（YAML 头 + `...` + Tab 数据行）与纯 txt 短语表；添加时做格式探测与校验（含 xhup 用户码表的 `#---config@` 头注释保留） | P1 |
| FR-3.3 | 系统/方案自带词典（如 `cn_dicts/*.dict.yaml`、`xhup_dicts/xhup.primary` 等主码表）**只读**，永不写入。唯一例外：`rime_ice.dict.yaml` 的 `import_tables` 中个人词典 import 行，作为受控配置点按 FR-3.20 处理 | P0（边界约束） |
| FR-3.4 | 分组是工具级元数据（存工具数据库，键 = 词典+文本+编码），不写入 Rime 文件；导出时可选附带分组列 | P0 |
| FR-3.5 | 格式与权重语义按文件类型区分并在 UI 说明：① `custom_phrase*.txt` = `文本\t编码\t权重`，权重 = 同编码内排序（越大越靠前），整体 `initial_quality: 99` 置顶；② **xhup 用户码表 = `文本\t编码` 两列（无权重列，实测格式）**，排序靠导入顺序与 `.top` 表；③ 普通 `dict.yaml` 权重 = 词频（相对值）。工具写入时保持各文件原有列风格 | P0 |

#### 3.2 浏览与维护

| 编号 | 需求 | 优先级 |
| --- | --- | --- |
| FR-3.6 | 词条列表：虚拟滚动支撑 ≥ 10 万条；列：文本/编码/权重/分组/来源文件；搜索（文本/编码，SQLite 索引）、排序、按分组与文件筛选；分组显示条目数 | P0 |
| FR-3.7 | 行内/弹窗编辑文本、编码、权重；编码输入过滤不可打印字符、中文、全角与空白，但保留标准编辑键行为 | P0 |
| FR-3.8 | 批量选择、批量删除、批量改分组、批量调权重（偏移/设值）；批量操作前展示影响摘要 | P0 |
| FR-3.9 | 多编码维护：双击词条集中管理该文本的全部编码，可从建议一键追加（同 FR-2.6） | P0 |
| FR-3.10 | 保存模型：编辑先暂存（脏标记），「保存全部」统一校验 → 写前备份 → 原子写入 → 触发部署；离开页面有未保存提示 | P0 |
| FR-3.11 | 保存校验：Tab 列数（按文件类型，xhup 用户码表两列合法）、编码字符集**以当前方案 `speller/alphabet` 为准**（xhup 含 `;` `'`）、码长提示（xhup `max_code_length: 4`，超长警告）、权重为非负整数（允许缺省）、重复条目（同文本+同编码）检测；错误逐条定位，阻止带错写入 | P0 |
| FR-3.12 | 导入：txt / csv / dict.yaml；冲突策略（跳过/覆盖/保留两者）逐项可选；导入前校验与预览 | P0 |
| FR-3.13 | 导出：当前筛选结果或全量，txt / csv（可选分组列） | P0 |
| FR-3.14 | 重码管理：按「重码编码」（同编码多文本）与「重码文本」（同文本多编码）两个视角浏览全库冲突；二级窗口集中编辑/暂存删除，统一保存 | P1 |
| FR-3.15 | 修改历史：每次写入记录快照与 diff，可回滚到任一历史版本；回滚前自动保护当前版本 | P1 |
| FR-3.16 | 词库检查：全库扫描格式问题、异常权重、疑似重复、编码与方案字符集冲突，输出报告 | P1 |
| FR-3.17 | 多词典管理：多个受管文件并行维护、跨文件移动词条 | P1 |
| FR-3.18 | 词典中心：按当前方案列出全部词典文件（名称/用途/类型），系统词典只读浏览与搜索（后台索引，不阻塞 UI） | P2 |
| FR-3.19 | 跨方案导入（如雾凇 ↔ 万象个人词库互导）：来源只读、事务快照、冲突逐项处理 | **暂缓**（2026-11-26 用户决策） |
| FR-3.20 | **造词词典路径（mydict）**：创建 `cn_dicts/mydict.dict.yaml` 并在 `rime_ice.dict.yaml` 的 `import_tables` 定点启用/停用 import 行（rime-ice 官方推荐路径）；写入前展示 diff 并经用户确认；该词典参与造句与自学习，与 custom_phrase 的「置顶不造词」互补 | P1 |
| FR-3.21 | **音形置顶操作**：xhup 场景提供「置顶/取消置顶」，实现为词条在 `xhup.user.dict.yaml` 与 `xhup.user.top.dict.yaml` 之间移动（保持两文件头注释与列格式），不引入权重列 | P0 |
| FR-3.22 | **造词阻断提示**：向 custom_phrase 类文件保存「完整拼音/双拼码」时，提示该词将不参与自学习造词（文件头注释明示的机制），建议改用 mydict（FR-3.20）或方案 `pin_cand_filter`；用户确认后仍可保存 | P0 |
| FR-3.23 | **新增词条编码建议**（自 FR-2.6 移入）：工作台「新增词条」对话框按当前方案类型生成建议——全拼/双拼（algebra 解析）/音形词码与单字形码（本地码表反查）/英文规则；多候选供选择、可手动修改、支持一次多编码入库（规则细节见 FR-2.6 与 §2.4） | P0 |
| FR-3.24 | **候选预览**（自 FR-2.8 移入）：编辑/新增词条时预览该编码的候选排序。P0 = 按词库权重与导入顺序的**静态排序预览**；P1 = **librime 隔离会话实时预览**（前 N 项，N=`menu/page_size`；只读、不影响正在使用的输入法；依赖 M0 R1 验证，失败则保持静态预览并明示）。**音形类方案须处理自动上屏**：`auto_select` 仅在「编码唯一候选且匹配 `auto_select_pattern`」时上屏（xhup 4 码多为唯一，故顶屏常见）；解法 = 隔离副本写 `<schema>.custom.yaml` 关闭 `auto_select` + 逐键 `GetCommit`/`GetContext` 兜底回退 | P0（静态）/ P1（实时） |

**AC**：10 万条 `custom_phrase.txt` 打开 ≤ 2s、搜索 ≤ 200ms；批量删除 1000 条 → 保存 → 部署后输入法侧生效；任何写入失败后原文件完整无损（备份可恢复）。

### FR-4 基础配置管理（P1）

**描述**：覆盖高频配置项，全部通过 `*.custom.yaml` patch 机制实现，**绝不直接改方案原文件**。

| 编号 | 需求 | 优先级 |
| --- | --- | --- |
| FR-4.1 | 方案启停：编辑 `default.custom.yaml` 的 `patch/schema_list`，以列表勾选方式管理；保留用户已有 patch 的其他键与注释（写入策略见 §6.3） | P1 |
| FR-4.2 | 常用开关（表单化，写入对应 patch 键）：候选数 `menu/page_size`；中英切换键（Caps Lock / Shift / Ctrl+Space 等 `ascii_composer/switch_key` 预设）；默认中英文状态；简繁切换开关 | P1 |
| FR-4.3 | 每项配置显示「当前生效值 / 来源文件（原值 or patch）」，保存后状态明确区分「已保存未部署 / 已生效」 | P1 |
| FR-4.4 | YAML 写入前语法校验 + 写入后 `rime_deployer` 试构建校验（可选），失败自动回滚到写前备份 | P1 |
| FR-4.5 | 提供「技术详情」展开：显示实际写入的 patch 片段与文件路径，满足进阶用户审查需求 | P1 |
| FR-4.6 | 符号表（`symbols_v.yaml` 等）管理、输入习惯深度项 | P2 |

**AC**：对含有用户手写 patch 与注释的 `default.custom.yaml` 执行方案启停后，除目标键外内容（含注释）逐字节保留；部署后输入法方案菜单与设置一致。

### FR-5 部署与状态（P0/P1）

**描述**：把「重新部署」变成可靠、可见、可恢复的一等操作。

| 编号 | 需求 | 优先级 |
| --- | --- | --- |
| FR-5.1 | 手动「立即部署」按钮（主界面 + 托盘） | P0 |
| FR-5.2 | 「保存后自动部署」开关（默认开）；关闭时保存后提示「已保存，尚未部署」 | P0 |
| FR-5.3 | **Win 部署机制**：调用小狼毫 `WeaselDeployer.exe /deploy`（安装目录内，M0 验证各版本行为）；不重启 WeaselServer | P0 |
| FR-5.4 | **Linux 部署机制**（按优先级）：① D-Bus `org.fcitx.Fcitx5 /controller org.fcitx.Fcitx.Controller1.SetConfig("fcitx://config/addon/rime/deploy", "")`（[来源](https://levinion.github.io/posts/Linux/%E5%88%A9%E7%94%A8pacman%20hook%E8%87%AA%E5%8A%A8%E9%83%A8%E7%BD%B2rime-wanxiang/)、[fcitx5-rime#28](https://github.com/fcitx/fcitx5-rime/issues/28)）；② `ReloadAddonConfig("rime")`；③ 兜底：`rime_deployer --build <user_dir> /usr/share/rime-data <user_dir>/build` 预构建 + 提示用户重启 fcitx5。**明确不使用 `fcitx5-remote -r`**（实测不会让 Rime 重读 build/，[来源](https://rainboylvx.github.io/blog/fcitx5-rime-ice-easy-en-capslock.html)） | P0 |
| FR-5.5 | 部署结果判定：以 `build/` 产物时间戳/内容变化 + 前端状态为准，不以命令退出码 alone 判定成功；超时与失败给出原因和下一步建议。状态查询可用 D-Bus `org.fcitx.Fcitx.Rime1.GetCurrentSchema`（**已在用户实机只读验证可用**，返回当前方案 `xhup`） | P0 |
| FR-5.6 | 部署状态中心：当前任务进度、最近 N 次结果与日志入口；并发部署请求合并为至多一次补跑；部署期间 UI 不阻塞 | P1 |
| FR-5.7 | 部署失败回滚：若本次部署由工具写入触发且失败，提示并可一键恢复写前备份后重新部署 | P1 |
| FR-5.8 | 大词库部署耗时预估与「输入可能短暂受影响」提示 | P1 |
| FR-5.9 | 用户数据同步（userdb sync，触发前端同步机制）与定时部署 | P2 |

**AC**：两端修改词条保存后，无需用户手工操作即可在输入法中生效；杀掉 fcitx5 / WeaselServer 后点部署，得到明确错误与恢复指引，不静默失败。

### FR-6 备份与恢复（P0/P1）

| 编号 | 需求 | 优先级 |
| --- | --- | --- |
| FR-6.1 | 关键写入前自动备份受管文件（命名 `原文件名_日期时间.bak`），备份目录可配置、可一键打开 | P0 |
| FR-6.2 | 手动「立即备份」（可含全部受管文件 + 相关 custom.yaml） | P0 |
| FR-6.3 | 备份列表 + 从任一备份恢复；恢复前自动备份当前版本 | P0 |
| FR-6.4 | 定期备份（周期可配）、保留份数、自动清理旧备份 | P1 |
| FR-6.5 | 恢复前差异比较：展示当前文件与所选备份的增删改摘要 | P1 |
| FR-6.6 | 备份范围扩展到输入方案/符号表等用户选择文件 | P1 |
| FR-6.7 | **方案更新保护（凇鹤特有风险，§2.4）**：`xhup_dicts/xhup.user*.dict.yaml` 位于 rime-crane 更新时被整体覆盖的目录，且不在 Rime sync 范围。工具须：① 默认将其纳入写前备份与定期备份；② 外部修改检测发现内容骤减/被替换时告警并提供「从备份恢复 / 合并差异」；③ 提供一键导出音形用户码表（跨设备迁移兜底） | P1（①为 P0） |

**AC**：连续保存 100 次不产生数据丢失；磁盘满/权限错误时备份失败会阻止写入并告警；恢复操作全程可再次回滚。

### FR-7 系统集成（P0/P1/P2）

| 编号 | 需求 | 优先级 |
| --- | --- | --- |
| FR-7.1 | 系统托盘常驻：显示主窗口、立即部署、退出；关闭窗口默认最小化到托盘（可配置为退出）。（「采集剪贴板」菜单项随 FR-2 暂缓） | P0 |
| FR-7.2 | 单实例保护：重复启动时恢复已运行窗口（Tauri single-instance） | P0 |
| FR-7.3 | 主题：浅色/深色，默认跟随系统；水墨主题 P2 | P0 |
| FR-7.4 | 界面语言：简体中文 P0；英文 P1（i18n 框架自 P0 预留） | P0 |
| FR-7.5 | 开机自启（Win 启动项 / Linux XDG autostart），显示真实系统状态（含被系统禁用的情形），失败不重复触发 | P1 |
| FR-7.6 | 应用内更新（检查/下载/校验/替换/回滚，参考工具 v0.10.1 的交接确认经验）；仅用户显式检查，不后台联网 | P2 |
| FR-7.7 | 便携模式：配置与数据可随程序目录携带（Win 便携版 / Linux AppImage） | P2 |

### FR-8 设置（P0）

集中管理：用户目录与前端选择、受管文件列表、热键、采集行为（重试/剪贴板恢复）、自动部署开关、备份策略、主题/语言、日志级别与日志目录打开入口。所有设置本地持久化，损坏时可重置为默认并保留词库数据。

---

## 4. 非功能需求

| 类别 | 要求 |
| --- | --- |
| 性能 | 冷启动 ≤ 2s（SSD）；10 万词条搜索 ≤ 200ms；列表虚拟滚动 ≥ 55fps；索引构建后台执行、可中断，构建期间沿用旧索引 |
| 资源 | 空闲内存 ≤ 150MB；安装包 Win ≤ 25MB、Linux deb/AppImage ≤ 30MB；不常驻后台扫描 |
| 可靠性 | 词库写入 = 临时文件 + fsync + 原子 rename；写前备份强制；外部修改检测（mtime + hash）冲突时提示合并而非覆盖；崩溃后重启可恢复未保存暂存 |
| 安全与隐私 | 全本地运行；无遥测、无默认联网；剪贴板读取仅在采集动作内发生且用后恢复；日志不记录词库正文与剪贴板历史；诊断信息最小化 |
| 兼容性 | Win10 22H2 / Win11；Ubuntu 26.04 / 24.04（webkit2gtk-4.1）为主构建目标，22.04（webkit2gtk-4.0）尽力兼容；librime 双版本矩阵 1.13.1（Weasel 0.17.4）/ 1.16.1（fcitx5-rime 5.1.13）见 §6.5；高 DPI 与 125%/150% 缩放正常 |
| 可用性 | 核心路径全键盘可达；危险操作（批量删除/恢复/回滚）有摘要确认；错误信息包含原因 + 建议动作；首次启动有 ≤ 5 步引导 |
| 可维护性 | Rust 核心与 UI 分层；平台适配层隔离（§6.2）；核心逻辑（解析/校验/编码建议/patch 写入）单元测试覆盖 ≥ 80%；双平台 CI |
| 许可 | 项目许可证待定（建议 MIT/Apache-2.0）；注意 librime（BSD）、Tauri（MIT/Apache）及拼音数据源许可兼容。**凇鹤 rime-crane 为 GPL-3.0**：工具只在运行时解析用户本地方案/码表文件，不随工具分发其内容（含双拼映射与鹤形码表），避免许可传染；小鹤官方码表同理仅运行时反查 |

---

## 5. 关键用户旅程（验收场景）

**J1 双机开发者加词（核心旅程，按用户实机凇鹤环境；v1 走工作台路径）**
1. Ubuntu 26.04 打开词库工作台 →「新增词条」输入「中州韵」→ 当前方案为小鹤双拼时显示双拼全码 `vsvzyy`（中 vs / 州 vz / 韵 yy）与首键简码 `vvy` 建议、候选排序预览；
2. 勾选编码、选分组「项目」、权重 100 → Enter 保存到 `custom_phrase_double.txt` → 自动部署 → 输入法输入 `vsvzyy` 出现「中州韵」；
3. 切到 xhup（小鹤音形）方案新增同一词：建议词码 `vvy`（三字词取各字双拼首键），保存到 `xhup_dicts/xhup.user.dict.yaml`（两列格式），可一键置顶（移入 `.top` 表）；
4. 周末在 Windows（Weasel 0.17.4）用同一工具打开 `%APPDATA%\Rime`：双拼短语经 Rime sync 已同步；音形用户码表用工具导出/导入迁移（不在 sync 范围），两端候选一致。

**J2 新手调候选顺序**：词库工作台搜索某词 → 调高权重 → 保存 → 预览确认候选位置变化 → 完成，全程无需理解 YAML。

**J3 误操作恢复**：批量删除误删 500 条 → 修改历史/备份恢复 → 恢复前差异确认 → 恢复并重新部署 → 词条回归。

**J4 Wayland 降级采集**（**暂缓**，随 FR-2 移出 v1）：Ubuntu Wayland 会话复制一个词 → 确认条弹出 → 点击采集 → 正常入库；界面明示「Wayland 会话：使用剪贴板采集模式」。

**J5 配置修改不破坏手写 patch**：用户 `default.custom.yaml` 含手写注释与多个 patch 键 → 工具内关闭某方案 → 部署 → 仅 `schema_list` 变化，其余逐字节保留。

**J6 凇鹤方案更新保护**：用户按 rime-crane README 覆盖 `xhup_dicts/` 更新方案 → 工具检测到 `xhup.user.dict.yaml` 内容骤减（外部修改检测）→ 告警并展示差异 → 从写前备份恢复个人词条（或合并）→ 重新部署生效。

---

## 6. 技术方案与约束

### 6.1 总体架构（已确认：Tauri 2 + Rust + Web 前端）

```
┌────────────────────────────────────────────┐
│  Web UI（React + TypeScript + Vite）        │
│  词库工作台 / 采集窗口 / 配置 / 部署 / 设置   │
└──────────────┬─────────────────────────────┘
               │ Tauri IPC（command / event）
┌──────────────┴─────────────────────────────┐
│  Rust 核心（crate: rime-tools-core）        │
│  · 词库解析/校验/序列化（txt / dict.yaml）   │
│  · 编码建议（拼音数据）                      │
│  · patch 安全写入（YAML 保留注释）           │
│  · 备份/历史/索引（SQLite）                  │
│  · 部署编排（状态机、并发合并）               │
├────────────────────────────────────────────┤
│  平台适配层（trait FrontendAdapter）         │
│  · weasel.rs：目录/注册表探测、Deployer 调用 │
│  · fcitx5.rs：D-Bus 部署、会话类型检测       │
│  · capture：热键/选区/剪贴板（按平台实现）    │
│  · librime FFI：隔离会话候选预览             │
└────────────────────────────────────────────┘
```

- Tauri 插件：`global-shortcut`、`clipboard-manager`、`single-instance`、`autostart`、`dialog`、`store`、`log`、`updater`（P2）；托盘用 Tauri 2 内置 tray。
- 前端状态：轻量方案（Zustand/TanStack Query 级别），列表用虚拟滚动组件。
- **librime FFI**：候选预览用隔离会话（`RimeInitialize` / `RimeCreateSession` / `RimeSimulateKeySequence` / `RimeGetContext`）。librime 动态库来源：Win 优先加载 Weasel 安装目录内 `rime.dll`（实机 1.13.1）；Linux 加载系统 `librime.so.1`（实机 1.16.1，路径已确认）——**需兼容两代 ABI**。绑定可用现成 crate（如 `librime-sys`）或自维护 bindgen，**M0 验证**；失败降级为静态权重预览（FR-2.8）。
- **Wayland 全局热键**（**暂缓**，随 FR-2 移出 v1，架构预留）：Tauri global-shortcut 插件不支持 Wayland，改用 xdg-desktop-portal GlobalShortcuts（Rust 侧 [ashpd](https://docs.rs/ashpd) / zbus）；用户实机 GNOME 50.1 已确认接口存在，供后续采集功能使用。
- **编码建议引擎**：内置小型 algebra 求值器（`xform/derive/xlit/erase/abbrev`），对合法全拼音节表求值得到双拼映射（运行时解析用户方案，不内置 GPL 内容）；音形词码按 §2.4 规则生成，单字形码从本地主码表反查。

### 6.2 平台差异与适配矩阵

| 能力 | Windows（Weasel） | Ubuntu X11（fcitx5-rime） | Ubuntu Wayland |
| --- | --- | --- | --- |
| 用户目录 | `%APPDATA%\Rime` | `~/.local/share/fcitx5/rime` | 同左 |
| 共享目录 | Weasel 安装目录 `data` | `/usr/share/rime-data` | 同左 |
| 全局热键（**v1 暂缓**） | Win32 RegisterHotKey（Tauri 插件） | XGrabKey（Tauri 插件） | **GlobalShortcuts portal**（GNOME 50.1 接口已确认存在；ashpd 实现）；不可用 → 应用内快捷键 + 托盘 |
| 选中文本获取（**v1 暂缓**） | 受控 Ctrl+C + 剪贴板恢复（重试 1 次） | PRIMARY selection 直读，回退受控复制 | **不可用**→ 剪贴板采集模式 |
| 重新部署 | `WeaselDeployer.exe /deploy` | D-Bus `SetConfig("fcitx://config/addon/rime/deploy","")`；兜底 `rime_deployer --build` + 重启提示 | 同左（D-Bus 与显示协议无关） |
| 部署状态查询 | 进程/build 产物/日志（M0 验证日志路径） | D-Bus `org.fcitx.Fcitx.Rime1`（`GetCurrentSchema` 等）+ build 产物 | 同左 |
| 托盘/自启 | 原生托盘 / 启动项 | 原生托盘（StatusNotifier）/ XDG autostart | 同左 |

> 来源：fcitx5 部署与状态接口见 [实测记录](https://rainboylvx.github.io/blog/fcitx5-rime-ice-easy-en-capslock.html)、[pacman hook 方案](https://levinion.github.io/posts/Linux/%E5%88%A9%E7%94%A8pacman%20hook%E8%87%AA%E5%8A%A8%E9%83%A8%E7%BD%B2rime-wanxiang/)、[fcitx5-rime#54](https://github.com/fcitx/fcitx5-rime/issues/54)。`fcitx5-remote -r` 只重读 fcitx5 全局配置、**不会**让 Rime 重载 build/，已列为禁用路径。D-Bus 服务与 `GetCurrentSchema` 已在用户实机（Ubuntu 26.04 / fcitx5-rime 5.1.13）只读验证在线。

### 6.3 数据设计

| 数据 | 存储 | 说明 |
| --- | --- | --- |
| 词条内容 | **Rime 原生文件**（唯一事实源） | `custom_phrase*.txt`、xhup 用户码表、受管 `*.dict.yaml`；UTF-8 无 BOM、Tab 分隔 |
| 分组/筛选预设 | 工具 SQLite | 键 = 词典+文本+编码；不污染 Rime 文件 |
| 搜索索引 | SQLite（按词典分表/分片） | 后台构建、原子切换；参考工具第三代分片索引经验 |
| 修改历史/暂存 | SQLite + 快照文件 | 写前快照、diff、回滚 |
| 备份 | 文件系统（`*.bak`） | 命名兼容参考工具；保留策略可配 |
| 设置 | Tauri store（JSON） | 损坏可重置 |

**写入安全协议**（所有受管文件写入统一走此流程）：
`读当前文件 → hash 比对（检测外部修改）→ 暂存校验 → 写前备份 → 临时文件写入 + fsync → 原子 rename → 更新索引/历史 → 触发部署`。

**YAML patch 写入**：使用保留注释与键序的 YAML 编辑策略（如基于事件/行级的定点修改，而非整树重序列化）；仅增改本工具声明的键；无法安全保留时明示用户并给出 diff 确认。

**外部修改冲突**：检测到文件被外部（用户手工/其他工具/方案更新）修改时，展示差异并让用户选择「以外部为准重新加载 / 以工具暂存为准覆盖（先备份）」，禁止静默覆盖。

### 6.4 文件格式约定

- `custom_phrase*.txt`（含双拼用 `custom_phrase_double.txt`）：`文本\t编码\t权重`，UTF-8 无 BOM；注释行 `#` 保留（注意「此行之后不能写注释」约束，数据区不得插入注释）；权重为非负整数，越大越靠前。
- **xhup 用户码表**（`xhup_dicts/xhup.user*.dict.yaml`）：`文本\t编码` 两列（实测无权重列）；YAML 头（`name/version/sort/use_preset_vocabulary`）与 `#---config@码表分类/别名` 头注释原样保留；置顶语义由 `.top` 表承担（FR-3.21）。
- `*.dict.yaml`：YAML 头（`name/version/sort/...`，可含 `import_tables`）+ `...` 结束符 + Tab 数据行；工具只改数据行与必要头部字段，头部其余内容保留。
- 换行符：保留原文件风格（CRLF/LF）；新建文件 Win 用 CRLF、Linux 用 LF（可配置统一）。
- 编码字符集：以当前方案 `speller/alphabet` 为准（xhup 含 `;` `'`）；码长约束按方案（xhup `max_code_length: 4`）；custom_phrase/用户码表允许任意自定义码。

### 6.5 技术风险与 M0 验证清单

| # | 风险/待验证项 | 验证方法 | 失败降级 |
| --- | --- | --- | --- |
| R1 | librime FFI 候选预览：Win `rime.dll` 1.13.1（Weasel 0.17.4）与 Ubuntu `librime.so.1` 1.16.1 的加载与 ABI 兼容；xhup 4 码 auto_select 的预览处理 | 最小 Rust demo：初始化 → 建会话 → 逐键模拟 → 取候选（含 xhup 方案） | **✅ Linux 已验证通过**（M0 报告 §2.1/2.2）；Win 仍为设计级 |
| R2 | Weasel 安装目录/注册表探测与 `WeaselDeployer.exe /deploy` 在 0.15/0.16+ 的行为 | 实机多版本测试 | 引导手动指定路径 |
| R3 | fcitx5 D-Bus 部署在 Ubuntu 22.04/24.04 的可用性与权限 | `busctl --user call ...` 实测 + 产物时间戳判定 | `rime_deployer --build` + 重启提示 |
| R4 | （**暂缓**，随 FR-2 移出 M0）Wayland 热键：portal 接口已在用户实机 GNOME 50.1 确认存在，但 BindShortcuts 授权流程、重启持久化、与 fcitx5 共存行为未验证；旧 GNOME（24.04/22.04）覆盖未知 | 后续采集功能立项时实测 | 应用内快捷键 + 托盘 + 剪贴板采集模式 |
| R5 | （**暂缓**，随 FR-2 移出 M0）划词采集成功率（Chromium/Electron/终端等）与剪贴板恢复完整性（图片/文件列表） | 后续采集功能立项时实测 | 受控重试 + 手动粘贴框 |
| R6 | 编码建议数据：① 全拼多音字/词级拼音准确性与许可（候选：pypinyin 数据、Rime 词典反查）；② algebra 求值器对 flypy 规则链（含 ⓆⓌ 占位 xlit、零声母 derive）的正确性；③ 音形词码规则与本地形码反查的覆盖率 | 用 flypy 方案全音节表对照官方键位图逐一校验；xhup 用户码表既有词（中国 vsgo、中国人 vgr 等合成词条）回归验证 | 用户可编辑建议编码；内置 flypy 键位事实表兜底 |
| R7 | YAML 保留注释的定点 patch 写入 | 对含注释/锚点/多文档的 custom.yaml 样本测试 | diff 确认 + 整树写入（明示损失） |
| R8 | Tauri 2 在 webkit2gtk-4.0（22.04）/4.1（24.04）双构建 | CI 双镜像构建 | 放弃 22.04 或 AppImage 兼容层 |
| R9 | 大词库（万象级 ≥ 50 万条）索引与部署耗时 | 压力测试 | 分片索引 + 空闲构建（参考工具已验证路线） |
| R10 | 凇鹤方案更新覆盖用户码表（§2.4 风险 1）：外部修改检测能否可靠区分「方案更新」与「用户手工编辑」 | 模拟覆盖 xhup_dicts 后启动工具，验证告警与恢复路径 | 写前备份 + 定期备份兜底；提示用户更新前先手动备份 |

---

## 7. 里程碑

| 里程碑 | 内容 | 出口标准 | 预估 |
| --- | --- | --- | --- |
| **M0 技术验证** | §6.5 R1–R3、R6–R8 实测（R4/R5 随采集暂缓；R2 Win 侧仅设计级）；Tauri Linux 骨架 + CI（Win 构建配置预留） | 验证报告 + go/no-go 决策；R1/R3 至少一条可行路径（本机实测） | 1–2 周 |
| **M1 MVP（P0）** | FR-1/3(P0)/5(P0)/6(P0)/7(P0)/8（含 FR-3.23 编码建议、静态预览）；J1/J2 旅程跑通 | Linux 本机内部 alpha（deb/AppImage）；核心 AC 全过；0 数据丢失 | 4–6 周 |
| **M2 v1.0（P1）** | 重码/历史/检查、librime 实时候选预览、配置管理、部署状态中心、定期备份、方案更新保护、mydict、自启、英文 | J3/J5/J6 旅程跑通；公测 → 正式发布（deb/AppImage；Win 安装包延后） | 4 周 |
| **M3 v1.x（P2）** | 词典中心、同步方案评审、应用内更新、水墨主题、更多前端评估；**划词快速采集与跨方案导入在此阶段重新评估立项** | 按社区反馈排序 | 持续 |

---

## 8. 验收与测试策略

- **单元测试**：词库解析/序列化往返（含注释、BOM、CRLF、异常格式）、编码建议、校验规则、patch 定点写入、备份命名/清理。
- **集成测试**：写入安全协议全链路（含外部修改冲突、磁盘满、权限错误注入）；部署状态机（成功/超时/失败/并发合并）。
- **平台矩阵测试**（M0 起持续）：**以 Linux 本机为门禁**——Ubuntu 26.04（本机）× X11/Wayland × fcitx5-rime；24.04/22.04 与 Win10/Win11 × Weasel 矩阵为设计级兼容目标，实测延后至有相应环境。划词应用矩阵随 FR-2 暂缓。
- **真实环境验收**：J1–J5 旅程在两端各跑一遍；用雾凇拼音与万象两套流行方案做兼容性验收（受管文件挂载差异）。
- **回归护栏**：任何写入路径必须通过「失败后原文件无损」测试；CI 双平台构建 + 核心测试门禁。

---

## 9. 风险与开放问题

| # | 问题 | 状态 |
| --- | --- | --- |
| Q1 | Wayland 全局热键 portal 在目标 Ubuntu 版本的实际覆盖（GNOME 支持进度） | 随 FR-2 暂缓，移出 M0；后续采集功能立项时验证 |
| Q2 | librime 版本碎片化（Weasel 内置 vs 系统库）是否值得自带 librime | M0 后决策；影响体积与预览一致性 |
| Q3 | 与万象/白霜等方案的专用文件（`wanxiang_user_phrase.txt` 等）兼容深度 | v1 走通用受管文件路线（已覆盖凇鹤双拼/音形，§2.4），万象/白霜专用适配 P2 评估 |
| Q4 | 多设备同步方案选型（WebDAV / Syncthing / Git / 自建） | P2 评审；v1 以导入导出满足迁移。**实测：Rime sync 已覆盖 custom_phrase_double.txt，但不覆盖 xhup 用户码表**——音形码表导出/迁移为 v1 刚需（FR-6.7③） |
| Q5 | 项目正式名称与许可证 | 待定（代号 RimeTools；建议 MIT/Apache-2.0） |
| Q6 | 参考工具功能取舍：是否需要「输入习惯」深度项与符号管理 | 已列 P2，按用户反馈再评 |
| Q7 | Windows ARM64 / Linux ARM64 是否纳入 | v1 不做，架构不阻塞 |

---

## 10. 附录

### 10.1 参考资料

- [RIME 配置小工具介绍页（RIME 坊）](https://rime.yougaoji.com/rime-config-tool-gui-setup-tool-intro/)
- [rime-config-tool GitHub（README/更新记录）](https://github.com/ysbushe/rime-config-tool)
- [凇鹤拼音 rime-crane（GitHub，GPL-3.0）](https://github.com/kchen0x/rime-crane)、[凇鹤拼音介绍（RIME 坊）](https://rime.yougaoji.com/rime-crane/)
- [小鹤双拼/音形官方](https://www.flypy.com/)
- [ashpd：Rust xdg-desktop-portal 客户端（GlobalShortcuts）](https://docs.rs/ashpd)
- 用户实机配置实测：`~/.local/share/fcitx5/rime`（2026-11-26，含方案/码表/同步目录/D-Bus 只读查询）
- [RimeVision 可视化配置](https://rime.yougaoji.com/rimevision/)
- [fcitx5-rime 部署实测：D-Bus 接口与 `fcitx5-remote -r` 陷阱](https://rainboylvx.github.io/blog/fcitx5-rime-ice-easy-en-capslock.html)
- [利用 pacman hook 自动部署 rime-wanxiang（D-Bus 部署命令）](https://levinion.github.io/posts/Linux/%E5%88%A9%E7%94%A8pacman%20hook%E8%87%AA%E5%8A%A8%E9%83%A8%E7%BD%B2rime-wanxiang/)
- [fcitx5-rime issue #28：D-Bus 触发部署](https://github.com/fcitx/fcitx5-rime/issues/28)、[issue #54](https://github.com/fcitx/fcitx5-rime/issues/54)
- [使用 fcitx5-rime 输入法（qdbus 部署示例）](https://blog.mkacg.com/2025/09/01/use-fcitx5-rime/)
- Rime 官方 wiki：[CustomizationGuide（patch 机制）](https://github.com/rime/home/wiki/CustomizationGuide)、[RimeWithSchemata](https://github.com/rime/home/wiki/RimeWithSchemata)

### 10.2 术语表

| 术语 | 含义 |
| --- | --- |
| 前端 | 承载 librime 的输入法程序：Weasel / Squirrel / fcitx5-rime / ibus-rime 等 |
| 用户目录 | 前端存放用户配置与词库的目录（Win `%APPDATA%\Rime`；fcitx5 `~/.local/share/fcitx5/rime`） |
| 部署 | librime 将 YAML 源配置/词库编译为 `build/` 二进制产物并加载的过程 |
| patch | `*.custom.yaml` 中对原配置的增量覆盖机制 |
| custom_phrase | Rime 自定义短语机制，`custom_phrase.txt` 按 `文本\t编码\t权重` 组织 |
| userdb | librime 自学习用户词库（LevelDB），本工具不直接读写 |
| 重码 | 同一编码对应多个文本（或同一文本多个编码）的冲突现象 |
| 隔离会话 | 工具独立创建的 librime session，仅用于候选预览，不影响用户正在使用的输入法 |
| 双拼 | 每音节压缩为两键（声母键+韵母键）的输入方式；小鹤双拼（flypy）为其中一种键位布局 |
| 小鹤音形 | 双拼 + 鹤形辅助码的方案（xhup）：单字全码 = 双拼 2 键 + 鹤形 2 键；词码只含双拼（二字词拼接、三字及以上取首键） |
| 鹤形 | 小鹤音形的字形辅助码；本工具从用户本地主码表运行时反查，不随工具分发 |
| 凇鹤拼音 | [rime-crane](https://github.com/kchen0x/rime-crane)，雾凇拼音 + 小鹤双拼/音形的整合配置仓库（GPL-3.0），本项目首要适配目标 |
| GlobalShortcuts portal | xdg-desktop-portal 的全局快捷键接口，Wayland 下替代系统级热键的标准途径 |

### 10.3 修订记录

| 版本 | 日期 | 变更 |
| --- | --- | --- |
| v0.1 | 2026-11-26 | 首版草稿：确认平台（Win + Ubuntu/fcitx5）、范围（词库核心 + 基础配置）、技术栈（Tauri 2 + Rust + Web）；完成功能/非功能需求、平台适配矩阵、M0 验证清单与里程碑 |
| v0.2 | 2026-11-26 | **适配用户实机凇鹤拼音（小鹤双拼 + 小鹤音形）**：新增 §2.4 实测基线；受管文件改为按方案自动识别（custom_phrase_double / xhup.user 码表）；编码建议扩展双拼 algebra 解析与音形词码/形码反查；Wayland 热键升级为 GlobalShortcuts portal 路线（GNOME 50.1 接口已确认）；新增 FR-1.8 / FR-3.20–3.22 / FR-6.7（方案更新覆盖保护）/ J6；平台基线更新为 Ubuntu 26.04；许可边界（GPL 码表运行时解析不分发）与 librime 双版本矩阵（1.13.1/1.16.1）落档 |
| v0.3 | 2026-11-26 | **范围裁剪（用户决策）**：划词快速采集（FR-2 整体）与跨方案导入（FR-3.19）暂缓至 M3 重新评估；编码建议/候选预览移入词库工作台（新增 FR-3.23/3.24，静态预览 P0、实时预览 P1）；v1 不再依赖全局热键/划词，Wayland 限制影响大幅降低；M0 验证项收敛为 R1–R3、R6–R8；J1 改为工作台路径、J4 暂缓；|
| v0.4 | 2026-11-26 | **开发验证策略（用户决策）**：开发机为 Linux，测试以本机（Ubuntu 26.04）通过为准；Windows 降为设计级兼容（适配层隔离 + 打包配置预留），Win 构建门禁/真机测试/安装包发布延后；里程碑出口与测试矩阵同步调整 |
| v0.5 | 2026-11-26 | **M0 验证完成，GO**：R1（librime FFI）已验证通过并修正 auto_select 机制理解（唯一候选才上屏）与解法；R3 接口参数已验证、真实部署待授权；新增「全码词」例外说明（`进口商 jkuh` 不可推导） |