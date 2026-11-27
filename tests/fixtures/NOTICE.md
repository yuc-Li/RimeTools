# 测试夹具来源与许可

本目录下的文件**仅用于测试**，不会被打包进发布产物。

## 合成数据（本项目原创，MIT）

以下文件是本项目**自行生成的合成夹具**，不含任何真实用户配置或词条：

- `rime-crane-xhup/xhup.user.dict.yaml`
- `rime-crane-xhup/xhup.user.top.dict.yaml`

其中两字词的编码由「内置拼音表 + 方案 algebra」计算得出，并用
`中国 → vsgo`、`你好 → nihc` 等已知映射做过自检。

## 上游方案文件（各自遵循原仓库许可）

为了让解析器与编码建议有真实结构可测，保留了以下上游文件的最小副本：

| 文件 | 来源 | 许可 |
| --- | --- | --- |
| `rime-crane-flypy/double_pinyin_flypy.schema.yaml` | [iDvel/rime-ice](https://github.com/iDvel/rime-ice) | GPL-3.0 |
| `rime-crane-flypy/rime_ice.dict.yaml` | 同上 | GPL-3.0 |
| `rime-crane-flypy/default.custom.yaml` | 同上 | GPL-3.0 |
| `rime-crane-flypy/custom_phrase_double.txt` | 同上（模板自带示例词，非本项目用户数据） | GPL-3.0 |
| `rime-crane-xhup/xhup.schema.yaml` | [kchen0x/rime-crane](https://github.com/kchen0x/rime-crane) | GPL-3.0 |
| `rime-crane-xhup/xhup.dict.yaml` | 同上，内含官方小鹤音形码表引用 | GPL-3.0 |
| `rime-crane-xhup/xhup.primary.sample.dict.yaml` | 小鹤音形主码表片段（按需加载码表） | 见上游 |

这些文件**不构成本项目发布物的一部分**，其版权与许可归原作者所有；
本项目（MIT）不改变、也不替代这些文件的许可条款。

如果要在**完全 MIT** 的环境中复用本仓库，可删除本目录并跳过依赖夹具的测试
（相关用例会失败而非静默通过）。
