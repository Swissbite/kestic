// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Backup source types, exclude rules, linting, and path evaluation.

use crate::model::ModelError;
use crate::mount::normalize_lexical_path;
use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};

/// Kind of backup source (glossary: `Source`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    #[default]
    Folder,
    Drive,
}

/// Persistent hardware/mount identity for a removable or mounted drive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceIdentity {
    /// Block device filesystem UUID (resolved via udev / `/dev/disk/by-uuid`, never mountinfo).
    FsUuid { fs_uuid: String },
    /// Filesystem without UUID (e.g. `nfs4`, `sshfs`, `tmpfs`).
    MountSource {
        mount_source: String,
        fs_type: String,
    },
}

/// Backup source directory/drive (glossary: `Source`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub path: PathBuf,
    #[serde(default)]
    pub kind: SourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<SourceIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive_label: Option<String>,
}

impl Source {
    /// Creates a [`Source`] of kind [`SourceKind::Folder`].
    pub fn folder(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: SourceKind::Folder,
            identity: None,
            drive_label: None,
        }
    }

    /// Creates a [`Source`] of kind [`SourceKind::Drive`].
    pub fn drive(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: SourceKind::Drive,
            identity: None,
            drive_label: None,
        }
    }

    /// Creates a [`Source`] of kind [`SourceKind::Drive`] with detected identity & label.
    pub fn drive_with_identity(
        path: impl Into<PathBuf>,
        identity: Option<SourceIdentity>,
        drive_label: Option<String>,
    ) -> Self {
        Self {
            path: path.into(),
            kind: SourceKind::Drive,
            identity,
            drive_label,
        }
    }
}

/// Parsed single exclude rule entry (glossary: `ExcludeRule::Negated`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "pattern", rename_all = "snake_case")]
pub enum ExcludeRule {
    Pattern(String),
    Negated(String),
}

impl ExcludeRule {
    /// Parses a non-empty, non-comment line into an [`ExcludeRule`].
    pub fn parse_line(line: &str) -> Option<Self> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return None;
        }
        if let Some(rest) = trimmed.strip_prefix('!') {
            Some(Self::Negated(rest.to_string()))
        } else {
            Some(Self::Pattern(trimmed.to_string()))
        }
    }
}

/// Standard desktop trash exclusion patterns emitted when `exclude_trash` is active.
pub const TRASH_EXCLUDE_PATTERNS: [&str; 2] = ["$HOME/.local/share/Trash", "**/.Trash-*"];

/// Exclude configuration for a [`Backup`](crate::model::Backup) (glossary: `ExcludeRules`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExcludeRules {
    /// Simple-mode excluded folders (`$HOME/...` inside `$HOME`, `/...` outside `$HOME`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<String>,
    /// Expert-mode raw exclude lines (`**`, `!negation`, `$VAR`, `--exclude-file` rules).
    pub lines: Vec<String>,
    /// Passed to `--exclude-caches`.
    pub exclude_caches: bool,
    /// Template: exclude user/per-volume Trash directories.
    pub exclude_trash: bool,
    /// Passed to `--exclude-larger-than <size>` (e.g. `"2G"`, `"500M"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_larger_than: Option<String>,
    /// Use `--iexclude-file` instead of `--exclude-file` (case-insensitive matching).
    pub ignore_case: bool,
    /// Passed via repeatable `--exclude-if-present <entry>` (e.g. `".nobackup"`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exclude_if_present: Vec<String>,
}

