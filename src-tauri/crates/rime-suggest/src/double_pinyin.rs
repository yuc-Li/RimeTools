//! 双拼映射：对合法全拼音节表求值 `speller/algebra`，得到「全拼 → 双拼码」。
//!
//! 依据实测结论：双拼映射**不需要
//! 内置码表**，运行时解析用户方案的 algebra 即可。这里额外内置了一张
//! **小鹤双拼键位事实表**作为兜底（PRD R7 风险项），仅在方案没有 algebra 或
//! algebra 无法解析时启用；它只是键位事实（声母/韵母 → 键位），不含任何码表词条。

use crate::algebra::{Algebra, AlgebraError};
use crate::syllable::{self, SYLLABLES};
use rime_core::types::AlgebraSpec;
use std::collections::HashMap;

/// 一个音节的全部可用拼写。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyllableCodes {
    /// 主码（`xform` 链结果，供「词码」合成使用）
    pub primary: String,
    /// 派生/简拼变体（可选拼写，不影响主码）
    pub variants: Vec<String>,
}

/// 双拼方案映射表。
#[derive(Debug, Clone)]
pub struct DoublePinyin {
    alphabet: String,
    map: HashMap<&'static str, SyllableCodes>,
    /// 表外音节的兜底求值（来自同一套 algebra）
    algebra: Option<Algebra>,
}

impl DoublePinyin {
    /// 用 algebra 对音节表逐条求值，建立映射。
    ///
    /// 求值失败/结果不在 `alphabet` 内的音节会被跳过（`tracing::warn`），
    /// 以免产出含圈字母中间态的垃圾编码。
    pub fn from_algebra(algebra: Algebra, alphabet: impl Into<String>) -> Self {
        let alphabet = alphabet.into();
        let mut map = HashMap::with_capacity(SYLLABLES.len());
        for &syllable in SYLLABLES {
            let evaluation = algebra.evaluate(syllable);
            let Some(primary) = evaluation.primary else {
                tracing::warn!(syllable, "algebra 求值后主码为空（被 erase？），跳过");
                continue;
            };
            let bad: Vec<char> = primary.chars().filter(|c| !alphabet.contains(*c)).collect();
            if !bad.is_empty() {
                tracing::warn!(
                    syllable,
                    primary,
                    invalid = ?bad,
                    "双拼码含 alphabet 之外的字符（是否漏了 xlit？），跳过"
                );
                continue;
            }
            let mut variants: Vec<String> = Vec::new();
            for code in evaluation
                .derived
                .into_iter()
                .chain(evaluation.abbreviations)
            {
                if code == primary || variants.contains(&code) {
                    continue;
                }
                if code.chars().all(|c| alphabet.contains(c)) {
                    variants.push(code);
                }
            }
            map.insert(syllable, SyllableCodes { primary, variants });
        }
        DoublePinyin {
            alphabet,
            map,
            algebra: Some(algebra),
        }
    }

    /// 从 `rime_core::types::AlgebraSpec`（方案解析结果）建立映射。
    pub fn from_spec(spec: &AlgebraSpec) -> Result<Self, AlgebraError> {
        // 未知计算类型只告警跳过：用户方案可能带本工具尚未支持的扩展规则。
        Ok(DoublePinyin::from_algebra(
            Algebra::parse_lenient(&spec.rules),
            spec.alphabet.clone(),
        ))
    }

    /// 为方案选择映射：有 algebra 用它求值，否则退回内置小鹤键位表。
    ///
    /// 这是上层（词库工作台）最常用的入口。
    pub fn for_scheme(spec: Option<&AlgebraSpec>) -> Self {
        match spec {
            Some(spec) if !spec.rules.is_empty() => {
                let algebra = Algebra::parse_lenient(&spec.rules);
                if algebra.is_empty() {
                    tracing::warn!("方案 algebra 全部无法解析，退回内置小鹤双拼键位表");
                    return DoublePinyin::flypy();
                }
                DoublePinyin::from_algebra(algebra, spec.alphabet.clone())
            }
            _ => {
                tracing::info!("方案未提供 speller/algebra，使用内置小鹤双拼键位表");
                DoublePinyin::flypy()
            }
        }
    }

