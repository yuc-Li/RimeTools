//! 编码建议引擎（PRD FR-3.23 / FR-2.6）。
//!
//! 把已用真实码表验证过的原型
//! 移植为 Rust：
//!
//! 1. [`algebra`]：Rime `speller/algebra` 求值器（`xform`/`derive`/`erase`/`xlit`/`abbrev`）。
//!    **规则逐条顺序作用、允许重复命中**（`ang → aang → aⓗ → ah` 依赖此语义）。
//! 2. [`double_pinyin`]：对合法全拼音节表求值得到「全拼 → 双拼码」；方案无 algebra
//!    时退回内置小鹤键位事实表（PRD R7 兜底）。
//! 3. [`syllable`]：自建全拼音节表（语言事实枚举，无第三方码表内容）。
//! 4. [`pinyin`]：汉字 → 拼音数据源，内置常用字表 + [`pinyin::PinyinSource`] trait 可注入。
//! 5. [`shape`]：形码反查规范化（过滤 `xhup_reverse` 里含 `` ` `` 的万能码变体）。
//! 6. [`english`]：英文小写/大写/原样，短语去空格与连字符。
//! 7. [`engine`]：按方案策略合成词码（C2 规则），输出多候选 + 置信度 + 警告。
//!
//! ## 许可边界（PRD §4）
//!
//! 凇鹤（rime-crane）为 GPL-3.0。本 crate **不分发任何方案码表内容**：双拼映射由
//! 运行时解析用户方案 algebra 得到，形码由上层从本地码表反查后注入；内置的只有
//! 音节表（语言事实）与小鹤键位表（键位事实）。拼音内置表是手工录入的读音事实。
//!
//! ## 典型用法
//!
//! ```
//! use rime_suggest::{SuggestEngine, SuggestMode, SuggestOptions};
//!
//! // 上层从 *.schema.yaml 解析出 speller/algebra（rime_core::types::AlgebraSpec），
//! // 这里用内置小鹤键位表兜底。
//! let engine = SuggestEngine::for_double_pinyin(None);
//! let opts = SuggestOptions::new(SuggestMode::ShapeWord);
//! let suggestions = engine.suggest("报关单", &opts);
//! assert_eq!(suggestions[0].code, "bgd");
//! ```

#![forbid(unsafe_code)]

pub mod algebra;
pub mod double_pinyin;
pub mod engine;
pub mod english;
pub mod pinyin;
pub mod shape;
pub mod syllable;
pub mod tones;

pub use algebra::{Algebra, AlgebraError, Evaluation, Rule, RuleKind, Spelling, SpellingOrigin};
pub use double_pinyin::{DoublePinyin, SyllableCodes};
pub use engine::{blocks_learning, SuggestEngine, SuggestMode, SuggestOptions, DEFAULT_LIMIT};
pub use english::EnglishCase;
pub use pinyin::{is_han, BuiltinPinyin, PinyinReading, PinyinSource};
pub use shape::{canonical_shape, ShapeMap, ShapeSource};
pub use syllable::{is_syllable, SYLLABLES};