/// Validates restic `--exclude-larger-than` size literal (digits + optional `k/K/m/M/g/G/t/T`).
pub fn validate_exclude_larger_than(raw: &str) -> Result<String, ModelError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ModelError::InvalidBackup(
            "exclude_larger_than must not be blank".to_string(),
        ));
    }
    let last = trimmed.chars().last().expect("non-empty");
    let digits = if matches!(last, 'k' | 'K' | 'm' | 'M' | 'g' | 'G' | 't' | 'T') {
        &trimmed[..trimmed.len() - last.len_utf8()]
    } else {
        trimmed
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ModelError::InvalidBackup(format!(
            "invalid exclude_larger_than size '{raw}'; expected e.g. 500M or 2G"
        )));
    }
    let num: u64 = digits.parse().map_err(|_| {
        ModelError::InvalidBackup(format!("exclude_larger_than number too large: '{raw}'"))
    })?;
    if num == 0 {
        return Err(ModelError::InvalidBackup(
            "exclude_larger_than must be greater than 0".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

/// Normalizes a simple folder exclude rule (`$HOME/...` when under `home_dir`, else absolute).
pub fn normalize_folder_rule(raw: &str, home_dir: Option<&Path>) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed == "$HOME" || trimmed.starts_with("$HOME/") {
        return trimmed.trim_end_matches('/').to_string();
    }

    let path = Path::new(trimmed);
    let norm = normalize_lexical_path(path);
    if let Some(home) = home_dir {
        let norm_home = normalize_lexical_path(home);
        if norm == norm_home {
            return "$HOME".to_string();
        }
        if let Ok(rel) = norm.strip_prefix(&norm_home) {
            return format!("$HOME/{}", rel.to_string_lossy());
        }
    }
    norm.to_string_lossy().into_owned()
}

impl ExcludeRules {
    /// Creates [`ExcludeRules`] from multi-line expert syntax text input.
    pub fn from_text(text: &str) -> Self {
        let lines = text.lines().map(|l| l.to_string()).collect();
        Self {
            folders: Vec::new(),
            lines,
            exclude_caches: false,
            exclude_trash: false,
            exclude_larger_than: None,
            ignore_case: false,
            exclude_if_present: Vec::new(),
        }
    }

    /// Parses all active [`ExcludeRule`] entries from `self.lines` (ignoring comments and blank lines).
    pub fn parsed_rules(&self) -> Vec<ExcludeRule> {
        self.lines
            .iter()
            .filter_map(|line| ExcludeRule::parse_line(line))
            .collect()
    }

    /// Returns the number of active expert-syntax rules (shown as «N further rules in restic syntax»).
    pub fn expert_rule_count(&self) -> usize {
        self.parsed_rules().len()
    }

    /// Returns `true` if any rule contributes a line to the generated `--exclude-file`.
    pub fn has_exclude_file_entries(&self) -> bool {
        self.exclude_trash
            || self.folders.iter().any(|f| !f.trim().is_empty())
            || self.lines.iter().any(|l| !l.trim().is_empty())
    }

    /// Normalizes all folder rules relative to `home_dir`.
    pub fn normalize_folders(&mut self, home_dir: Option<&Path>) {
        let mut out = Vec::with_capacity(self.folders.len());
        for raw in &self.folders {
            let norm = normalize_folder_rule(raw, home_dir);
            if !norm.is_empty() && !out.contains(&norm) {
                out.push(norm);
            }
        }
        self.folders = out;
    }

    /// Serializes all exclude file entries using the current process `HOME`.
    pub fn to_exclude_file_contents(&self) -> String {
        let home = env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        self.to_exclude_file_contents_with_home(home.as_deref())
    }

    /// Serializes all exclude file entries for `$XDG_STATE_HOME/kestic/excludes/<backup-id>.txt`
    /// (verbatim expert syntax without regex conversion).
    pub fn to_exclude_file_contents_with_home(&self, home_dir: Option<&Path>) -> String {
        let mut out = String::new();
        for folder in &self.folders {
            let norm = normalize_folder_rule(folder, home_dir);
            if !norm.is_empty() {
                out.push_str(&norm);
                out.push('\n');
            }
        }
        if self.exclude_trash {
            for pat in TRASH_EXCLUDE_PATTERNS {
                out.push_str(pat);
                out.push('\n');
            }
        }
        for line in &self.lines {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                out.push_str(trimmed);
                out.push('\n');
            }
        }
        out
    }

    /// Validates exclude options (`exclude_larger_than`, `folders`, `exclude_if_present`).
    pub fn validate(&self) -> Result<(), ModelError> {
        for f in &self.folders {
            if f.contains('\n') || f.contains('\r') {
                return Err(ModelError::InvalidBackup(
                    "exclude folder rule must not contain newlines".to_string(),
                ));
            }
        }
        if let Some(ref size) = self.exclude_larger_than
            && !size.trim().is_empty()
        {
            validate_exclude_larger_than(size)?;
        }
        for marker in &self.exclude_if_present {
            if marker.trim().is_empty() || marker.contains('\n') || marker.contains('\r') {
                return Err(ModelError::InvalidBackup(
                    "exclude_if_present filename must not be empty".to_string(),
                ));
            }
        }
        Ok(())
    }
}

/// Advisory warning emitted when a `!negation` rule is shadowed by a whole-directory exclusion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcludeLintWarning {
    /// 1-based line number inside `ExcludeRules::lines`.
    pub line_number: usize,
    /// The shadowed negation rule (e.g. `"!dir/inner"`).
    pub rule: String,
    /// The parent rule that excludes the whole directory (e.g. `"dir"`).
    pub parent_rule: String,
    /// Suggested replacement for the parent rule (e.g. `"dir/*"`).
    pub suggested_fix: String,
}