    /// 内置小鹤双拼（flypy）键位事实表兜底。
    pub fn flypy() -> Self {
        let mut map = HashMap::with_capacity(SYLLABLES.len());
        for &syllable in SYLLABLES {
            if let Some(code) = flypy_code(syllable) {
                map.insert(
                    syllable,
                    SyllableCodes {
                        primary: code,
                        variants: Vec::new(),
                    },
                );
            }
        }
        DoublePinyin {
            alphabet: "abcdefghijklmnopqrstuvwxyz".to_owned(),
            map,
            algebra: None,
        }
    }

    /// 方案字母表（`speller/alphabet`）。
    pub fn alphabet(&self) -> &str {
        &self.alphabet
    }

    /// 已建立映射的音节数（可用于健康检查/自检）。
    pub fn syllable_count(&self) -> usize {
        self.map.len()
    }

    /// 某音节是否已有映射。
    pub fn has_syllable(&self, syllable: &str) -> bool {
        self.map.contains_key(syllable)
    }

    /// 全拼音节 → 双拼主码。表外音节走 algebra 兜底求值。
    pub fn code_for(&self, syllable: &str) -> Option<String> {
        if let Some(codes) = self.map.get(syllable) {
            return Some(codes.primary.clone());
        }
        let algebra = self.algebra.as_ref()?;
        let evaluation = algebra.evaluate(syllable);
        evaluation.primary.filter(|code| self.is_valid_code(code))
    }

    /// 全拼音节 → 全部拼写（主码在前，派生变体在后）。
    pub fn codes_for(&self, syllable: &str) -> Vec<String> {
        match self.map.get(syllable) {
            Some(codes) => {
                let mut out = Vec::with_capacity(codes.variants.len() + 1);
                out.push(codes.primary.clone());
                out.extend(codes.variants.iter().cloned());
                out
            }
            None => self.code_for(syllable).into_iter().collect(),
        }
    }

    /// 双拼主码的首键（简拼/三字词码用）。
    pub fn first_key(&self, syllable: &str) -> Option<char> {
        self.code_for(syllable)?.chars().next()
    }

    /// 编码是否全部落在方案字母表内。
    pub fn is_valid_code(&self, code: &str) -> bool {
        !code.is_empty() && code.chars().all(|c| self.alphabet.contains(c))
    }

    /// 编码中不在字母表内的字符（`SuggestionWarning::UnsupportedCharset` 用）。
    pub fn invalid_chars(&self, code: &str) -> Vec<char> {
        code.chars()
            .filter(|c| !self.alphabet.contains(*c))
            .collect()
    }
}

// ─────────────────── 内置小鹤双拼键位事实表（兜底） ───────────────────

/// 声母 → 键位（小鹤双拼：zh→v、ch→i、sh→u）。
fn flypy_initial_key(initial: &str) -> Option<char> {
    Some(match initial {
        "" => return None,
        "zh" => 'v',
        "ch" => 'i',
        "sh" => 'u',
        "b" => 'b',
        "p" => 'p',
        "m" => 'm',
        "f" => 'f',
        "d" => 'd',
        "t" => 't',
        "n" => 'n',
        "l" => 'l',
        "g" => 'g',
        "k" => 'k',
        "h" => 'h',
        "j" => 'j',
        "q" => 'q',
        "x" => 'x',
        "r" => 'r',
        "z" => 'z',
        "c" => 'c',
        "s" => 's',
        "y" => 'y',
        "w" => 'w',
        _ => return None,
    })
}

