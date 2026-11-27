//! 合法全拼音节表与声韵母拆分。
//!
//! **数据来源与许可**：音节表是「普通话有哪些音节」这一语言事实的枚举，
//! 由本项目按公开的拼音教学资料手工整理，**不复制任何第三方码表或词库内容**
//! （凇鹤/rime-crane 为 GPL-3.0，其码表只在运行时解析、不随工具分发，见 PRD §4）。
//! 覆盖普通话常用音节（408 音节体系）；极少数叹词/方言音节（`m`、`n`、`ng`、
//! `hm`、`hng`）未收录，遇到时相关字不会产出建议，由上层提示手动填写。

/// 声母表（`zh`/`ch`/`sh` 为双字母声母；`y`/`w` 视作声母处理，与双拼方案一致）。
pub const INITIALS: &[&str] = &[
    "zh", "ch", "sh", "b", "p", "m", "f", "d", "t", "n", "l", "g", "k", "h", "j", "q", "x", "r",
    "z", "c", "s", "y", "w",
];

/// 韵母表（`ü` 统一写作 `v`，与 Rime 双拼方案的写法一致；`ue` 仅出现在
/// `jue/que/xue/yue`，“üe”写作 `ve` 出现在 `lve/nve`）。
pub const FINALS: &[&str] = &[
    "a", "o", "e", "i", "u", "v", "ai", "ei", "ao", "ou", "an", "en", "ang", "eng", "er", "in",
    "ing", "un", "ong", "ia", "iao", "ian", "iang", "ie", "iu", "iong", "ua", "uo", "uai", "uan",
    "uang", "ui", "ue", "ve",
];

/// 全拼音节表（零声母音节排在最前，其余按声母分组）。
pub const SYLLABLES: &[&str] = &[
    // 零声母
    "a", "ai", "an", "ang", "ao", "e", "ei", "en", "eng", "er", "o", "ou", // b
    "ba", "bai", "ban", "bang", "bao", "bei", "ben", "beng", "bi", "bian", "biao", "bie", "bin",
    "bing", "bo", "bu", // p
    "pa", "pai", "pan", "pang", "pao", "pei", "pen", "peng", "pi", "pian", "piao", "pie", "pin",
    "ping", "po", "pou", "pu", // m
    "ma", "mai", "man", "mang", "mao", "me", "mei", "men", "meng", "mi", "mian", "miao", "mie",
    "min", "ming", "miu", "mo", "mou", "mu", // f
    "fa", "fan", "fang", "fei", "fen", "feng", "fo", "fou", "fu", // d
    "da", "dai", "dan", "dang", "dao", "de", "dei", "den", "deng", "di", "dia", "dian", "diao",
    "die", "ding", "diu", "dong", "dou", "du", "duan", "dui", "dun", "duo", // t
    "ta", "tai", "tan", "tang", "tao", "te", "teng", "ti", "tian", "tiao", "tie", "ting", "tong",
    "tou", "tu", "tuan", "tui", "tun", "tuo", // n
    "na", "nai", "nan", "nang", "nao", "ne", "nei", "nen", "neng", "ni", "nian", "niang", "niao",
    "nie", "nin", "ning", "niu", "nong", "nou", "nu", "nuan", "nuo", "nv", "nve", // l
    "la", "lai", "lan", "lang", "lao", "le", "lei", "leng", "li", "lia", "lian", "liang", "liao",
    "lie", "lin", "ling", "liu", "lo", "long", "lou", "lu", "luan", "lun", "luo", "lv", "lve",
    // g
    "ga", "gai", "gan", "gang", "gao", "ge", "gei", "gen", "geng", "gong", "gou", "gu", "gua",
    "guai", "guan", "guang", "gui", "gun", "guo", // k
    "ka", "kai", "kan", "kang", "kao", "ke", "ken", "keng", "kong", "kou", "ku", "kua", "kuai",
    "kuan", "kuang", "kui", "kun", "kuo", // h
    "ha", "hai", "han", "hang", "hao", "he", "hei", "hen", "heng", "hong", "hou", "hu", "hua",
    "huai", "huan", "huang", "hui", "hun", "huo", // j
    "ji", "jia", "jian", "jiang", "jiao", "jie", "jin", "jing", "jiong", "jiu", "ju", "juan",
    "jue", "jun", // q
    "qi", "qia", "qian", "qiang", "qiao", "qie", "qin", "qing", "qiong", "qiu", "qu", "quan",
    "que", "qun", // x
    "xi", "xia", "xian", "xiang", "xiao", "xie", "xin", "xing", "xiong", "xiu", "xu", "xuan",
    "xue", "xun", // zh
    "zha", "zhai", "zhan", "zhang", "zhao", "zhe", "zhei", "zhen", "zheng", "zhi", "zhong", "zhou",
    "zhu", "zhua", "zhuai", "zhuan", "zhuang", "zhui", "zhun", "zhuo", // ch
    "cha", "chai", "chan", "chang", "chao", "che", "chen", "cheng", "chi", "chong", "chou", "chu",
    "chua", "chuai", "chuan", "chuang", "chui", "chun", "chuo", // sh
    "sha", "shai", "shan", "shang", "shao", "she", "shei", "shen", "sheng", "shi", "shou", "shu",
    "shua", "shuai", "shuan", "shuang", "shui", "shun", "shuo", // r
    "ran", "rang", "rao", "re", "ren", "reng", "ri", "rong", "rou", "ru", "rua", "ruan", "rui",
    "run", "ruo", // z
    "za", "zai", "zan", "zang", "zao", "ze", "zei", "zen", "zeng", "zi", "zong", "zou", "zu",
    "zuan", "zui", "zun", "zuo", // c
    "ca", "cai", "can", "cang", "cao", "ce", "cen", "ceng", "ci", "cong", "cou", "cu", "cuan",
    "cui", "cun", "cuo", // s
    "sa", "sai", "san", "sang", "sao", "se", "sen", "seng", "si", "song", "sou", "su", "suan",
    "sui", "sun", "suo", // y
    "ya", "yan", "yang", "yao", "ye", "yi", "yin", "ying", "yo", "yong", "you", "yu", "yuan",
    "yue", "yun", // w
    "wa", "wai", "wan", "wang", "wei", "wen", "weng", "wo", "wu",
];

