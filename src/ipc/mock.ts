/**
 * 浏览器 mock 后端（纯 `npm run dev`、无 Tauri 宿主时使用）。
 *
 * 目的：让 UI 在没有 Rust 后端时也能完整开发调试。**只有词库工作台用它**，
 * 真实环境一律走 `src/ipc/client.ts` 的 `call()`（见 `src/pages/workbench/api.ts` 的分发）。
 *
 * 模拟的两类真实文件（用户实机「凇鹤拼音」）：
 * 1. 小鹤双拼（`double_pinyin_flypy`）→ `custom_phrase_double.txt`
 *    三列带权重：`词汇<Tab>编码<Tab>权重`，权重语义是「同编码内排序」（越大越靠前）。
 * 2. 小鹤音形（`xhup`）→ `xhup.user.dict.yaml` / `xhup.user.top.dict.yaml`
 *    **两列无权重**：`词汇<Tab>编码`，所以 `Entry.weight` 恒为 `null`、`columns` 里没有 `weight`。
 *
 * 编码规则依据 `docs/PRD.md` FR-3.23（与 Rust 侧 rime-suggest 一致）：
 * - 双拼映射按 fixtures 里 `double_pinyin_flypy.schema.yaml` 的 `speller/algebra` 化简而来
 *   （zh→v、ch→i、sh→u；iu=q、ei=w、uan=r、ue=t、un=y、uo=o、ie=p、ong=s、ing/uai=k …）；
 * - 音形词码：2 字 = 各字双拼全码拼接；3 字 = 各字首键；≥4 字 = 前 3 字首键 + 末字首键；
 * - 英文：小写 / 原样（短语去空格与连字符）。
 * 与夹具对齐的校验点：你好→nihc、编码→bmma、保存→bccy、数据→uuju、中国人→vgr、计算机→jsj。
 *
 * 数据安全语义同样模拟（`docs/ipc.md` §5.1 / §6）：
 * - `dict_save` 前若检测到「外部修改」，抛 `externalModified`，**绝不自动覆盖**；
 * - 保存返回新 `fingerprint`，UI 必须刷新（不做乐观更新）。
 */

import type {
  AppError,
  CodeSuggestion,
  Confidence,
  DeployResult,
  Entry,
  EntryKey,
  EntryPage,
  EntryQuery,
  ErrorCode,
  FileId,
  FileRole,
  Fingerprint,
  GroupCount,
  ManagedFileRef,
  SaveRequest,
  SchemeInfo,
  SuggestionKind,
  SuggestionWarning,
  ValidationIssue,
  // §3.5–3.7 部署 / 备份 / 设置
  AppSettings,
  BackupRef,
  DeployState,
  DeployTrigger,
  FileDiff,
  Theme,
} from "./types";
import { __setMockDeployState } from "./events";

// ───────────────────────── 通用工具 ─────────────────────────

/** 模拟网络/IPC 延迟，让加载态、竞态等 UI 分支真的会被执行到 */
function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function appError(code: ErrorCode, message: string, extra?: Partial<AppError>): AppError {
  return { code, message, ...extra };
}

/** 确定性伪 sha256（mock 只用于展示与「指纹变了」的比对，不做真实校验） */
function fakeSha(seed: number): string {
  let s = seed * 2654435761 + 0x9e3779b9;
  let out = "";
  for (let i = 0; i < 64; i++) {
    s = (s * 1103515245 + 12345) & 0x7fffffff;
    out += "0123456789abcdef"[s % 16];
  }
  return out;
}

const MOCK_EPOCH = 1_764_000_000_000; // 2025-11-26 前后，仅用于展示 mtime

function fakeFingerprint(version: number, size: number): Fingerprint {
  return {
    mtimeMs: MOCK_EPOCH + version * 61_000,
    size,
    sha256: fakeSha(version + size),
  };
}

function cloneEntry(e: Entry): Entry {
  return {
    key: { ...e.key },
    weight: e.weight,
    group: e.group,
    lineNo: e.lineNo,
    origin: e.origin,
  };
}

function keyId(key: EntryKey): string {
  return `${key.fileId}\u0000${key.text}\u0000${key.code}`;
}

// ───────────────────────── 小鹤双拼映射 ─────────────────────────

/**
 * 韵母键位（据 `double_pinyin_flypy.schema.yaml` 的 algebra 化简）。
 * 只保留「声母之后、末尾」的韵母形态；零声母单独处理（见 `flypy`）。
 */
const FLYPY_FINALS: Record<string, string> = {
  a: "a",
  o: "o",
  e: "e",
  i: "i",
  u: "u",
  v: "v", // ü
  ai: "d",
  ei: "w",
  ao: "c",
  ou: "z",
  an: "j",
  en: "f",
  ang: "h",
  eng: "g",
  er: "r",
  ia: "x",
  ie: "p",
  iao: "n",
  iu: "q",
  ian: "m",
  in: "b",
  iang: "l",
  ing: "k",
  iong: "s",
  ua: "x",
  uo: "o",
  uai: "k",
  ui: "v",
  uan: "r",
  un: "y",
  uang: "l",
  ong: "s",
  ve: "t", // lve / nve / xue
  ue: "t",
  "ü": "v",
  "üe": "t",
};

/** 声母键位：zh/ch/sh 各占一位 */
const FLYPY_INITIALS: Record<string, string> = { zh: "v", ch: "i", sh: "u" };

/**
 * 全拼音节 → 小鹤双拼码。
 * 零声母按 algebra 的主形态：a→aa、ang→ah、an→an、ai→ai、ou→ou、er→er。
 */
export function flypy(syllable: string): string | null {
  const syl = syllable.toLowerCase();

  // 零声母
  if (!/^(?:zh|ch|sh|[bpmfdtnlgkhjqxrzcsyw])/.test(syl)) {
    if (/^[aoe]$/.test(syl)) return syl + syl;
    // ang / eng / ong：首字母重复 + 韵母键（ang→ah）
    if (/^([aoe])ng$/.test(syl)) {
      const key = FLYPY_FINALS[syl.slice(1)];
      return key ? syl[0] + key : null;
    }
    // an / ai / en / ou / ei：保持短式（与 algebra 主形态一致）
    if (/^([aoe])[ioun]$/.test(syl)) return syl;
    if (syl === "er") return "er";
    return FLYPY_FINALS[syl] ? syl[0] + FLYPY_FINALS[syl] : null;
  }

  const initial = /^(zh|ch|sh)/.exec(syl)?.[0] ?? syl[0];
  const rest = syl.slice(initial.length);
  const initialKey = FLYPY_INITIALS[initial] ?? initial;

  if (!rest) return null;
  const finalKey = FLYPY_FINALS[rest];
  if (!finalKey) return null;
  return initialKey + finalKey;
}

