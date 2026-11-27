//! 逐键模拟 + 候选采集的纯逻辑层。
//!
//! 这里刻意把“会话 I/O”抽象成 [`SessionIo`]，于是：
//! - 真实实现 [`crate::preview::LibrimeSessionIo`] 走 librime；
//! - 单测用脚本化 mock 精确复现 xhup 的 4 码 auto_select 行为（第 4 键上屏、context 清空），
//!   不依赖本机是否装了 librime。
//!
//! ## xhup 4 码自动上屏（PRD FR-3.24 / §6.5 R1）
//!
//! xhup 的 `speller/auto_select: true` + `auto_select_pattern: ^;.$|^\w{4}$` 会让第 4 码
//! **同步上屏**，`RimeGetContext` 只能读到空 context，随后再按键就是新组合。
//!
//! 本工具的两层处理：
//! 1. 首选：隔离目录里的方案副本通过 `<schema>.custom.yaml` 关闭 `speller/auto_select`
//!    （只改临时副本，不碰用户文件），这样任意码长都能稳定读到候选；
//! 2. 兜底：仍然逐键读 context 并缓存快照。若调用方给的是没打 patch 的目录（auto_select
//!    仍开着），第 4 键触发上屏后回退到“最后一个有候选的前缀”，并在 `note` 里说明
//!    已上屏文本与所用前缀，绝不静默返回空候选。

use crate::error::FfiError;

/// 一个原始候选（text/comment 已从 librime 内存拷贝出来）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawCandidate {
    pub text: String,
    pub comment: Option<String>,
}

/// 一次 `RimeGetContext` 的快照。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawContext {
    /// 组合区（preedit）文本，通常是已输入的编码。
    pub preedit: String,
    /// `menu/page_size`，即 PRD 里候选预览的 N。
    pub page_size: usize,
    pub candidates: Vec<RawCandidate>,
}

/// 模拟一个按键之后的结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StepResult {
    /// 该键是否被会话接受（`RimeSimulateKeySequence` 返回值）。
    pub accepted: bool,
    /// 该键触发的上屏文本（auto_select / 选词都会在这里出现）。
    pub committed: Option<String>,
    /// 按键后的 context 快照。
    pub context: RawContext,
}

/// 预览会话 I/O 抽象：真实实现走 librime，测试用 mock。
pub trait SessionIo {
    /// 发送一个按键，并返回该键之后的 commit/context。
    fn step(&mut self, key: char) -> Result<StepResult, FfiError>;
}

/// 一次候选采集的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    pub candidates: Vec<RawCandidate>,
    /// 实际用到的前缀（正常情况下等于输入编码）。
    pub prefix_used: String,
    /// 该前缀下的 `menu/page_size`。
    pub page_size: usize,
    /// 若回退到前缀候选、或编码无候选，给出可展示的中文说明。
    pub note: Option<String>,
    /// 本次模拟期间最后一次上屏文本（auto_select 会触发）。
    pub committed: Option<String>,
    /// 是否成功读到“完整编码”对应的候选（未回退）。
    pub exact: bool,
}

impl Capture {
    /// 无候选、无上屏时的安全默认值。
    fn empty() -> Self {
        Self {
            candidates: Vec::new(),
            prefix_used: String::new(),
            page_size: 0,
            note: Some("该编码在当前方案下没有候选".to_owned()),
            committed: None,
            exact: false,
        }
    }
}

