//! 拼音声调显示（`bao4 → bào`），只用于 `CodeSuggestion::reading` 的可读标注，
//! 不参与编码计算（编码只用无调音节）。

/// 给无调音节加声调符号；`tone` 为 `None`/`0`/`5`（轻声）时原样返回。
///
/// 记调规则与《汉语拼音方案》一致：有 `a` 标 `a`，否则有 `e` 标 `e`，
/// `ou` 标 `o`，其余标最后一个元音。`v` 视作 `ü`（`lv3 → lǚ`）。
pub fn with_tone(syllable: &str, tone: Option<u8>) -> String {
    let syllable: Vec<char> = syllable
        .chars()
        .map(|c| if c == 'v' { 'ü' } else { c })
        .collect();
    let Some(tone) = tone.filter(|t| (1..=4).contains(t)) else {
        return syllable.into_iter().collect();
    };
    let Some(index) = accent_index(&syllable) else {
        return syllable.into_iter().collect();
    };
    let mut out = String::with_capacity(syllable.len() + 1);
    for (i, ch) in syllable.iter().enumerate() {
        if i == index {
            out.push(accented(*ch, tone).unwrap_or(*ch));
        } else {
            out.push(*ch);
        }
    }
    out
}

fn accent_index(chars: &[char]) -> Option<usize> {
    if let Some(i) = chars.iter().position(|c| *c == 'a') {
        return Some(i);
    }
    if let Some(i) = chars.iter().position(|c| *c == 'e') {
        return Some(i);
    }
    // `ou` 标在 o 上
    for (i, w) in chars.windows(2).enumerate() {
        if w == ['o', 'u'] {
            return Some(i);
        }
    }
    chars
        .iter()
        .rposition(|c| matches!(c, 'i' | 'o' | 'u' | 'ü'))
}

fn accented(vowel: char, tone: u8) -> Option<char> {
    let table: [char; 4] = match vowel {
        'a' => ['ā', 'á', 'ǎ', 'à'],
        'e' => ['ē', 'é', 'ě', 'è'],
        'i' => ['ī', 'í', 'ǐ', 'ì'],
        'o' => ['ō', 'ó', 'ǒ', 'ò'],
        'u' => ['ū', 'ú', 'ǔ', 'ù'],
        'ü' => ['ǖ', 'ǘ', 'ǚ', 'ǜ'],
        _ => return None,
    };
    Some(table[(tone - 1) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_the_right_vowel() {
        assert_eq!(with_tone("bao", Some(4)), "bào");
        assert_eq!(with_tone("xing", Some(2)), "xíng");
        assert_eq!(with_tone("mei", Some(3)), "měi");
        assert_eq!(with_tone("shuang", Some(1)), "shuāng");
        assert_eq!(with_tone("tuan", Some(2)), "tuán");
        assert_eq!(with_tone("hui", Some(4)), "huì");
        assert_eq!(with_tone("you", Some(3)), "yǒu");
        assert_eq!(with_tone("liu", Some(2)), "liú");
        assert_eq!(with_tone("lv", Some(3)), "lǚ");
        assert_eq!(with_tone("lve", Some(4)), "lüè");
    }

    #[test]
    fn neutral_tone_is_plain() {
        assert_eq!(with_tone("de", Some(5)), "de");
        assert_eq!(with_tone("de", None), "de");
        assert_eq!(with_tone("de", Some(0)), "de");
    }
}