fn strip_trailing_slash(s: &str) -> &str {
    if s == "/" { s } else { s.trim_end_matches('/') }
}

fn is_whole_dir_pattern(pat: &str) -> bool {
    let trimmed = pat.trim();
    if trimmed.is_empty() {
        return false;
    }
    let clean = strip_trailing_slash(trimmed);
    !clean.ends_with("/*") && !clean.ends_with("/**") && !clean.ends_with('\\')
}

fn pattern_components(pat: &str) -> Vec<&str> {
    let clean = strip_trailing_slash(pat.trim());
    clean
        .trim_start_matches('/')
        .split('/')
        .filter(|c| !c.is_empty())
        .collect()
}

fn is_ancestor_exclusion(parent_pat: &str, negated_pat: &str) -> bool {
    if !is_whole_dir_pattern(parent_pat) {
        return false;
    }
    let parent_anchored = parent_pat.trim().starts_with('/');
    let neg_anchored = negated_pat.trim().starts_with('/');
    let parent_comps = pattern_components(parent_pat);
    let neg_comps = pattern_components(negated_pat);
    if parent_comps.is_empty() || neg_comps.len() <= parent_comps.len() {
        return false;
    }

    if parent_anchored || parent_pat.contains('/') {
        if parent_anchored != neg_anchored {
            return false;
        }
        return neg_comps.starts_with(&parent_comps);
    }

    // Single-component unanchored parent pattern (e.g. `dir` excludes any `dir` subtree)
    let target = parent_comps[0];
    let has_wildcards = target.contains('*') || target.contains('?') || target.contains('[');
    if !has_wildcards {
        return neg_comps[..neg_comps.len() - 1].contains(&target);
    }
    false
}