/// 逐键模拟 `code`，采集候选。
///
/// - 每键后先读 commit 再读 context（这样 auto_select 的上屏文本不会丢）；
/// - 若中途出现上屏（auto_select / 顶屏），停止继续发送剩余按键（否则会输入到新组合里）；
/// - 优先返回“完整编码 + preedit 匹配”的那次快照；否则回退到最后一个有候选的前缀快照，
///   并在 `note` 里说明回退原因。
pub fn collect_candidates<S: SessionIo>(
    io: &mut S,
    code: &str,
    max_items: Option<usize>,
) -> Result<Capture, FfiError> {
    let keys: Vec<char> = code.chars().filter(|ch| !ch.is_whitespace()).collect();
    if keys.is_empty() {
        return Err(FfiError::InvalidCode {
            reason: "编码为空".to_owned(),
        });
    }

    let mut snapshots: Vec<(usize, StepResult)> = Vec::new();
    let mut last_commit: Option<String> = None;
    let mut keys_rejected: Vec<char> = Vec::new();
    let mut stopped_early = false;

    for (index, key) in keys.iter().enumerate() {
        let step = io.step(*key)?;
        if !step.accepted {
            keys_rejected.push(*key);
        }
        if let Some(text) = step.committed.as_ref().filter(|text| !text.is_empty()) {
            last_commit = Some(text.clone());
        }
        let prefix_len = index + 1;
        let committed_now = step.committed.is_some();
        snapshots.push((prefix_len, step));
        // 上屏后 context 已被清空；继续敲剩余键会进入新的组合，语义就错了。
        if committed_now && prefix_len < keys.len() {
            stopped_early = true;
            break;
        }
    }

    let full_code: String = keys.iter().collect();

    // 完整编码的快照：preedit 归一化后与编码一致（table/abc segmentor 通常直接相等）。
    // 部分方案在组合区不写 preedit（为空），此时以“长度已达完整编码 + 有候选”为准。
    let exact_snapshot = snapshots.iter().find(|(len, step)| {
        *len == keys.len()
            && !step.context.candidates.is_empty()
            && (normalize_preedit(&step.context.preedit) == full_code
                || step.context.preedit.trim().is_empty())
    });

    if let Some((_, step)) = exact_snapshot {
        let mut capture = finish(
            &step.context,
            full_code.clone(),
            last_commit.clone(),
            None,
            true,
        );
        capture.note = notes_for(
            true,
            last_commit.as_deref(),
            stopped_early,
            !keys_rejected.is_empty(),
            None,
        );
        truncate_candidates(&mut capture, max_items);
        return Ok(capture);
    }

    // 回退：最后一个有候选的快照。
    let fallback = snapshots
        .iter()
        .rev()
        .find(|(_, step)| !step.context.candidates.is_empty());

    let Some((prefix_len, step)) = fallback else {
        let mut capture = Capture::empty();
        capture.committed = last_commit;
        capture.note = notes_for(
            false,
            capture.committed.as_deref(),
            stopped_early,
            !keys_rejected.is_empty(),
            None,
        )
        .or_else(|| Some("该编码在当前方案下没有候选".to_owned()));
        return Ok(capture);
    };

    let prefix: String = keys.iter().take(*prefix_len).collect();
    let mut capture = finish(
        &step.context,
        prefix.clone(),
        last_commit.clone(),
        None,
        false,
    );
    capture.note = notes_for(
        false,
        last_commit.as_deref(),
        stopped_early,
        !keys_rejected.is_empty(),
        Some(&prefix),
    );
    truncate_candidates(&mut capture, max_items);
    Ok(capture)
}

fn finish(
    context: &RawContext,
    prefix_used: String,
    committed: Option<String>,
    note: Option<String>,
    exact: bool,
) -> Capture {
    Capture {
        candidates: context.candidates.clone(),
        prefix_used,
        page_size: context.page_size,
        note,
        committed,
        exact,
    }
}

/// 截断到 N（PRD：N = `menu/page_size`，可被调用方显式收紧）。
fn truncate_candidates(capture: &mut Capture, max_items: Option<usize>) {
    let limit = match (max_items, capture.page_size) {
        (Some(max), page_size) if page_size > 0 => max.min(page_size),
        (Some(max), _) => max,
        (None, page_size) if page_size > 0 => page_size,
        (None, _) => capture.candidates.len(),
    };
    if capture.candidates.len() > limit {
        capture.candidates.truncate(limit);
    }
}