// ───────────────────────── 常用字读音表 ─────────────────────────
//
// mock 只内置常用字（真实后端由 rime-suggest 的拼音/码表数据提供）。
// 多音字给多个读音，用于演示 `SuggestionWarning.polyphone` 与多候选。
const READINGS: Record<string, string[]> = {
  // 一 / 以 / 已 / 亦 / 议 / 义 / 意 / 衣 / 依...
  一: ["yi"], 以: ["yi"], 已: ["yi"], 亦: ["yi"], 议: ["yi"], 义: ["yi"], 意: ["yi"], 衣: ["yi"], 依: ["yi"],
  有: ["you"], 又: ["you"], 由: ["you"], 友: ["you"], 邮: ["you"],
  个: ["ge"], 各: ["ge"], 格: ["ge"], 更: ["geng"],
  说: ["shuo"], 是: ["shi"], 时: ["shi"], 识: ["shi"], 试: ["shi"], 失: ["shi"], 十: ["shi"],
  美: ["mei"], 没: ["mei"], 每: ["mei"], 妹: ["mei"], 眉: ["mei"],
  团: ["tuan"], 关: ["guan"], 管: ["guan"], 官: ["guan"], 观: ["guan"],
  报: ["bao"], 保: ["bao"], 包: ["bao"], 宝: ["bao"],
  单: ["dan", "shan"], 但: ["dan"], 蛋: ["dan"],
  编: ["bian"], 边: ["bian"], 变: ["bian"],
  码: ["ma"], 马: ["ma"], 吗: ["ma"],
  建: ["jian"], 件: ["jian"], 见: ["jian"], 简: ["jian"], 间: ["jian"], 检: ["jian"], 键: ["jian"],
  词: ["ci"], 此: ["ci"], 次: ["ci"],
  库: ["ku"], 苦: ["ku"], 快: ["kuai"],
  理: ["li"], 力: ["li"], 立: ["li"], 里: ["li"], 李: ["li"],
  摸: ["mo"], 莫: ["mo"], 默: ["mo"], 末: ["mo"],
  鱼: ["yu"], 于: ["yu"], 语: ["yu"], 雨: ["yu"], 与: ["yu"], 域: ["yu"],
  会: ["hui", "kuai"], 回: ["hui"], 恢: ["hui"],
  儿: ["er"], 而: ["er"],
  精: ["jing"], 京: ["jing"], 经: ["jing"], 静: ["jing"],
  重: ["zhong", "chong"], 中: ["zhong"], 钟: ["zhong"], 种: ["zhong"],
  置: ["zhi"], 只: ["zhi"], 知: ["zhi"], 直: ["zhi"], 至: ["zhi"], 制: ["zhi"], 志: ["zhi"],
  骑: ["qi"], 其: ["qi"], 起: ["qi"], 期: ["qi"], 启: ["qi"], 器: ["qi"], 气: ["qi"],
  行: ["xing", "hang"], 形: ["xing"], 星: ["xing"], 兴: ["xing"], 型: ["xing"],
  张: ["zhang"], 章: ["zhang"], 长: ["chang", "zhang"], 常: ["chang"], 场: ["chang"],
  三: ["san"], 散: ["san"], 伞: ["san"],
  标: ["biao"], 表: ["biao"], 别: ["bie"],
  输: ["shu"], 书: ["shu"], 数: ["shu"], 属: ["shu"], 树: ["shu"], 术: ["shu"],
  入: ["ru"], 如: ["ru"], 日: ["ri"],
  法: ["fa"], 发: ["fa"],
  文: ["wen"], 问: ["wen"], 稳: ["wen"],
  测: ["ce"], 策: ["ce"], 册: ["ce"],
  自: ["zi"], 字: ["zi"], 子: ["zi"],
  定: ["ding"], 顶: ["ding"], 订: ["ding"],
  短: ["duan"], 段: ["duan"], 断: ["duan"],
  权: ["quan"], 全: ["quan"], 泉: ["quan"],
  分: ["fen"], 份: ["fen"], 芬: ["fen"],
  组: ["zu"], 族: ["zu"], 祖: ["zu"],
  搜: ["sou"], 艘: ["sou"],
  索: ["suo"], 所: ["suo"], 锁: ["suo"],
  排: ["pai"], 派: ["pai"], 牌: ["pai"],
  序: ["xu"], 需: ["xu"], 徐: ["xu"], 许: ["xu"],
  存: ["cun"], 村: ["cun"],
  部: ["bu"], 不: ["bu"], 布: ["bu"], 步: ["bu"],
  署: ["shu"],
  备: ["bei"], 北: ["bei"], 被: ["bei"], 背: ["bei"],
  哈: ["ha"],
  东: ["dong"], 动: ["dong"], 懂: ["dong"],
  上: ["shang"], 商: ["shang"], 尚: ["shang"],
  海: ["hai"], 还: ["hai", "huan"],
  广: ["guang"], 光: ["guang"],
  州: ["zhou"], 周: ["zhou"], 洲: ["zhou"],
  深: ["shen"], 身: ["shen"], 神: ["shen"],
  圳: ["zhen"], 真: ["zhen"], 阵: ["zhen"], 镇: ["zhen"],
  今: ["jin"], 金: ["jin"], 进: ["jin"], 近: ["jin"], 仅: ["jin"],
  天: ["tian"], 田: ["tian"], 填: ["tian"],
  明: ["ming"], 名: ["ming"], 命: ["ming"],
  工: ["gong"], 公: ["gong"], 功: ["gong"], 共: ["gong"],
  作: ["zuo"], 做: ["zuo"], 坐: ["zuo"],
  学: ["xue"], 雪: ["xue"],
  习: ["xi"], 系: ["xi"], 西: ["xi"], 洗: ["xi"], 喜: ["xi"], 细: ["xi"],
  电: ["dian"], 点: ["dian"], 店: ["dian"], 典: ["dian"],
  脑: ["nao"], 闹: ["nao"],
  手: ["shou"], 受: ["shou"], 收: ["shou"], 首: ["shou"],
  机: ["ji"], 及: ["ji"], 级: ["ji"], 即: ["ji"], 计: ["ji"], 记: ["ji"], 技: ["ji"], 己: ["ji"],
  微: ["wei"], 为: ["wei"], 位: ["wei"], 未: ["wei"], 味: ["wei"], 委: ["wei"],
  信: ["xin"], 新: ["xin"], 心: ["xin"], 欣: ["xin"],
  箱: ["xiang"], 想: ["xiang"], 相: ["xiang"], 项: ["xiang"], 向: ["xiang"], 像: ["xiang"],
  密: ["mi"], 米: ["mi"], 迷: ["mi"],
  题: ["ti"], 提: ["ti"], 体: ["ti"],
  统: ["tong"], 同: ["tong"], 通: ["tong"],
  据: ["ju"], 句: ["ju"], 局: ["ju"], 聚: ["ju"],
  配: ["pei"], 培: ["pei"],
  方: ["fang"], 放: ["fang"], 房: ["fang"],
  案: ["an"], 安: ["an"], 按: ["an"], 暗: ["an"],
  装: ["zhuang"], 状: ["zhuang"], 庄: ["zhuang"],
  删: ["shan"], 山: ["shan"], 闪: ["shan"], 善: ["shan"],
  除: ["chu"], 出: ["chu"], 初: ["chu"], 处: ["chu"],
  辑: ["ji"], 集: ["ji"],
  加: ["jia"], 家: ["jia"], 假: ["jia"], 价: ["jia"], 夹: ["jia"],
  载: ["zai"], 在: ["zai"], 再: ["zai"],
  你: ["ni"], 尼: ["ni"], 泥: ["ni"],
  好: ["hao"], 号: ["hao"], 毫: ["hao"],
  谢: ["xie"], 些: ["xie"], 写: ["xie"],
  完: ["wan"], 晚: ["wan"], 万: ["wan"], 湾: ["wan"],
  成: ["cheng"], 程: ["cheng"], 城: ["cheng"], 承: ["cheng"],
  败: ["bai"], 白: ["bai"], 百: ["bai"], 拜: ["bai"],
  认: ["ren"], 人: ["ren"], 任: ["ren"], 仁: ["ren"],
  注: ["zhu"], 主: ["zhu"], 住: ["zhu"], 祝: ["zhu"], 著: ["zhu"], 助: ["zhu"],
  复: ["fu"], 付: ["fu"], 服: ["fu"], 福: ["fu"], 副: ["fu"],
  查: ["cha"], 茶: ["cha"], 差: ["cha"],
  升: ["sheng"], 生: ["sheng"], 声: ["sheng"], 省: ["sheng"],
  帮: ["bang"], 棒: ["bang"],
  版: ["ban"], 办: ["ban"], 半: ["ban"], 班: ["ban"],
  本: ["ben"], 奔: ["ben"],
  界: ["jie"], 接: ["jie"], 解: ["jie"], 借: ["jie"], 节: ["jie"],
  面: ["mian"], 免: ["mian"], 棉: ["mian"],
  条: ["tiao"], 挑: ["tiao"], 跳: ["tiao"],
  拼: ["pin"], 品: ["pin"], 贫: ["pin"],
  音: ["yin"], 因: ["yin"], 银: ["yin"], 引: ["yin"],
  双: ["shuang"], 霜: ["shuang"],
  小: ["xiao"], 笑: ["xiao"], 校: ["xiao"], 消: ["xiao"],
  鹤: ["he"], 和: ["he", "huo"], 合: ["he"], 河: ["he"], 喝: ["he"],
  凇: ["song"], 松: ["song"], 送: ["song"], 宋: ["song"],
  空: ["kong"], 孔: ["kong"], 控: ["kong"],
  候: ["hou"], 后: ["hou"], 厚: ["hou"],
  选: ["xuan"], 宣: ["xuan"], 玄: ["xuan"],
  口: ["kou"],
};

