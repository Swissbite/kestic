// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The fields of the exclude rules editor: read from [`ExcludeRules`], written back to them
//! and compared with the saved ones.

use crate::excludes::ExcludeRules;
use serde::{Deserialize, Serialize};

/// The unit of the size limit (`--exclude-larger-than 500M`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SizeUnit {
    K,
    M,
    G,
    T,
}

impl SizeUnit {
    fn from_letter(letter: u8) -> Option<Self> {
        match letter.to_ascii_uppercase() {
            b'K' => Some(Self::K),
            b'M' => Some(Self::M),
            b'G' => Some(Self::G),
            b'T' => Some(Self::T),
            _ => None,
        }
    }

    fn letter(self) -> char {
        match self {
            Self::K => 'K',
            Self::M => 'M',
            Self::G => 'G',
            Self::T => 'T',
        }
    }
}

/// The fields of the exclude rules editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcludeForm {
    pub folders: Vec<String>,
    /// The expert rules, one per line.
    pub lines_text: String,
    pub exclude_caches: bool,
    pub exclude_trash: bool,
    pub size_limit_enabled: bool,
    pub size_limit_num: u64,
    pub size_limit_unit: SizeUnit,
    pub ignore_case: bool,
    /// The marker files (`--exclude-if-present`), separated by commas.
    pub markers_text: String,
}

/// What the editor shows for its current fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExcludeFormState {
    /// The rules the fields stand for.
    pub rules: ExcludeRules,
    /// The rules differ from the saved ones.
    pub dirty: bool,
    /// Expert rules (not blank, not a comment), which the simple mode only counts.
    pub expert_rule_count: usize,
}

/// The size limit as the editor shows it (`500M`, `2 gb`); `None` for one it cannot show.
fn size_limit(raw: &str) -> Option<(u64, SizeUnit)> {
    let raw = raw.trim();
    let raw = raw
        .strip_suffix(['B', 'b'])
        .filter(|rest| rest.ends_with(|c: char| c.is_ascii_alphabetic()))
        .unwrap_or(raw);
    let (&letter, number) = raw.as_bytes().split_last()?;
    let unit = SizeUnit::from_letter(letter)?;
    let digits = std::str::from_utf8(number).ok()?.trim_end();
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((digits.parse::<u64>().ok()?.max(1), unit))
}

/// The lines of `text`; an empty text has none.
fn split_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect()
}

/// The marker files of a comma-separated text, trimmed, without empty ones.
fn split_markers(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .collect()
}

/// `rules` with a blank size limit as none, so it compares equal to no limit.
fn comparable(rules: &ExcludeRules) -> ExcludeRules {
    let mut rules = rules.clone();
    if rules.exclude_larger_than.as_deref() == Some("") {
        rules.exclude_larger_than = None;
    }
    rules
}

impl ExcludeForm {
    /// The fields for `rules`; a size limit the editor cannot show is off and offers 100 MB.
    pub fn from_rules(rules: &ExcludeRules) -> Self {
        let size = rules.exclude_larger_than.as_deref().and_then(size_limit);
        let (size_limit_num, size_limit_unit) = size.unwrap_or((100, SizeUnit::M));
        Self {
            folders: rules.folders.clone(),
            lines_text: rules.lines.join("\n"),
            exclude_caches: rules.exclude_caches,
            exclude_trash: rules.exclude_trash,
            size_limit_enabled: size.is_some(),
            size_limit_num,
            size_limit_unit,
            ignore_case: rules.ignore_case,
            markers_text: rules.exclude_if_present.join(", "),
        }
    }

    /// The rules the fields stand for; a size below 1 counts as 1.
    pub fn to_rules(&self) -> ExcludeRules {
        ExcludeRules {
            folders: self.folders.clone(),
            lines: split_lines(&self.lines_text),
            exclude_caches: self.exclude_caches,
            exclude_trash: self.exclude_trash,
            exclude_larger_than: self.size_limit_enabled.then(|| {
                format!(
                    "{}{}",
                    self.size_limit_num.max(1),
                    self.size_limit_unit.letter()
                )
            }),
            ignore_case: self.ignore_case,
            exclude_if_present: split_markers(&self.markers_text),
        }
    }

    /// The rules and whether they differ from `saved`.
    pub fn state(&self, saved: &ExcludeRules) -> ExcludeFormState {
        let rules = self.to_rules();
        ExcludeFormState {
            dirty: comparable(&rules) != comparable(saved),
            expert_rule_count: rules.expert_rule_count(),
            rules,
        }
    }
}

/// `lines_text` with the first line that is `parent_rule` (ignoring the blanks around it)
/// replaced by `suggested_fix`, as the lint suggests for a shadowed `!` rule.
pub fn apply_lint_fix(lines_text: &str, parent_rule: &str, suggested_fix: &str) -> String {
    let mut replaced = false;
    split_lines(lines_text)
        .into_iter()
        .map(|line| {
            if !replaced && line.trim() == parent_rule {
                replaced = true;
                suggested_fix.to_string()
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[path = "exclude_form_tests.rs"]
mod tests;
