/**
 * 设置页的中文文案与展示辅助（FR-1.8 / FR-3.1 / FR-3.5 的「格式与权重语义」说明）。
 *
 * 只做纯展示映射，不含状态；同一份文案也用于「这个方案会改哪些文件」总览，
 * 保证用户在各处看到的说法一致。
 */

import type {
  ColumnKind,
  FileFormat,
  FileRole,
  FrontendKind,
  ManagedFileRef,
  SchemeInfo,
  Theme,
  TranslatorKind,
} from "../../ipc/types";

export const FRONTEND_LABEL: Record<FrontendKind, string> = {
  weasel: "小狼毫",
  fcitx5Rime: "fcitx5-rime",
  ibusRime: "ibus-rime",
  unknown: "未知前端",
};

export const TRANSLATOR_LABEL: Record<TranslatorKind, string> = {
  script: "拼音 / 双拼（script_translator）",
  table: "音形 / 码表（table_translator）",
  mixed: "混合翻译器（mixed）",
  unknown: "未知翻译器",
};

export const ROLE_LABEL: Record<FileRole, string> = {
  customPhrase: "自定义短语",
  userDict: "用户码表",
  userTopDict: "置顶码表",
  personalDict: "个人词典",
  readOnlyDict: "系统词典（只读）",
};

export const FORMAT_LABEL: Record<FileFormat, string> = {
  customPhrase: "custom_phrase · txt",
  rimeDictYaml: "Rime 词典 · yaml",
  plainTxt: "纯文本 · txt",
  csv: "CSV",
};

const COLUMN_LABEL: Record<ColumnKind, string> = {
  text: "文本",
  code: "编码",
  weight: "权重",
};

const CN_NUMBER = ["零", "一", "二", "三", "四", "五"];

/** 列风格，如「三列：文本 / 编码 / 权重」 */
export function columnStyle(columns: ColumnKind[]): string {
  if (columns.length === 0) return "列风格未识别";
  const count = columns.length === 2 ? "两" : CN_NUMBER[columns.length] ?? String(columns.length);
  return `${count}列：${columns.map((column) => COLUMN_LABEL[column]).join(" / ")}`;
}

/** 取路径末段（界面上显示短名，完整路径放在 title / 单独一行） */
export function baseName(path: string): string {
  const parts = path.split(/[/\\]/);
  return parts[parts.length - 1] || path;
}

/**
 * 受管文件的写入语义（FR-3.5）。
 * 这是设置页要“一眼看懂”的核心：工具写哪几列、权重怎么理解、会不会动系统文件。
 */
export function roleWriteNote(file: ManagedFileRef): string {
  switch (file.role) {
    case "customPhrase":
      return "写入 文本 ⇥ 编码 ⇥ 权重（权重可缺省）。权重是「同编码内排序」，越大越靠前；custom_phrase 整体以 initial_quality: 99 置顶。";
    case "userDict":
      return "写入 文本 ⇥ 编码（两列，无权重）。排序由导入顺序与 .top 置顶表决定；工具写入时保持两列格式。";
    case "userTopDict":
      return "置顶码表：文本 ⇥ 编码（两列）。同一编码的候选会优先于普通用户码表展示。";
    case "personalDict":
      return "个人词典：文本 ⇥ 编码 ⇥ 权重（权重为词频，相对值）；工具保持文件原有列风格。";
    case "readOnlyDict":
      return "系统 / 方案自带词典只读，工具永不写入。";
    default:
      return "工具保持文件原有列风格。";
  }
}

export type RoleTone = "accent" | "info" | "warn" | "muted";

export function roleTone(role: FileRole): RoleTone {
  switch (role) {
    case "customPhrase":
      return "accent";
    case "userDict":
    case "userTopDict":
      return "info";
    case "readOnlyDict":
      return "warn";
    default:
      return "muted";
  }
}

/** 部署方式的中文说明（`DeployCapability.method`，契约 §2） */
export function deployMethodLabel(method: string): string {
  switch (method) {
    case "dbus-setconfig":
      return "D-Bus SetConfig（fcitx5 重新部署）";
    case "rime-deployer":
      return "rime_deployer --build（预构建 build/）";
    case "weasel-deployer":
      return "WeaselDeployer.exe /deploy";
    case "none":
      return "无自动部署方式";
    default:
      return method || "未知方式";
  }
}

/** 不支持自动部署时的可操作兜底（FR-5.4 ③） */
export function deployFallbackNote(): string {
  return "请保存后手动部署：rime_deployer --build <用户目录> /usr/share/rime-data <用户目录>/build，或重启输入法前端。";
}

export interface ManagedFileGroup {
  file: ManagedFileRef;
  /** 引用该文件的方案（同一物理文件可能被多个方案挂载） */
  schemes: SchemeInfo[];
}

/**
 * 按物理路径归并受管文件，得到「工具会改哪些文件」的全景。
 * 系统只读词典不参与写入，也不计入总览。
 */
export function groupManagedFiles(schemes: SchemeInfo[]): ManagedFileGroup[] {
  const groups = new Map<string, ManagedFileGroup>();
  for (const scheme of schemes) {
    for (const file of scheme.managedFiles) {
      if (file.role === "readOnlyDict") continue;
      const existing = groups.get(file.path);
      if (existing) {
        if (!existing.schemes.some((item) => item.schemaId === scheme.schemaId)) {
          existing.schemes.push(scheme);
        }
      } else {
        groups.set(file.path, { file, schemes: [scheme] });
      }
    }
  }
  return [...groups.values()];
}

export interface ThemeOption {
  value: Theme;
  label: string;
  description: string;
}

export const THEME_OPTIONS: ThemeOption[] = [
  { value: "system", label: "跟随系统", description: "默认；随系统深浅色切换" },
  { value: "light", label: "浅色", description: "始终使用浅色界面" },
  { value: "dark", label: "深色", description: "始终使用深色界面" },
];

export function themeLabel(theme: Theme): string {
  return THEME_OPTIONS.find((option) => option.value === theme)?.label ?? theme;
}