// ───────────────────────── mock 数据仓库 ─────────────────────────

interface MockFile {
  ref: ManagedFileRef;
  schemeId: string;
  entries: Entry[];
  nextLineNo: number;
  version: number;
  fingerprint: Fingerprint;
  /** 模拟「外部程序改过文件」：下一次 dict_save 必定返回 externalModified（见 §5.1） */
  externallyModified: boolean;
}

const RIME_DIR = "/home/user/.local/share/fcitx5/rime";

const CUSTOM_PHRASE_ID: FileId = `cp:${RIME_DIR}/custom_phrase_double.txt`;
const XHUP_USER_ID: FileId = `dict:${RIME_DIR}/xhup.user.dict.yaml`;
const XHUP_TOP_ID: FileId = `dict:${RIME_DIR}/xhup.user.top.dict.yaml`;

const files = new Map<FileId, MockFile>();
const schemes: SchemeInfo[] = [];

function makeEntry(
  fileId: FileId,
  text: string,
  code: string,
  weight: number | null,
  group: string | null,
  lineNo: number,
): Entry {
  return {
    key: { fileId, text, code },
    weight,
    group,
    lineNo,
    origin: "file",
  };
}

/** 按行号顺序造行，返回 [Entry[] , 下一行号] */
function seedRows(
  fileId: FileId,
  rows: Array<[text: string, code: string, weight: number | null, group: string | null]>,
  firstLineNo = 1,
): Entry[] {
  return rows.map(([text, code, weight, group], i) =>
    makeEntry(fileId, text, code, weight, group, firstLineNo + i),
  );
}

function installFile(
  ref: ManagedFileRef,
  schemeId: string,
  entries: Entry[],
): MockFile {
  const file: MockFile = {
    ref,
    schemeId,
    entries,
    nextLineNo: entries.length + 1,
    version: 1,
    fingerprint: fakeFingerprint(1, entries.length * 24 + 512),
    externallyModified: false,
  };
  files.set(ref.id, file);
  return file;
}