/// Checks `rules` for ineffective `!` negation rules shadowed by whole-directory exclusions.
/// Non-blocking: produces warnings only, never prevents saving.
pub fn lint_exclude_rules(rules: &ExcludeRules) -> Vec<ExcludeLintWarning> {
    let mut active_exclusions: Vec<(String, Option<String>)> = Vec::new();
    for folder in &rules.folders {
        let trimmed = folder.trim();
        if !trimmed.is_empty() {
            let clean = strip_trailing_slash(trimmed);
            active_exclusions.push((trimmed.to_string(), Some(format!("{clean}/*"))));
        }
    }

    let mut warnings = Vec::new();
    for (idx, raw_line) in rules.lines.iter().enumerate() {
        let line_no = idx + 1;
        let Some(parsed) = ExcludeRule::parse_line(raw_line) else {
            continue;
        };
        match parsed {
            ExcludeRule::Pattern(pat) => {
                let clean = strip_trailing_slash(pat.trim());
                let fix = if is_whole_dir_pattern(&pat) {
                    Some(format!("{clean}/*"))
                } else {
                    None
                };
                active_exclusions.push((pat, fix));
            }
            ExcludeRule::Negated(neg_pat) => {
                // Only consider exclusions active *before* or unconditionally via folders.
                if let Some((parent, Some(suggested_fix))) = active_exclusions
                    .iter()
                    .rev()
                    .find(|(p, _)| is_ancestor_exclusion(p, &neg_pat))
                {
                    let already_wildcarded = active_exclusions
                        .iter()
                        .any(|(p, _)| strip_trailing_slash(p) == suggested_fix);
                    if !already_wildcarded {
                        warnings.push(ExcludeLintWarning {
                            line_number: line_no,
                            rule: format!("!{neg_pat}"),
                            parent_rule: parent.clone(),
                            suggested_fix: suggested_fix.clone(),
                        });
                    }
                }
            }
        }
    }
    warnings
}

/// Result of testing a path against [`ExcludeRules`] («Test path…»).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcludePathTestResult {
    pub included: bool,
    pub matched_rule: Option<String>,
    /// 1-based line number in expert `lines` if matched by an expert rule.
    pub matched_line: Option<usize>,
    pub matched_source: String,
}

fn expand_home_var(pattern: &str, home_dir: Option<&Path>) -> String {
    if let Some(home) = home_dir {
        let home_str = home.to_string_lossy();
        pattern.replace("$HOME", &home_str)
    } else if let Some(home) = env::var_os("HOME").filter(|v| !v.is_empty()) {
        pattern.replace("$HOME", &home.to_string_lossy())
    } else {
        pattern.to_string()
    }
}

fn glob_segment_matches(pat: &str, text: &str, ignore_case: bool) -> bool {
    let p_chars: Vec<char> = if ignore_case {
        pat.to_lowercase().chars().collect()
    } else {
        pat.chars().collect()
    };
    let t_chars: Vec<char> = if ignore_case {
        text.to_lowercase().chars().collect()
    } else {
        text.chars().collect()
    };
    glob_segment_rec(&p_chars, &t_chars)
}

fn glob_segment_rec(p: &[char], t: &[char]) -> bool {
    if p.is_empty() {
        return t.is_empty();
    }
    match p[0] {
        '*' => {
            for skip in 0..=t.len() {
                if glob_segment_rec(&p[1..], &t[skip..]) {
                    return true;
                }
            }
            false
        }
        '?' => !t.is_empty() && glob_segment_rec(&p[1..], &t[1..]),
        '[' => {
            if t.is_empty() {
                return false;
            }
            if let Some(close) = p[1..].iter().position(|&c| c == ']') {
                let class = &p[1..1 + close];
                let negate = class.first().is_some_and(|&c| c == '!' || c == '^');
                let inner = if negate { &class[1..] } else { class };
                let ch = t[0];
                let mut matched = false;
                let mut i = 0;
                while i < inner.len() {
                    if i + 2 < inner.len() && inner[i + 1] == '-' {
                        if inner[i] <= ch && ch <= inner[i + 2] {
                            matched = true;
                        }
                        i += 3;
                    } else {
                        if inner[i] == ch {
                            matched = true;
                        }
                        i += 1;
                    }
                }
                if negate {
                    matched = !matched;
                }
                matched && glob_segment_rec(&p[2 + close..], &t[1..])
            } else {
                t[0] == '[' && glob_segment_rec(&p[1..], &t[1..])
            }
        }
        c => !t.is_empty() && t[0] == c && glob_segment_rec(&p[1..], &t[1..]),
    }
}