/// 韵母 → 键位（小鹤双拼官方键位图）。
fn flypy_final_key(fin: &str) -> Option<char> {
    Some(match fin {
        "a" => 'a',
        "o" => 'o',
        "e" => 'e',
        "i" => 'i',
        "u" => 'u',
        "v" => 'v',
        "ai" => 'd',
        "ei" => 'w',
        "ao" => 'c',
        "ou" => 'z',
        "an" => 'j',
        "en" => 'f',
        "ang" => 'h',
        "eng" => 'g',
        "er" => 'r',
        "in" => 'b',
        "ing" => 'k',
        "un" => 'y',
        "ong" => 's',
        "ia" => 'x',
        "iao" => 'n',
        "ian" => 'm',
        "iang" => 'l',
        "ie" => 'p',
        "iu" => 'q',
        "iong" => 's',
        "ua" => 'x',
        "uo" => 'o',
        "uai" => 'k',
        "uan" => 'r',
        "uang" => 'l',
        "ui" => 'v',
        "ue" => 't',
        "ve" => 't',
        _ => return None,
    })
}

/// 内置小鹤双拼映射（单个音节）。零声母音节按官方规则：单韵母双写、
/// `ang→ah`、`eng→eg`，`ai/an/ao/ei/en/er/ou` 保持两字母。
fn flypy_code(syllable: &str) -> Option<String> {
    let (initial, fin) = syllable::split_initial(syllable);
    if initial.is_empty() {
        let code = match syllable {
            "ang" => "ah".to_owned(),
            "eng" => "eg".to_owned(),
            other if other.chars().count() == 1 => format!("{other}{other}"),
            other => other.to_owned(),
        };
        return Some(code);
    }
    let initial_key = flypy_initial_key(initial)?;
    let final_key = flypy_final_key(fin)?;
    Some([initial_key, final_key].iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flypy_fallback_covers_all_syllables() {
        let dp = DoublePinyin::flypy();
        assert_eq!(dp.syllable_count(), SYLLABLES.len());
        assert_eq!(dp.alphabet(), "abcdefghijklmnopqrstuvwxyz");
    }

    /// 报告 §2.2/§2.3 的关键音节（兜底表侧）。
    #[test]
    fn flypy_fallback_key_syllables() {
        let dp = DoublePinyin::flypy();
        let cases = [
            ("zhong", "vs"),
            ("mei", "mw"),
            ("tuan", "tr"),
            ("xing", "xk"),
            ("shuang", "ul"),
            ("nv", "nv"),
            ("lve", "lt"),
            ("ang", "ah"),
            ("an", "an"),
            ("a", "aa"),
            ("chong", "is"),
            ("zhou", "vz"),
            ("yun", "yy"),
            ("ai", "ai"),
            ("ao", "ao"),
            ("er", "er"),
            ("eng", "eg"),
            ("o", "oo"),
            ("e", "ee"),
        ];
        for (syllable, expected) in cases {
            assert_eq!(
                dp.code_for(syllable).as_deref(),
                Some(expected),
                "音节 {syllable}"
            );
        }
    }

    #[test]
    fn flypy_codes_are_two_letters() {
        let dp = DoublePinyin::flypy();
        for s in SYLLABLES {
            let code = dp.code_for(s).unwrap_or_else(|| panic!("{s} 无映射"));
            assert_eq!(code.chars().count(), 2, "{s} → {code} 不是两键");
            assert!(dp.is_valid_code(&code));
        }
    }

    #[test]
    fn first_key_and_codes_for() {
        let dp = DoublePinyin::flypy();
        assert_eq!(dp.first_key("zhong"), Some('v'));
        assert_eq!(dp.codes_for("mei"), vec!["mw".to_string()]);
        assert_eq!(dp.code_for("m"), None);
        assert_eq!(dp.code_for("zzz"), None);
    }

    #[test]
    fn unknown_syllable_without_algebra_yields_none() {
        let dp = DoublePinyin::flypy();
        assert!(!dp.has_syllable("hm"));
        assert_eq!(dp.code_for("hm"), None);
    }

    #[test]
    fn invalid_chars_reports_alphabet_violations() {
        let dp = DoublePinyin::flypy();
        assert_eq!(dp.invalid_chars("abⓆ"), vec!['Ⓠ']);
        assert!(dp.is_valid_code("abc"));
        assert!(!dp.is_valid_code(""));
    }
}