function seed(): void {
  // ── ① 小鹤双拼：custom_phrase_double.txt（三列，权重可选） ──
  // 行内容取自 tests/fixtures/rime-crane-flypy/custom_phrase_double.txt，权重/分组为演示数据。
  const cpRef: ManagedFileRef = {
    id: CUSTOM_PHRASE_ID,
    path: `${RIME_DIR}/custom_phrase_double.txt`,
    format: "customPhrase",
    role: "customPhrase",
    columns: ["text", "code", "weight"],
  };
  const cpEntries = seedRows(CUSTOM_PHRASE_ID, [
    ["噷", "hm", null, null],
    ["呣", "m", null, null],
    ["呒", "m", null, null],
    ["有", "u", 3, "高频"],
    ["又", "u", 2, "高频"],
    ["由", "u", 1, null],
    ["一", "i", 4, "高频"],
    ["以", "i", 3, null],
    ["已", "i", 2, null],
    ["亦", "i", 1, null],
    ["一个", "ig", null, null],
    ["有个", "ug", null, null],
    ["是一个", "sig", null, null],
    ["有一个", "uig", null, null],
    ["有没有", "umu", null, null],
    ["哈哈哈", "hhh", null, "聊天"],
    ["又有", "uu", null, null],
    ["又又又", "uuu", null, null],
    ["又双叒叕", "uuuu", null, null],
    ["go", "go", null, "英文"],
    ["Golang", "golang", null, "英文"],
    ["Hugo", "hugo", null, null],
    ["goto", "goto", null, null],
    ["domain", "domain", null, "英文"],
    ["TODO", "todo", null, null],
    ["Dota", "dota", null, null],
    ["main", "main", null, null],
    ["Amazon", "amazon", null, "英文"],
  ], 262);
  installFile(cpRef, "double_pinyin_flypy", cpEntries);

  // ── ② 小鹤音形：xhup.user.dict.yaml（两列，无权重） ──
  const userRef: ManagedFileRef = {
    id: XHUP_USER_ID,
    path: `${RIME_DIR}/xhup.user.dict.yaml`,
    format: "rimeDictYaml",
    role: "userDict",
    columns: ["text", "code"],
  };
  const userEntries = seedRows(XHUP_USER_ID, [
    ["中国人", "vgr", null, null],
    ["测试", "ceui", null, null],
    ["00000000000", "puj", null, null],
    ["张三", "pxm", null, null],
    ["000000000000000000", "pufv", null, null],
    ["计算机", "jsj", null, null],
    ["编码", "bmma", null, null],
    ["中文", "vswf", null, null],
    ["你好", "nihc", null, null],
    ["保存", "bccy", null, null],
    ["数据", "uuju", null, null],
    ["输入法", "urf", null, null],
    ["词库", "ciku", null, null],
    ["拼音", "pbyb", null, null],
    ["双拼", "ulpb", null, null],
    ["自定义", "zdy", null, null],
    ["编码", "bmma", null, null],
    ["候选", "hzxr", null, null],
    ["测试", "ceui", null, null],
    ["备份", "bwff", null, null],
    ["删除", "ujiu", null, null],
    ["编辑", "bmji", null, null],
    ["文件", "wfjm", null, null],
    ["文件夹", "wjj", null, null],
  ], 10);
  installFile(userRef, "xhup", userEntries);

  // ── ③ 小鹤音形：xhup.user.top.dict.yaml（置顶码表，两列） ──
  const topRef: ManagedFileRef = {
    id: XHUP_TOP_ID,
    path: `${RIME_DIR}/xhup.user.top.dict.yaml`,
    format: "rimeDictYaml",
    role: "userTopDict",
    columns: ["text", "code"],
  };
  const topEntries = seedRows(XHUP_TOP_ID, [
    ["进口商", "jkuh", null, null],
    ["按时", "anui", null, null],
    ["按到", "andc", null, null],
    ["而是", "erui", null, null],
    ["反应", "fjyk", null, null],
    ["成为", "igww", null, null],
    ["传统", "irts", null, null],
    ["即可", "jike", null, null],
    ["简单", "jmdj", null, null],
    ["计算", "jisr", null, null],
    ["联系", "lmxi", null, null],
    ["每周", "mwvz", null, null],
    ["美丽", "mwli", null, null],
    ["期间", "qijm", null, null],
    ["台湾", "tdwj", null, null],
    ["时间", "uijm", null, null],
    ["水平", "uvpk", null, null],
    ["修正", "xqvg", null, null],
    ["现金", "xmjb", null, null],
    ["一直", "yivi", null, null],
    ["在线", "zdxm", null, null],
  ], 10);
  installFile(topRef, "xhup", topEntries);

  schemes.push(
    {
      schemaId: "double_pinyin_flypy",
      name: "小鹤双拼",
      version: "1",
      isActive: true,
      isCurrent: true,
      translatorKind: "script",
      alphabet: "abcdefghijklmnopqrstuvwxyz`",
      managedFiles: [cpRef],
      isDoublePinyin: true,
      algebra: {
        rules: [
          "erase/^xx$/",
          "derive/^([jqxy])u$/$1v/",
          "xform/^sh/Ⓤ/",
          "xform/^ch/Ⓘ/",
          "xform/^zh/Ⓥ/",
        ],
        alphabet: "abcdefghijklmnopqrstuvwxyz`",
      },
    },
    {
      schemaId: "xhup",
      name: "小鹤音形",
      version: "v10.9s",
      isActive: true,
      isCurrent: false,
      translatorKind: "table",
      alphabet: "abcdefghijklmnopqrstuvwxyz;'",
      maxCodeLength: 4,
      managedFiles: [userRef, topRef],
      isDoublePinyin: true,
    },
  );
}

seed();

// ───────────────────────── 查询（dict_query / dict_groups） ─────────────────────────

interface CachedQuery {
  version: number;
  total: number;
  rows: Entry[];
}

const queryCache = new Map<string, CachedQuery>();

function sortRows(rows: Entry[], field: EntryQuery["sort"]["field"], desc: boolean): Entry[] {
  const dir = desc ? -1 : 1;
  const cmp = (a: Entry, b: Entry): number => {
    switch (field) {
      case "text":
        return (
          a.key.text.localeCompare(b.key.text, "zh-Hans-CN") ||
          a.key.code.localeCompare(b.key.code)
        );
      case "code":
        return (
          a.key.code.localeCompare(b.key.code, "en") ||
          a.key.text.localeCompare(b.key.text, "zh-Hans-CN")
        );
      case "weight":
        return (a.weight ?? -1) - (b.weight ?? -1) || (a.lineNo ?? 0) - (b.lineNo ?? 0);
      case "lineNo":
      default:
        return (a.lineNo ?? 0) - (b.lineNo ?? 0);
    }
  };
  return rows.sort((a, b) => cmp(a, b) * dir);
}

function filteredRows(file: MockFile, q: EntryQuery): Entry[] {
  const keyword = (q.keyword ?? "").trim().toLowerCase();
  const group = q.group ?? null;
  const cacheKey = [
    file.ref.id,
    file.version,
    keyword,
    group === null ? "\u0000null" : group,
    q.sort.field,
    q.sort.desc ? "desc" : "asc",
  ].join("\u0001");

  const hit = queryCache.get(cacheKey);
  if (hit && hit.version === file.version) return hit.rows;

  let rows = file.entries;
  if (keyword) {
    // 关键词同时匹配文本与编码，大小写不敏感（契约 §2 EntryQuery）
    rows = rows.filter(
      (e) =>
        e.key.text.toLowerCase().includes(keyword) || e.key.code.toLowerCase().includes(keyword),
    );
  }
  if (group !== null) {
    rows = rows.filter((e) => (e.group ?? "") === group);
  }
  const sorted = sortRows([...rows], q.sort.field, q.sort.desc);
  const cached: CachedQuery = { version: file.version, total: sorted.length, rows: sorted };
  // 简单 LRU：最多留 8 份查询结果，避免压测数据把内存吃满
  if (queryCache.size >= 8) {
    const oldest = queryCache.keys().next().value;
    if (oldest !== undefined) queryCache.delete(oldest);
  }
  queryCache.set(cacheKey, cached);
  return sorted;
}

function groupCounts(file: MockFile): GroupCount[] {
  const counts = new Map<string, number>();
  for (const e of file.entries) {
    const name = e.group ?? "";
    counts.set(name, (counts.get(name) ?? 0) + 1);
  }
  return [...counts.entries()]
    .map(([name, count]) => ({ name, count }))
    .sort((a, b) => (a.name === "" ? 1 : b.name === "" ? -1 : a.name.localeCompare(b.name)));
}