fn glob_components_match(pat_comps: &[&str], path_comps: &[&str], ignore_case: bool) -> bool {
    if pat_comps.is_empty() {
        return path_comps.is_empty();
    }
    if pat_comps[0] == "**" {
        for skip in 0..=path_comps.len() {
            if glob_components_match(&pat_comps[1..], &path_comps[skip..], ignore_case) {
                return true;
            }
        }
        return false;
    }
    if path_comps.is_empty() {
        return false;
    }
    glob_segment_matches(pat_comps[0], path_comps[0], ignore_case)
        && glob_components_match(&pat_comps[1..], &path_comps[1..], ignore_case)
}

fn restic_pattern_matches(
    pattern: &str,
    path_comps: &[&str],
    match_ancestors: bool,
    ignore_case: bool,
) -> bool {
    let trimmed = strip_trailing_slash(pattern.trim());
    if trimmed.is_empty() {
        return false;
    }
    let pat_comps: Vec<&str> = trimmed
        .trim_start_matches('/')
        .split('/')
        .filter(|c| !c.is_empty())
        .collect();
    if pat_comps.is_empty() {
        return false;
    }

    let check_slice = |slice: &[&str]| -> bool {
        if pat_comps.len() == 1 && !trimmed.starts_with('/') && pat_comps[0] != "**" {
            // Unanchored single pattern matches any single basename component (when checking sub-slices)
            slice
                .last()
                .is_some_and(|seg| glob_segment_matches(pat_comps[0], seg, ignore_case))
        } else if trimmed.starts_with('/') {
            glob_components_match(&pat_comps, slice, ignore_case)
        } else {
            // Unanchored multi-component or wildcard pattern matches suffix OR full relative slice
            for start in 0..slice.len() {
                if glob_components_match(&pat_comps, &slice[start..], ignore_case) {
                    return true;
                }
            }
            false
        }
    };

    if check_slice(path_comps) {
        return true;
    }
    if match_ancestors {
        for prefix_len in 1..path_comps.len() {
            if check_slice(&path_comps[..prefix_len]) {
                return true;
            }
        }
    }
    false
}

struct ExcludeEvalState<'a> {
    abs_comps: Vec<&'a str>,
    rel_comps: Vec<&'a str>,
    home_dir: Option<&'a Path>,
    ignore_case: bool,
    excluded: bool,
    matched_rule: Option<String>,
    matched_line: Option<usize>,
    matched_source: String,
    ancestor_blocked_by_whole_dir: Option<(String, Option<usize>, String)>,
}

impl<'a> ExcludeEvalState<'a> {
    fn select_comps(&self, expanded: &str) -> &[&'a str] {
        if expanded.starts_with('/') {
            &self.abs_comps
        } else {
            &self.rel_comps
        }
    }

    fn apply_folder_rules(&mut self, folders: &[String]) {
        for folder in folders {
            let norm_folder = normalize_folder_rule(folder, self.home_dir);
            if norm_folder.is_empty() {
                continue;
            }
            let expanded = expand_home_var(&norm_folder, self.home_dir);
            let comps = self.select_comps(&expanded);
            if restic_pattern_matches(&expanded, comps, true, self.ignore_case) {
                self.excluded = true;
                self.matched_rule = Some(folder.clone());
                self.matched_line = None;
                self.matched_source = "folder".to_string();
                self.ancestor_blocked_by_whole_dir =
                    Some((folder.clone(), None, "folder".to_string()));
            }
        }
    }

    fn apply_trash_rules(&mut self) {
        for pat in TRASH_EXCLUDE_PATTERNS {
            let expanded = expand_home_var(pat, self.home_dir);
            let comps = self.select_comps(&expanded);
            if restic_pattern_matches(&expanded, comps, true, self.ignore_case) {
                self.excluded = true;
                self.matched_rule = Some(pat.to_string());
                self.matched_line = None;
                self.matched_source = "trash".to_string();
            }
        }
    }