fn notes_for(
    exact: bool,
    committed: Option<&str>,
    stopped_early: bool,
    keys_rejected: bool,
    prefix: Option<&str>,
) -> Option<String> {
    let mut notes: Vec<String> = Vec::new();
    if !exact {
        match (committed, prefix) {
            (Some(text), Some(prefix)) if stopped_early => notes.push(format!(
                "输入在完成前触发自动上屏（已上屏「{text}」），完整编码的候选不可读；已回退为前缀「{prefix}」的候选。隔离预览目录会关闭 speller/auto_select 以避免此问题"
            )),
            (Some(text), Some(prefix)) => notes.push(format!(
                "输入完成时触发自动上屏（已上屏「{text}」），完整编码的候选不可读；已回退为前缀「{prefix}」的候选"
            )),
            (Some(text), None) => notes.push(format!(
                "输入触发自动上屏（已上屏「{text}」），完整编码没有任何候选"
            )),
            (None, Some(prefix)) => {
                notes.push(format!("完整编码没有候选，已回退为前缀「{prefix}」的候选"))
            }
            (None, None) => {}
        }
    }
    if keys_rejected {
        notes.push("部分按键未被隔离会话接受，结果可能不完整".to_owned());
    }
    if notes.is_empty() {
        None
    } else {
        Some(notes.join("；"))
    }
}