function queryPage(file: MockFile, q: EntryQuery): EntryPage {
  const rows = filteredRows(file, q);
  const offset = Math.max(0, q.offset);
  const limit = Math.max(0, q.limit);
  return {
    items: rows.slice(offset, offset + limit).map(cloneEntry),
    total: rows.length,
    groups: groupCounts(file),
  };
}

// ───────────────────────── 校验（dict_validate，FR-3.11 简化版） ─────────────────────────

/** 用「现有条目 + 本次变更」构造最终结果集，用于查重 */
function projectEntries(
  file: MockFile,
  req: SaveRequest,
): Array<{ text: string; code: string; weight: number | null; group: string | null; lineNo: number | null }> {
  const deleted = new Set(req.delete.map(keyId));
  const updated = new Map(
    req.update.map((u) => [
      keyId(u.key),
      {
        text: u.newText ?? u.key.text,
        code: u.newCode ?? u.key.code,
        weight: u.weight === undefined ? null : u.weight,
        group: u.group === undefined ? null : u.group,
        lineNo: null as number | null,
      },
    ]),
  );

  const result: Array<{
    text: string;
    code: string;
    weight: number | null;
    group: string | null;
    lineNo: number | null;
  }> = [];

  for (const e of file.entries) {
    const id = keyId(e.key);
    if (deleted.has(id)) continue;
    const patch = updated.get(id);
    result.push(
      patch ?? {
        text: e.key.text,
        code: e.key.code,
        weight: e.weight,
        group: e.group,
        lineNo: e.lineNo,
      },
    );
  }
  for (const add of req.add) {
    result.push({
      text: add.text,
      code: add.code,
      weight: add.weight ?? null,
      group: add.group ?? null,
      lineNo: null,
    });
  }
  return result;
}

function validateMock(file: MockFile, req: SaveRequest): ValidationIssue[] {
  const scheme = schemes.find((s) => s.schemaId === file.schemeId);
  const alphabet = new Set((scheme?.alphabet ?? "").split(""));
  const maxLen = scheme?.maxCodeLength;
  const hasWeightColumn = file.ref.columns.includes("weight");
  const issues: ValidationIssue[] = [];
  const seen = new Map<string, number>();

  for (const row of projectEntries(file, req)) {
    const { text, code, weight, lineNo } = row;
    const lineNote = lineNo ?? undefined;

    if (!text) {
      issues.push({
        code: "validationFailed",
        message: "文本不能为空",
        lineNo: lineNote,
        entryCode: code,
      });
      continue;
    }
    if (!code) {
      issues.push({
        code: "validationFailed",
        message: `「${text}」的编码不能为空`,
        lineNo: lineNote,
        text,
      });
      continue;
    }
    if (/[\t\n\r]/.test(text) || /[\t\n\r]/.test(code)) {
      issues.push({
        code: "fileFormatInvalid",
        message: "文本或编码包含 Tab / 换行",
        lineNo: lineNote,
        text,
        entryCode: code,
      });
    }
    if (alphabet.size > 0) {
      const bad = [...code].filter((c) => !alphabet.has(c));
      if (bad.length > 0) {
        issues.push({
          code: "codeCharsetInvalid",
          message: `编码含方案字母表之外的字符：${[...new Set(bad)].join(" ")}`,
          lineNo: lineNote,
          text,
          entryCode: code,
        });
      }
    }
    if (maxLen != null && code.length > maxLen) {
      issues.push({
        code: "codeTooLong",
        message: `编码长度 ${code.length} 超过方案上限 ${maxLen}`,
        lineNo: lineNote,
        text,
        entryCode: code,
      });
    }
    if (hasWeightColumn && weight !== null) {
      if (!Number.isInteger(weight) || weight < 0) {
        issues.push({
          code: "validationFailed",
          message: `权重必须是非负整数（当前：${weight}）`,
          lineNo: lineNote,
          text,
          entryCode: code,
        });
      }
    }
    const dup = `${text}\u0000${code}`;
    if (seen.has(dup)) {
      issues.push({
        code: "duplicateEntry",
        message: `重复词条：${text} / ${code}`,
        lineNo: lineNote,
        text,
        entryCode: code,
      });
    } else {
      seen.set(dup, 1);
    }
  }
  return issues;
}

// ───────────────────────── 编码建议（suggest_codes，FR-3.23） ─────────────────────────

const KIND_LABEL: Record<SuggestionKind, string> = {
  fullPinyin: "全拼",
  doublePinyin: "双拼",
  abbrev: "简拼",
  shapeWord: "音形词码",
  shapeChar: "单字形码",
  english: "英文",
  history: "历史",
};

function combos(readings: string[][], limit: number): string[][] {
  let acc: string[][] = [[]];
  for (const options of readings) {
    const next: string[][] = [];
    for (const prefix of acc) {
      for (const option of options) {
        next.push([...prefix, option]);
        if (next.length >= limit * 4) break;
      }
      if (next.length >= limit * 4) break;
    }
    acc = next;
  }
  // 主形态优先（第一个），其余按生成顺序截断
  return acc.slice(0, limit);
}

/** 音形词码：2 字 = 各字双拼全码；3 字 = 各字首键；≥4 字 = 前 3 字首键 + 末字首键 */
function shapeWordCode(syllables: string[]): string | null {
  const double = syllables.map(flypy);
  if (double.some((c) => c === null)) return null;
  const codes = double as string[];
  if (codes.length === 1) return codes[0];
  if (codes.length === 2) return codes[0] + codes[1];
  if (codes.length === 3) return codes.map((c) => c[0]).join("");
  return codes
    .slice(0, 3)
    .map((c) => c[0])
    .join("") + codes[codes.length - 1][0];
}

