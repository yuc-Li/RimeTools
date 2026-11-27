//! 汉字 → 拼音数据源。
//!
//! # 数据来源与许可
//!
//! `BUILTIN_PINYIN` 是**手工录入的常用字读音事实**（字→普通话读音）。读音本身
//! 是语言事实、不构成受版权保护的表达，本表也**不从任何 GPL 码表/词库复制**
//! （凇鹤 rime-crane 为 GPL-3.0，其码表只在运行时解析，见 PRD §4 许可边界）。
//!
//! 内置表只覆盖常用字，用于「开箱可用」；生产环境应通过 [`PinyinSource`] 注入更
//! 完整的数据源（可选：Unicode `Unihan` 的 `kMandarin` 字段，Unicode License；
//! 或运行时从用户本地方案码表反查），本 crate 只依赖 trait，不绑定具体来源。

use crate::tones;
use std::collections::HashMap;

/// 一个读音：无调音节 + 声调（1–4；`None`/5 = 轻声）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinyinReading {
    pub syllable: String,
    pub tone: Option<u8>,
}

impl PinyinReading {
    pub fn new(syllable: impl Into<String>, tone: Option<u8>) -> Self {
        PinyinReading {
            syllable: syllable.into(),
            tone,
        }
    }

    /// 解析 `"bao4"` / `"de5"` / `"de"` 形式。
    pub fn parse(text: &str) -> Option<PinyinReading> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let (syllable, tone) = match text.chars().last() {
            Some(c) if c.is_ascii_digit() => {
                let digit = c.to_digit(10)? as u8;
                (
                    &text[..text.len() - c.len_utf8()],
                    Some(digit).filter(|t| (1..=5).contains(t)),
                )
            }
            _ => (text, None),
        };
        if syllable.is_empty() {
            return None;
        }
        Some(PinyinReading::new(syllable, tone))
    }

    /// 带调显示（`xing2 → xíng`），用于 `CodeSuggestion::reading`。
    pub fn display(&self) -> String {
        tones::with_tone(&self.syllable, self.tone)
    }
}

/// 汉字读音来源（可注入：内置表 / Unihan / 本地码表反查）。
pub trait PinyinSource: Send + Sync {
    /// 返回该字的全部读音，**第一个为主读音**（与码表首选一致）。
    fn readings(&self, ch: char) -> Vec<PinyinReading>;

    /// 是否认识该字。
    fn contains(&self, ch: char) -> bool {
        !self.readings(ch).is_empty()
    }
}

/// 是否为汉字（CJK 统一表意文字，含扩展 A 与兼容区）。
pub fn is_han(ch: char) -> bool {
    matches!(ch as u32,
        0x3400..=0x4DBF |      // 扩展 A
        0x4E00..=0x9FFF |      // 基本区
        0xF900..=0xFAFF |      // 兼容表意文字
        0x20000..=0x2FA1F      // 扩展 B–F 与兼容补充
    )
}

/// 内置常用字拼音表。
#[derive(Debug, Clone, Default)]
pub struct BuiltinPinyin {
    map: HashMap<char, Vec<PinyinReading>>,
}

impl BuiltinPinyin {
    /// 用内置表构造。
    pub fn new() -> Self {
        Self::parse(BUILTIN_PINYIN)
    }

    /// 解析「字 读音 读音…」逐行格式（`#` 开头为注释）。
    pub fn parse(table: &str) -> Self {
        let mut map: HashMap<char, Vec<PinyinReading>> = HashMap::new();
        for line in table.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let Some(word) = parts.next() else { continue };
            let mut chars = word.chars();
            let (Some(ch), None) = (chars.next(), chars.next()) else {
                tracing::warn!(line, "拼音表条目首列应为单个汉字，跳过");
                continue;
            };
            let readings: Vec<PinyinReading> = parts.filter_map(PinyinReading::parse).collect();
            if readings.is_empty() {
                tracing::warn!(line, "拼音表条目缺少读音，跳过");
                continue;
            }
            if map.insert(ch, readings).is_some() {
                tracing::warn!(ch = %ch, "拼音表条目重复，后出现的覆盖前者");
            }
        }
        BuiltinPinyin { map }
    }

    /// 表内字数。
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl PinyinSource for BuiltinPinyin {
    fn readings(&self, ch: char) -> Vec<PinyinReading> {
        self.map.get(&ch).cloned().unwrap_or_default()
    }
}