    fn apply_expert_line(&mut self, line_no: usize, raw_line: &str) {
        let Some(parsed) = ExcludeRule::parse_line(raw_line) else {
            return;
        };
        match parsed {
            ExcludeRule::Pattern(pat) => {
                let expanded = expand_home_var(&pat, self.home_dir);
                let comps = self.select_comps(&expanded);
                let whole_dir = is_whole_dir_pattern(&expanded);
                let hits_strict_ancestor = self.abs_comps.len() > 1
                    && whole_dir
                    && comps.len() > 1
                    && restic_pattern_matches(
                        &expanded,
                        &comps[..comps.len() - 1],
                        true,
                        self.ignore_case,
                    );

                if restic_pattern_matches(&expanded, comps, whole_dir, self.ignore_case) {
                    self.excluded = true;
                    self.matched_rule = Some(pat.clone());
                    self.matched_line = Some(line_no);
                    self.matched_source = "expert".to_string();
                    if hits_strict_ancestor {
                        self.ancestor_blocked_by_whole_dir =
                            Some((pat, Some(line_no), "expert".to_string()));
                    }
                }
            }
            ExcludeRule::Negated(neg) => {
                let expanded = expand_home_var(&neg, self.home_dir);
                let comps = self.select_comps(&expanded);
                if restic_pattern_matches(&expanded, comps, false, self.ignore_case) {
                    if let Some((ref blocker_rule, blocker_line, ref blocker_src)) =
                        self.ancestor_blocked_by_whole_dir
                    {
                        self.excluded = true;
                        self.matched_rule = Some(blocker_rule.clone());
                        self.matched_line = blocker_line;
                        self.matched_source = blocker_src.clone();
                    } else {
                        self.excluded = false;
                        self.matched_rule = Some(format!("!{neg}"));
                        self.matched_line = Some(line_no);
                        self.matched_source = "expert".to_string();
                    }
                }
            }
        }
    }
}

/// Evaluates whether `test_path` would be backed up or excluded under `rules`.
pub fn evaluate_exclude_path(
    rules: &ExcludeRules,
    source_root: &Path,
    home_dir: Option<&Path>,
    test_path: &str,
) -> ExcludePathTestResult {
    let raw_test = test_path.trim();
    if raw_test.is_empty() {
        return ExcludePathTestResult {
            included: true,
            matched_rule: None,
            matched_line: None,
            matched_source: "empty".to_string(),
        };
    }

    let candidate_path = if Path::new(raw_test).is_absolute() {
        normalize_lexical_path(Path::new(raw_test))
    } else {
        normalize_lexical_path(&source_root.join(raw_test))
    };
    let candidate_str = candidate_path.to_string_lossy().into_owned();
    let abs_comps: Vec<&str> = candidate_str
        .trim_start_matches('/')
        .split('/')
        .filter(|c| !c.is_empty())
        .collect();

    let norm_src = normalize_lexical_path(source_root);
    let rel_str_owned = candidate_path
        .strip_prefix(&norm_src)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| candidate_str.clone());
    let rel_comps: Vec<&str> = rel_str_owned.split('/').filter(|c| !c.is_empty()).collect();

    let mut state = ExcludeEvalState {
        abs_comps,
        rel_comps,
        home_dir,
        ignore_case: rules.ignore_case,
        excluded: false,
        matched_rule: None,
        matched_line: None,
        matched_source: "none".to_string(),
        ancestor_blocked_by_whole_dir: None,
    };

    state.apply_folder_rules(&rules.folders);
    if rules.exclude_trash {
        state.apply_trash_rules();
    }
    for (idx, raw_line) in rules.lines.iter().enumerate() {
        state.apply_expert_line(idx + 1, raw_line);
    }

    ExcludePathTestResult {
        included: !state.excluded,
        matched_rule: state.matched_rule,
        matched_line: state.matched_line,
        matched_source: state.matched_source,
    }
}