function suggestionsForText(
  text: string,
  scheme: SchemeInfo,
  role: FileRole,
  exclude: Set<string>,
): CodeSuggestion[] {
  const maxLen = scheme.maxCodeLength;
  const alphabet = new Set(scheme.alphabet.split(""));
  const out: CodeSuggestion[] = [];

  const push = (code: string, kind: SuggestionKind, confidence: Confidence, reading?: string, textHasPolyphone = false) => {
    if (!code || exclude.has(code)) return;
    if (out.some((s) => s.code === code && s.kind === kind)) return;
    const warnings = new Set<SuggestionWarning>();
    // FR-3.22：向 custom_phrase 写入「完整拼音/双拼码」会阻断自学习造词
    if (role === "customPhrase" && (kind === "fullPinyin" || kind === "doublePinyin")) {
      warnings.add("blocksLearning");
    }
    if (maxLen != null && code.length > maxLen) warnings.add("exceedsMaxCodeLength");
    if (alphabet.size > 0 && [...code].some((c) => !alphabet.has(c))) {
      warnings.add("unsupportedCharset");
    }
    if (textHasPolyphone) warnings.add("polyphone");
    out.push({
      code,
      kind,
      label: KIND_LABEL[kind],
      reading,
      confidence,
      warnings: [...warnings],
    });
  };

  // 英文：小写 / 原样（短语去空格与连字符，FR-3.23）
  if (/^[\x20-\x7e]+$/.test(text) && /[A-Za-z]/.test(text)) {
    const compact = text.replace(/[\s-]+/g, "");
    push(compact.toLowerCase(), "english", "high");
    if (compact !== compact.toLowerCase()) {
      push(compact, "english", "medium");
    }
    return out;
  }

  const readings: string[][] = [];
  for (const ch of [...text]) {
    const r = READINGS[ch];
    if (!r) return []; // 未知字：交给用户手填编码（建议只是默认填充，永远可编辑）
    readings.push(r);
  }
  const textHasPolyphone = readings.some((r) => r.length > 1);

  const expanded = combos(readings, 4);
  for (const [index, syllables] of expanded.entries()) {
    const reading = syllables.join(" ");
    const readingWarn = textHasPolyphone; // 多音字：请确认读音
    const confidence: Confidence = index === 0 ? "high" : "medium";

    if (scheme.translatorKind === "table") {
      if (syllables.length === 1) {
        const code = flypy(syllables[0]);
        if (code) push(code, "shapeChar", confidence, reading, readingWarn);
      } else {
        const code = shapeWordCode(syllables);
        if (code) push(code, "shapeWord", confidence, reading, readingWarn);
      }
    } else {
      const double = syllables.map(flypy);
      if (!double.some((c) => c === null)) {
        push(syllables.join(""), "fullPinyin", confidence, reading, readingWarn);
        push((double as string[]).join(""), "doublePinyin", confidence, reading, readingWarn);
        push(syllables.map((s) => s[0]).join(""), "abbrev", "low", reading, readingWarn);
      }
    }
  }
  return out;
}

// ───────────────────────── 保存（dict_save，§5.1） ─────────────────────────

function applySave(file: MockFile, req: SaveRequest): number {
  const deleted = new Set(req.delete.map(keyId));
  file.entries = file.entries.filter((e) => !deleted.has(keyId(e.key)));

  let written = req.add.length;
  for (const u of req.update) {
    const entry = file.entries.find((e) => keyId(e.key) === keyId(u.key));
    if (!entry) continue;
    entry.key.text = u.newText ?? entry.key.text;
    entry.key.code = u.newCode ?? entry.key.code;
    if (file.ref.columns.includes("weight")) {
      entry.weight = u.weight === undefined ? entry.weight : u.weight;
    }
    if (u.group !== undefined) entry.group = u.group;
    entry.origin = "userEdited";
    written += 1;
  }

  for (const add of req.add) {
    file.entries.push({
      key: { fileId: file.ref.id, text: add.text, code: add.code },
      weight: file.ref.columns.includes("weight") ? add.weight ?? null : null,
      group: add.group ?? null,
      lineNo: file.nextLineNo++,
      origin: "userAdded",
    });
  }

  if (req.reorder && req.reorder.length > 0) {
    // mock：按 reorder 的先后重排（未列出的保持原有相对顺序）
    const order = new Map(req.reorder.map((k, i) => [keyId(k), i]));
    file.entries.sort((a, b) => {
      const ai = order.get(keyId(a.key));
      const bi = order.get(keyId(b.key));
      if (ai === undefined && bi === undefined) return (a.lineNo ?? 0) - (b.lineNo ?? 0);
      if (ai === undefined) return 1;
      if (bi === undefined) return -1;
      return ai - bi;
    });
    file.entries.forEach((e, i) => {
      e.lineNo = i + 1;
    });
    file.nextLineNo = file.entries.length + 1;
  }

  file.version += 1;
  file.fingerprint = fakeFingerprint(file.version, file.entries.length * 24 + 512);
  return written;
}

function mockDeploy(trigger: DeployResult["trigger"]): DeployResult {
  return {
    trigger,
    success: true,
    durationMs: 180 + (trigger === "afterSave" ? 60 : 0),
    outputVerified: true,
  };
}

// ───────────────────────── 压测数据（演示虚拟滚动） ─────────────────────────

const LETTERS = "abcdefghijklmnopqrstuvwxyz";

function lettersOf(index: number): string {
  let n = index;
  let out = "";
  for (let i = 0; i < 4; i++) {
    out += LETTERS[n % 26];
    n = Math.floor(n / 26);
  }
  return out;
}

/**
 * 向指定文件灌入 N 行合成数据，用于验证「10 万行虚拟滚动」。
 * 只在 mock 模式下由工作台的压测按钮调用。
 */
export function mockFillStress(fileId: FileId, count = 100_000): { fileId: FileId; total: number } {
  const file = files.get(fileId);
  if (!file) throw appError("notFound", `未注册的受管文件：${fileId}`);
  const withWeight = file.ref.columns.includes("weight");
  const start = file.entries.length;
  const rows: Entry[] = [];
  for (let i = 0; i < count; i++) {
    const n = start + i;
    rows.push({
      key: { fileId, text: `压测词条${n + 1}`, code: lettersOf(n) },
      weight: withWeight ? (n % 7) + 1 : null,
      group: n % 10 === 0 ? "压测" : null,
      lineNo: n + 1,
      origin: "file",
    });
  }
  file.entries = [...file.entries, ...rows];
  file.nextLineNo = file.entries.length + 1;
  file.version += 1;
  file.fingerprint = fakeFingerprint(file.version, file.entries.length * 24 + 512);
  queryCache.clear();
  return { fileId, total: file.entries.length };
}

/**
 * 模拟「外部程序修改了文件」：往文件里追加一行并置脏标记。
 * 下一次 `dict_save` 会返回 `externalModified`（数据安全红线：绝不自动覆盖）。
 */
export function mockSimulateExternalChange(fileId: FileId): { fileId: FileId; fingerprint: Fingerprint } {
  const file = files.get(fileId);
  if (!file) throw appError("notFound", `未注册的受管文件：${fileId}`);
  const line = file.nextLineNo++;
  file.entries.push({
    key: { fileId, text: "外部程序新增", code: file.ref.columns.includes("weight") ? "waibu" : "wbxz" },
    weight: file.ref.columns.includes("weight") ? 9 : null,
    group: "外部",
    lineNo: line,
    origin: "file",
  });
  file.version += 1;
  file.fingerprint = fakeFingerprint(file.version, file.entries.length * 24 + 512);
  file.externallyModified = true;
  queryCache.clear();
  return { fileId, fingerprint: file.fingerprint };
}