/// 便于上层注入外部数据：`HashMap<汉字, ["bao4", "pao2"]>`。
impl PinyinSource for HashMap<char, Vec<String>> {
    fn readings(&self, ch: char) -> Vec<PinyinReading> {
        self.get(&ch)
            .map(|list| {
                list.iter()
                    .filter_map(|s| PinyinReading::parse(s))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// 内置常用字拼音表（手工录入的读音事实，见模块注释的许可说明）。
///
/// 格式：`汉字 读音[ 读音…]`，读音带声调数字（1–4；5 或省略 = 轻声），第一个为主读音。
/// 多音字列多个读音，供多候选输出使用。
const BUILTIN_PINYIN: &str = r#"
# —— 示例词条（回归用例覆盖）——
中 zhong1 zhong4
州 zhou1
韵 yun4
报 bao4
关 guan1
单 dan1 shan4 chan2
美 mei3
团 tuan2
一 yi1
会 hui4 kuai4
儿 er2
骑 qi2
行 xing2 hang2
标 biao1
识 shi2 zhi4
精 jing1
力 li4
重 zhong4 chong2
置 zhi4
摸 mo1
鱼 yu2
张 zhang1
三 san1
李 li3
雨 yu3
辰 chen2
程 cheng2
狗 gou3
咬 yao3
吕 lv3
洞 dong4
宾 bin1
临 lin2
时 shi2
抱 bao4
佛 fo2 fu2
脚 jiao3 jue2
十 shi2
年 nian2
生 sheng1
死 si3
两 liang3
茫 mang2
个 ge4
天 tian1
上 shang4
下 xia4
地 di4 de5
鼻 bi2
塞 sai1 se4
痞 pi3
登 deng1
陆 lu4 liu4
跌 die1
价 jia4 jie4
薄 bo2 bao2 bo4
伯 bo2 bai3
堡 bao3 bu3
柏 bai3 bo2
# —— 单字/码表常用 ——
安 an1
按 an4
案 an4
爱 ai4
阿 a1 e1
奥 ao4
岸 an4
埃 ai1
艾 ai4
碍 ai4
癌 ai2
昂 ang2
暗 an4
傲 ao4
早 zao3
晚 wan3
今 jin1
明 ming2
昨 zuo2
前 qian2
后 hou4
左 zuo3
右 you4
高 gao1
低 di1
长 chang2 zhang3
短 duan3
大 da4
小 xiao3
多 duo1
少 shao3 shao4
好 hao3 hao4
新 xin1
旧 jiu4
快 kuai4
慢 man4
冷 leng3
热 re4
远 yuan3
近 jin4
香 xiang1
甜 tian2
苦 ku3
辣 la4
酸 suan1
咸 xian2
东 dong1
南 nan2
西 xi1
北 bei3
春 chun1
夏 xia4
秋 qiu1
冬 dong1
# —— 人称/高频虚词 ——
中 zhong1 zhong4
国 guo2
人 ren2
民 min2
共 gong4
和 he2 huo4
的 de5 di2
是 shi4
不 bu4
了 le5 liao3
在 zai4
有 you3
我 wo3
你 ni3
他 ta1
她 ta1
它 ta1
们 men5
这 zhe4
那 na4
哪 na3
谁 shui2 shei2
说 shuo1
话 hua4
书 shu1
写 xie3
读 du2
听 ting1
看 kan4
走 zou3
跑 pao3
吃 chi1
喝 he1
睡 shui4
觉 jiao4 jue2
# —— 工作台/工具语境 ——
工 gong1
作 zuo4
学 xue2
校 xiao4 jiao4
老 lao3
师 shi1
同 tong2
朋 peng2
友 you3
家 jia1
门 men2
窗 chuang1
户 hu4
车 che1
路 lu4
桥 qiao2
数 shu4
据 ju4
表 biao3
格 ge2
查 cha2
找 zhao3
删 shan1
除 chu2
修 xiu1
改 gai3
添 tian1
加 jia1
保 bao3
存 cun2
退 tui4
回 hui2
首 shou3
页 ye4
排 pai2
序 xu4
移 yi2
动 dong4
制 zhi4
剪 jian3
粘 zhan1
贴 tie1
打 da3
印 yin4
导 dao3
出 chu1
入 ru4
法 fa3
双 shuang1
拼 pin1
音 yin1
形 xing2
词 ci2
库 ku4
管 guan3
理 li3
具 ju4
编 bian1
辑 ji2
档 dang4
备 bei4
份 fen4
恢 hui1
复 fu4
自 zi4
定 ding4
义 yi4
准 zhun3
确 que4
误 wu4
客 ke4
名 ming2
字 zi4
汉 han4
语 yu3
英 ying1
文 wen2
间 jian1 jian4
分 fen1 fen4
秒 miao3
点 dian3
号 hao4
码 ma3
输 shu1
思 si1
想 xiang3
知 zhi1
道 dao4
问 wen4
答 da2
请 qing3
谢 xie4
对 dui4
起 qi3
再 zai4
见 jian4
# —— 自然/生活 ——
山 shan1
水 shui3
火 huo3
土 tu3
木 mu4
金 jin1
日 ri4
月 yue4
星 xing1
云 yun2
风 feng1
雪 xue3
雷 lei2
电 dian4
花 hua1
草 cao3
树 shu4
叶 ye4
果 guo3
米 mi3
面 mian4
饭 fan4
菜 cai4
肉 rou4
蛋 dan4
奶 nai3
茶 cha2
酒 jiu3
糖 tang2
盐 yan2
油 you2
醋 cu4
猫 mao1
鸟 niao3
虫 chong2
虎 hu3
兔 tu4
龙 long2
蛇 she2
马 ma3
羊 yang2
猴 hou2
鸡 ji1
猪 zhu1
鼠 shu3
牛 niu2
# —— 用户码表词条覆盖（xhup.user / xhup.user.top 回归）——
而 er2
应 ying1 ying4
反 fan3
成 cheng2
为 wei4 wei2
传 chuan2 zhuan4
统 tong3
即 ji2
可 ke3 ke4
计 ji4
算 suan4
极 ji2
其 qi2
联 lian2
系 xi4 ji4
每 mei3
周 zhou1
丽 li4
期 qi1
任 ren4 ren2
务 wu4
台 tai2
湾 wan1
平 ping2
试 shi4
神 shen2
似 si4 shi4
只 zhi3 zhi1
能 neng2
正 zheng4 zheng1
小 xiao3
现 xian4
直 zhi2
片 pian4 pian1
线 xian4
进 jin4
口 kou3
商 shang1
到 dao4
简 jian3
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_readings_with_tone() {
        assert_eq!(
            PinyinReading::parse("bao4"),
            Some(PinyinReading::new("bao", Some(4)))
        );
        assert_eq!(
            PinyinReading::parse("de5"),
            Some(PinyinReading::new("de", Some(5)))
        );
        assert_eq!(
            PinyinReading::parse("de"),
            Some(PinyinReading::new("de", None))
        );
        assert_eq!(PinyinReading::parse(""), None);
        assert_eq!(PinyinReading::parse("4"), None);
    }

    #[test]
    fn display_uses_tone_marks() {
        assert_eq!(PinyinReading::parse("xing2").unwrap().display(), "xíng");
        assert_eq!(PinyinReading::parse("lv3").unwrap().display(), "lǚ");
    }

    #[test]
    fn builtin_covers_regression_chars() {
        let pinyin = BuiltinPinyin::new();
        assert!(pinyin.len() > 150, "内置表过小：{}", pinyin.len());
        for ch in "报关单美团一会儿骑行标识精力重置摸鱼狗咬吕洞宾临时抱佛脚十年生死两茫茫一个天上一个地下鼻塞地痞登陆跌价".chars() {
            assert!(pinyin.contains(ch), "内置拼音表缺少「{ch}」");
        }
    }

    #[test]
    fn builtin_has_polyphonic_readings() {
        let pinyin = BuiltinPinyin::new();
        let hang = pinyin.readings('行');
        assert_eq!(hang.len(), 2, "行应有 xing/hang 两个读音");
        assert_eq!(
            hang[0].syllable, "xing",
            "主读音应为 xing（与码表 qixk 一致）"
        );
        assert_eq!(hang[1].syllable, "hang");
        // 主读音必须排在最前
        assert_eq!(pinyin.readings('会')[0].syllable, "hui");
        assert_eq!(pinyin.readings('单')[0].syllable, "dan");
    }

    #[test]
    fn every_builtin_syllable_is_valid() {
        let pinyin = BuiltinPinyin::new();
        for (ch, readings) in &pinyin.map {
            for reading in readings {
                assert!(
                    crate::syllable::is_syllable(&reading.syllable),
                    "「{ch}」的读音 {} 不是合法音节",
                    reading.syllable
                );
            }
        }
    }

    #[test]
    fn hashmap_source_is_injectable() {
        let mut map: HashMap<char, Vec<String>> = HashMap::new();
        map.insert('行', vec!["hang2".into(), "xing2".into()]);
        let readings = map.readings('行');
        assert_eq!(readings[0].syllable, "hang");
        assert_eq!(readings.len(), 2);
        assert!(map.contains('行'));
        assert!(!map.contains('中'));
    }

    #[test]
    fn han_detection() {
        assert!(is_han('中'));
        assert!(is_han('㐀'));
        assert!(!is_han('a'));
        assert!(!is_han('，'));
    }
}