fn normalize_preedit(preedit: &str) -> String {
    preedit.chars().filter(|ch| !ch.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// 脚本化 mock：按预期顺序返回每一步结果；键序不符会 panic。
    struct ScriptedIo {
        expected: VecDeque<char>,
        results: VecDeque<StepResult>,
        seen: Vec<char>,
    }

    impl ScriptedIo {
        fn new(script: Vec<(char, StepResult)>) -> Self {
            Self {
                expected: script.iter().map(|(key, _)| *key).collect(),
                results: script.into_iter().map(|(_, result)| result).collect(),
                seen: Vec::new(),
            }
        }
    }

    impl SessionIo for ScriptedIo {
        fn step(&mut self, key: char) -> Result<StepResult, FfiError> {
            let expected = self
                .expected
                .pop_front()
                .unwrap_or_else(|| panic!("未预期的按键 {key:?}（脚本已耗尽）"));
            assert_eq!(key, expected, "按键顺序与脚本不一致");
            self.seen.push(key);
            Ok(self.results.pop_front().expect("脚本结果缺失"))
        }
    }

    fn candidate(text: &str) -> RawCandidate {
        RawCandidate {
            text: text.to_owned(),
            comment: None,
        }
    }

    fn step(preedit: &str, candidates: Vec<RawCandidate>) -> StepResult {
        StepResult {
            accepted: true,
            committed: None,
            context: RawContext {
                preedit: preedit.to_owned(),
                page_size: 5,
                candidates,
            },
        }
    }

    #[test]
    fn exact_code_returns_full_candidates() {
        let io = &mut ScriptedIo::new(vec![
            ('a', step("a", vec![candidate("啊")])),
            ('b', step("ab", vec![candidate("阿")])),
            ('c', step("abc", vec![candidate("按")])),
            ('d', step("abcd", vec![candidate("甲"), candidate("乙")])),
        ]);
        let capture = collect_candidates(io, "abcd", None).unwrap();
        assert!(capture.exact);
        assert_eq!(capture.prefix_used, "abcd");
        assert_eq!(capture.candidates, vec![candidate("甲"), candidate("乙")]);
        assert_eq!(capture.page_size, 5);
        assert!(capture.note.is_none());
        assert_eq!(io.seen, vec!['a', 'b', 'c', 'd']);
    }

    #[test]
    fn auto_select_commit_falls_back_to_last_prefix_with_note() {
        // 复现 xhup：第 4 码直接上屏，context 被清空。
        let io = &mut ScriptedIo::new(vec![
            ('a', step("a", vec![candidate("啊")])),
            ('b', step("ab", vec![candidate("阿")])),
            ('c', step("abc", vec![candidate("按")])),
            (
                'd',
                StepResult {
                    accepted: true,
                    committed: Some("甲".to_owned()),
                    context: RawContext::default(),
                },
            ),
        ]);
        let capture = collect_candidates(io, "abcd", None).unwrap();
        assert!(!capture.exact);
        assert_eq!(capture.prefix_used, "abc");
        assert_eq!(capture.candidates, vec![candidate("按")]);
        assert_eq!(capture.committed.as_deref(), Some("甲"));
        let note = capture.note.expect("必须给出回退说明");
        assert!(note.contains("自动上屏"), "note = {note}");
        assert!(note.contains("甲"), "note = {note}");
        assert!(note.contains("abc"), "note = {note}");
        // 第 4 键之后不应再发送按键。
        assert_eq!(io.seen, vec!['a', 'b', 'c', 'd']);
    }

    #[test]
    fn early_commit_stops_remaining_keys() {
        // 5 码编码在第 4 码上屏：剩余第 5 键不能发送。
        let io = &mut ScriptedIo::new(vec![
            ('a', step("a", vec![candidate("啊")])),
            ('b', step("ab", vec![candidate("阿")])),
            ('c', step("abc", vec![candidate("按")])),
            (
                'd',
                StepResult {
                    accepted: true,
                    committed: Some("甲".to_owned()),
                    context: RawContext::default(),
                },
            ),
        ]);
        let capture = collect_candidates(io, "abcde", None).unwrap();
        assert_eq!(io.seen, vec!['a', 'b', 'c', 'd']);
        assert!(!capture.exact);
        assert_eq!(capture.prefix_used, "abc");
        assert_eq!(capture.committed.as_deref(), Some("甲"));
        assert!(capture.note.is_some());
    }

    #[test]
    fn max_items_caps_candidates() {
        let io = &mut ScriptedIo::new(vec![(
            'a',
            step("a", vec![candidate("一"), candidate("二"), candidate("三")]),
        )]);
        let capture = collect_candidates(io, "a", Some(2)).unwrap();
        assert_eq!(capture.candidates.len(), 2);
    }

    #[test]
    fn page_size_caps_candidates_when_no_override() {
        let io = &mut ScriptedIo::new(vec![(
            'a',
            StepResult {
                accepted: true,
                committed: None,
                context: RawContext {
                    preedit: "a".to_owned(),
                    page_size: 2,
                    candidates: vec![candidate("一"), candidate("二"), candidate("三")],
                },
            },
        )]);
        let capture = collect_candidates(io, "a", None).unwrap();
        assert_eq!(capture.candidates.len(), 2);
    }

    #[test]
    fn empty_or_whitespace_code_is_invalid() {
        let io = &mut ScriptedIo::new(vec![]);
        assert!(matches!(
            collect_candidates(io, "", None),
            Err(FfiError::InvalidCode { .. })
        ));
        let io = &mut ScriptedIo::new(vec![]);
        assert!(matches!(
            collect_candidates(io, "   ", None),
            Err(FfiError::InvalidCode { .. })
        ));
    }

    #[test]
    fn no_candidates_gives_empty_capture_with_note() {
        let io = &mut ScriptedIo::new(vec![('a', step("a", vec![]))]);
        let capture = collect_candidates(io, "a", None).unwrap();
        assert!(capture.candidates.is_empty());
        assert!(!capture.exact);
        assert!(capture.note.unwrap().contains("没有候选"));
    }

    #[test]
    fn rejected_key_is_reported_in_note() {
        let mut result = step("a", vec![candidate("啊")]);
        result.accepted = false;
        let io = &mut ScriptedIo::new(vec![('a', result)]);
        let capture = collect_candidates(io, "a", None).unwrap();
        assert!(capture.candidates == vec![candidate("啊")]);
        assert!(capture.note.unwrap().contains("未被隔离会话接受"));
    }

    #[test]
    fn whitespace_in_code_is_ignored() {
        let io = &mut ScriptedIo::new(vec![
            ('a', step("a", vec![candidate("啊")])),
            ('b', step("ab", vec![candidate("阿")])),
        ]);
        let capture = collect_candidates(io, "a b", None).unwrap();
        assert_eq!(capture.prefix_used, "ab");
        assert!(capture.exact);
    }
}