/** 压测/演示用：mock 里当前所有文件 */
export function mockFiles(): ManagedFileRef[] {
  return [...files.values()].map((f) => f.ref);
}

// ───────────────────────── 命令分发 ─────────────────────────

function requireFile(fileId: FileId | undefined): MockFile {
  const file = fileId ? files.get(fileId) : undefined;
  if (!file) {
    throw appError("notFound", `词库尚未打开：${fileId ?? "(缺少 fileId)"}`, {
      hint: "请先调用 dict_open",
    });
  }
  return file;
}

function activeScheme(): SchemeInfo {
  return schemes.find((s) => s.isCurrent) ?? schemes[0];
}

function schemeById(id: string | undefined): SchemeInfo {
  if (!id) return activeScheme();
  const found = schemes.find((s) => s.schemaId === id);
  if (!found) {
    throw appError("schemeNotDetected", `未找到方案：${id}`, {
      hint: "请先调用 env_list_schemes 完成环境探测",
    });
  }
  return found;
}

/**
 * mock 命令入口。参数名与 Tauri 侧一致（camelCase；后端是 snake_case 参数，Tauri 自动转换）。
 * 命令语义见 `docs/ipc.md` §3。
 */

// ───────────────────────── 部署 / 备份 / 设置 的 mock 状态 ─────────────────────────

let mockDeployPhase: DeployState = {
  phase: "idle",
  startedAt: null,
  finishedAt: null,
};

export const mockDeployHistory: DeployResult[] = [];

function isMockDeploying(): boolean {
  return (
    mockDeployPhase.phase === "preparing" ||
    mockDeployPhase.phase === "running" ||
    mockDeployPhase.phase === "verifying"
  );
}

function setPhase(phase: DeployState["phase"], message?: string) {
  const now = Date.now();
  const started =
    phase === "preparing" ? now : mockDeployPhase.startedAt ?? now;
  mockDeployPhase = {
    phase,
    startedAt: started,
    finishedAt: ["success", "failed", "cancelled"].includes(phase) ? now : null,
    message,
    lastResult: mockDeployPhase.lastResult,
  };
  __setMockDeployState(mockDeployPhase);
}

/** 模拟一次部署：准备 → 运行 → 校验 → 成功。约 1.2s。 */
async function runMockDeploy(trigger: DeployTrigger = "manual") {
  const startedAt = Date.now();
  setPhase("preparing", "准备部署环境");
  await delay(200);
  setPhase("running", "触发 fcitx5 重新部署");
  await delay(700);
  setPhase("verifying", "校验 build/ 产物");
  await delay(300);

  const result: DeployResult = {
    trigger,
    success: true,
    finishedAt: Date.now(),
    durationMs: Date.now() - startedAt,
    outputVerified: true,
  };
  mockDeployHistory.unshift(result);
  if (mockDeployHistory.length > 20) mockDeployHistory.pop();

  mockDeployPhase = {
    phase: "success",
    startedAt,
    finishedAt: Date.now(),
    message: "部署完成",
    lastResult: result,
  };
  __setMockDeployState(mockDeployPhase);
}

export const mockBackups: BackupRef[] = [];

function mockCreateBackup(sourcePath: string, size: number): BackupRef {
  const createdAt = Date.now();
  const stamp = new Date(createdAt)
    .toISOString()
    .replace(/[-:T]/g, "")
    .slice(0, 14);
  const backup: BackupRef = {
    id: `mock-${createdAt}-${Math.random().toString(36).slice(2, 7)}`,
    path: `${sourcePath}_${stamp}.bak`,
    sourcePath,
    createdAt,
    size,
    reason: "beforeWrite",
  };
  mockBackups.unshift(backup);
  if (mockBackups.length > 30) mockBackups.pop();
  return backup;
}

// 造几条历史备份，便于备份页直接可看
for (const file of files.values()) {
  for (let i = 0; i < 3; i++) {
    const b = mockCreateBackup(file.ref.path, file.fingerprint.size);
    b.createdAt = Date.now() - i * 3600_000;
    b.reason = i === 0 ? "manual" : "beforeWrite";
  }
}

const DEFAULT_MOCK_SETTINGS: AppSettings = {
  userDir: RIME_DIR,
  frontend: "fcitx5Rime",
  managedFiles: [],
  autoDeployAfterSave: true,
  backup: {
    enabled: true,
    dir: null,
    keepCount: 30,
    scheduledEnabled: false,
    scheduledIntervalHours: 24,
  },
  theme: "system" as Theme,
  language: "zh-CN",
};

export let mockSettings: AppSettings = { ...DEFAULT_MOCK_SETTINGS };