/// 判断是否为合法全拼音节（大小写敏感，要求小写）。
pub fn is_syllable(s: &str) -> bool {
    static INDEX: std::sync::OnceLock<std::collections::HashSet<&'static str>> =
        std::sync::OnceLock::new();
    let index = INDEX.get_or_init(|| SYLLABLES.iter().copied().collect());
    index.contains(s)
}

/// 拆分声母与韵母：`zhong → ("zh", "ong")`，零声母音节返回 `("", 原串)`。
pub fn split_initial(syllable: &str) -> (&str, &str) {
    for initial in INITIALS {
        if let Some(rest) = syllable.strip_prefix(initial) {
            // 声母不能把韵母吃空；`zhi`/`chi`/`shi` 的韵母是 `i`，仍成立。
            if !rest.is_empty() {
                return (initial, rest);
            }
        }
    }
    ("", syllable)
}

/// 取韵母部分。
pub fn final_of(syllable: &str) -> &str {
    split_initial(syllable).1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 音节表内不得有重复（表按声母分组维护，因此不要求升序）。
    #[test]
    fn syllable_table_has_no_duplicates() {
        let set: std::collections::HashSet<_> = SYLLABLES.iter().copied().collect();
        assert_eq!(set.len(), SYLLABLES.len(), "音节表有重复项");
        assert!(
            SYLLABLES.len() > 380,
            "音节表过短，仅 {} 条",
            SYLLABLES.len()
        );
    }

    /// 每个音节都能被拆成「合法声母 + 合法韵母」，防止表里混入错字。
    #[test]
    fn every_syllable_splits_into_initial_and_final() {
        for s in SYLLABLES {
            let (initial, fin) = split_initial(s);
            assert!(
                INITIALS.contains(&initial) || initial.is_empty(),
                "{s} 的声母 {initial:?} 不在声母表"
            );
            assert!(FINALS.contains(&fin), "{s} 的韵母 {fin:?} 不在韵母表");
        }
    }

    #[test]
    fn covers_common_syllables() {
        for s in [
            "zhong", "mei", "tuan", "xing", "shuang", "nv", "lve", "ang", "an", "a", "chong",
            "zhou", "yun",
        ] {
            assert!(is_syllable(s), "{s} 应在音节表中");
        }
        assert!(!is_syllable("xx"));
        assert!(!is_syllable("zzz"));
    }

    #[test]
    fn split_initial_examples() {
        assert_eq!(split_initial("zhong"), ("zh", "ong"));
        assert_eq!(split_initial("shuang"), ("sh", "uang"));
        assert_eq!(split_initial("ang"), ("", "ang"));
        assert_eq!(split_initial("a"), ("", "a"));
        assert_eq!(split_initial("nv"), ("n", "v"));
        assert_eq!(split_initial("lve"), ("l", "ve"));
        assert_eq!(split_initial("ju"), ("j", "u"));
        assert_eq!(final_of("xing"), "ing");
    }
}