export async function mockInvoke<T>(
  cmd: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  await delay(20);

  switch (cmd) {
    // §3.1 env_list_schemes
    case "env_list_schemes":
      return schemes.map((s) => ({ ...s })) as T;

    // §3.1 env_detect
    case "env_detect":
      return {
        frontends: [
          {
            kind: "fcitx5Rime",
            displayName: "fcitx5-rime",
            version: "5.1.13",
            librimeVersion: "1.16.1",
            userDir: RIME_DIR,
            sharedDir: "/usr/share/rime-data",
            deployCapability: { supported: true, method: "dbus-setconfig" },
          },
        ],
        active: {
          kind: "fcitx5Rime",
          displayName: "fcitx5-rime",
          version: "5.1.13",
          librimeVersion: "1.16.1",
          userDir: RIME_DIR,
          sharedDir: "/usr/share/rime-data",
          deployCapability: { supported: true, method: "dbus-setconfig" },
        },
      } as T;

    // §3.1 env_set_user_dir：校验目录并返回方案列表（与 Rust 实现一致）
    case "env_set_user_dir": {
      const path = String(args.path ?? "");
      // mock 只认自己模拟的那个用户目录；共享目录要明确拒绝（真实后端同样会拒绝）
      if (!path.includes("fcitx5/rime") && !path.includes("AppData")) {
        throw appError("userDirInvalid", `目录不是 Rime 用户目录：${path}`, {
          hint: "请选择 Rime 用户目录（例如 ~/.local/share/fcitx5/rime），而不是共享数据目录。",
        });
      }
      mockSettings.userDir = path;
      return schemes.map((x) => ({ ...x })) as T;
    }

    // §3.2 dict_open：返回 ref + total + fingerprint；重新打开即视为「重新加载」
    case "dict_open": {
      const file = requireFile(args.fileId as FileId | undefined);
      file.externallyModified = false; // 重新加载后指纹一致，可再次保存
      return {
        ref: { ...file.ref },
        total: file.entries.length,
        fingerprint: { ...file.fingerprint },
      } as T;
    }

    // §3.2 dict_query
    case "dict_query": {
      const query = args.query as EntryQuery;
      const file = requireFile(query?.fileId ?? undefined);
      return queryPage(file, {
        ...query,
        sort: query.sort ?? { field: "lineNo", desc: false },
        offset: query.offset ?? 0,
        limit: query.limit ?? 200,
      }) as T;
    }

    // §3.2 dict_groups
    case "dict_groups": {
      const file = requireFile(args.fileId as FileId | undefined);
      return groupCounts(file) as T;
    }

    // §3.2 dict_validate
    case "dict_validate": {
      const req = args.request as SaveRequest;
      const file = requireFile(req?.fileId);
      const errors = validateMock(file, req);
      return { ok: errors.length === 0, errors } as T;
    }

    // §3.2 dict_save：指纹不一致 → externalModified（不写盘、不自动重试）
    case "dict_save": {
      const req = args.request as SaveRequest;
      const file = requireFile(req?.fileId);
      if (file.externallyModified) {
        throw appError(
          "externalModified",
          "文件已被外部程序修改，本次保存已取消（未写入任何内容）",
          {
            hint: "请先重新加载文件，确认外部改动后再保存；工具不会自动覆盖。",
            detail: `当前指纹：mtime=${file.fingerprint.mtimeMs} size=${file.fingerprint.size}`,
          },
        );
      }
      const errors = validateMock(file, req);
      if (errors.length > 0) {
        throw appError("validationFailed", `校验未通过（${errors.length} 项问题），已取消保存`, {
          detail: errors
            .slice(0, 20)
            .map(
              (i) =>
                `${i.lineNo != null ? `第 ${i.lineNo} 行` : "—"} ${i.text ?? ""}/${i.entryCode ?? ""}：${i.message}`,
            )
            .join("\n"),
        });
      }
      const written = applySave(file, req);
      const backupSize = file.entries.length * 24 + 512;
      return {
        written,
        backup: {
          id: `bak_${file.version}`,
          path: `${file.ref.path}_20261126_161500.bak`,
          sourcePath: file.ref.path,
          createdAt: MOCK_EPOCH + file.version * 61_000,
          size: backupSize,
          reason: "beforeWrite",
        },
        fingerprint: { ...file.fingerprint },
        deploy: req.deployAfter ? mockDeploy("afterSave") : undefined,
      } as T;
    }

    // §3.3 suggest_codes
    case "suggest_codes": {
      const text = String(args.text ?? "").trim();
      if (!text) return [] as T;
      const scheme = schemeById(args.schemaId as string | undefined);
      const role = (args.role as FileRole | undefined) ?? "customPhrase";
      const exclude = new Set((args.excludeExisting as string[] | undefined) ?? []);
      return suggestionsForText(text, scheme, role, exclude) as T;
    }

    // §3.3 suggest_preview：静态预览（按权重 + 导入顺序排序）
    case "suggest_preview": {
      const code = String(args.code ?? "").toLowerCase();
      const text = args.text as string | undefined;
      const schemeId = args.schemaId as string | undefined;
      const file = [...files.values()].find(
        (f) => !schemeId || f.schemeId === schemeId,
      );
      if (!file) {
        return {
          mode: "unavailable",
          items: [],
          note: "mock 中没有可预览的词库",
        } as T;
      }
      const items = file.entries
        .filter((e) => e.key.code.startsWith(code))
        .sort((a, b) => (b.weight ?? 0) - (a.weight ?? 0))
        .slice(0, 9)
        .map((e) => ({
          text: e.key.text,
          comment: e.weight != null ? `权重 ${e.weight}` : undefined,
          weight: e.weight ?? undefined,
          isTarget: text ? e.key.text === text : false,
        }));
      return { mode: "static", items } as T;
    }

    // §3.5 deploy_status
    case "deploy_status":
      return {
        phase: "idle",
        startedAt: null,
        finishedAt: null,
        message: "部署状态",
      } as T;


    // ── §3.5 部署（mock：异步推进状态，便于 UI 观察进度） ──
    case "deploy_start": {
      if (isMockDeploying()) {
        return { accepted: false, merged: true } as T;
      }
      void runMockDeploy(args.trigger as DeployTrigger | undefined);
      return { accepted: true, merged: false } as T;
    }

    case "deploy_history":
      return mockDeployHistory.slice(0, Number(args.limit ?? 10)) as T;

    case "deploy_wait": {
      const deadline = Date.now() + Number(args.timeoutMs ?? 30000);
      while (isMockDeploying() && Date.now() < deadline) {
        await delay(50);
      }
      return !isMockDeploying() as T;
    }

    // ── §3.6 备份 ──
    case "backup_list":
      return mockBackups.map((b) => ({ ...b })) as T;

    case "backup_now": {
      const created: BackupRef[] = [];
      for (const file of files.values()) {
        created.push(mockCreateBackup(file.ref.path, file.fingerprint.size));
      }
      return created as T;
    }

    case "backup_restore": {
      const id = args.backupId as string;
      const backup = mockBackups.find((b) => b.id === id);
      if (!backup) throw appError("notFound", `备份不存在：${id}`);
      const file = [...files.values()].find((f) => f.ref.path === backup.sourcePath);
      const entries = file?.entries.length ?? 0;
      const diff: FileDiff = {
        sourcePath: backup.sourcePath,
        added: [],
        removed: [],
        summary: { added: 0, removed: 0, modified: 1, unchanged: Math.max(0, entries - 1) },
      };
      if (args.dryRun) return { diff } as T;
      return {
        diff,
        result: {
          written: backup.size,
          backup: mockCreateBackup(backup.sourcePath, backup.size),
          fingerprint: {
            mtimeMs: Date.now(),
            size: backup.size,
            sha256: backup.id.repeat(8).slice(0, 64),
          },
        },
      } as T;
    }

    case "backup_delete": {
      const ids = new Set((args.backupIds as string[]) ?? []);
      const before = mockBackups.length;
      for (let i = mockBackups.length - 1; i >= 0; i--) {
        if (ids.has(mockBackups[i].id)) mockBackups.splice(i, 1);
      }
      return { deleted: before - mockBackups.length } as T;
    }

    case "history_list":
      return [] as T;

    // ── §3.7 设置 ──
    case "settings_get":
      return { ...mockSettings } as T;

    case "settings_update": {
      const patch = args.patch as Partial<AppSettings> | undefined;
      if (patch) Object.assign(mockSettings, patch);
      return { ...mockSettings } as T;
    }

    case "settings_open_dir":
      // mock 里没有文件管理器，明确告知而不是假装成功
      return {
        ok: false,
        path: String(args.which ?? ""),
        note: "浏览器环境无法打开系统文件管理器",
      } as T;

    case "settings_reset":
      mockSettings = { ...DEFAULT_MOCK_SETTINGS };
      return { ...mockSettings } as T;

    default:
      throw appError("internal", `mock 未实现的命令：${cmd}`, {
        hint: "请检查命令名，或改用真实 Tauri 宿主调试。",
      });
  }
}