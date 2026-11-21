#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

// Tripwire for pull requests into a milestone branch (`m<n>`) and for the closing pull request of
// a milestone into the default branch. Purely mechanical: it reads the pull request, its diff and
// the file contents of base and head through the GitHub API, as text in memory. It never checks
// out, builds or runs code of the pull request and writes nothing to disk except the step summary.
//
// Environment: GITHUB_REPOSITORY (owner/name), PR_NUMBER, GITHUB_TOKEN (read access); optional
// GITHUB_API_URL and GITHUB_STEP_SUMMARY. Exit code 0: no finding, 1: findings, 2: error.
//
// Outside the CI (same checks, read-only):
//   GITHUB_REPOSITORY=Swissbite/kestic PR_NUMBER=42 GITHUB_TOKEN="$(gh auth token)" \
//     node .github/scripts/tripwire.mjs

import { appendFileSync } from 'node:fs';
import { gunzipSync } from 'node:zlib';

const API = process.env.GITHUB_API_URL ?? 'https://api.github.com';
const REPO = process.env.GITHUB_REPOSITORY;
const PR_NUMBER = process.env.PR_NUMBER;
const TOKEN = process.env.GITHUB_TOKEN;

// Paths a story pull request must not change. Changes to them go through the maintainer.
const PROTECTED = [
  /^AGENTS\.md$/,
  /^CLAUDE\.md$/,
  /^\.agents\/skills\//,
  /^\.claude\//,
  /^\.gemini\//,
  /^docs\/spec\//,
  /^\.github\//,
  /^justfile$/,
  /^clippy\.toml$/,
  /^deny\.toml$/,
  /^\.?rustfmt\.toml$/,
  /^rust-toolchain\.toml$/,
  /(^|\/)Cargo\.toml$/,
  /^Cargo\.lock$/,
  /^\.cargo\//,
  /^\.config\//,
  /^\.reuse\//,
  /^REUSE\.toml$/,
  /^LICENSES\//,
  /^scripts\/merge-into-milestone\.sh$/,
  /^site\/package\.json$/,
  /^site\/pnpm-lock\.yaml$/,
];
// Open questions for the maintainer may always be added.
const ALWAYS_ALLOWED = [/^docs\/spec\/questions\.md$/];
// May only change by adding test binaries (`binary(…)`) to existing filter lines.
const NEXTEST_CONFIG = '.config/nextest.toml';

const CODE_FILE = /\.(rs|cpp|h|qml|js|mjs|ts|astro)$/;
const CRATE_SOURCE = /^(core|daemon|gui|i18n|test-support)\/.*\.(rs|cpp|h)$/;
const TEST_PATH = /(^|\/)tests\/.*\.rs$|_tests\.rs$|^gui\/src\/smoke[^/]*\.(cpp|h)$|^test-support\/src\/.*\.rs$/;
const CFG_TEST = '#[cfg(test)]';

// Lint or test suppressions in added lines.
const SUPPRESSION =
  /#!?\[\s*(?:allow|expect|ignore)\b|cfg_attr\([^\n]*\b(?:allow|expect|ignore)\b|#\s*pragma\s+(?:GCC|clang|warning)|NOLINT|qmllint\s+disable|eslint-disable|@ts-(?:ignore|expect-error|nocheck)|-Wno-|["']-w["']/;
// Start of an assertion statement in Rust tests or in the C++ smoke harness.
const ASSERT_START =
  /\b(?:assert(?:_eq|_ne|_matches)?|debug_assert\w*|prop_assert\w*|panic|unreachable)!|\bSMOKE_(?:CHECK|FAIL)\w*\s*\(|\bQ(?:COMPARE|VERIFY|TRY_VERIFY|TRY_COMPARE|FAIL)\s*\(/;
const TEST_ATTR = /^\s*#\[\s*(?:[a-z_]+::)*test\b/;
const TIMEOUT_HINT = /timeout|deadline|Duration::from_|from_secs|from_millis|sleep|wait|retr(?:y|ies)|attempt|poll/i;

class Findings {
  constructor() {
    this.items = [];
    this.notes = [];
  }
  add(rule, file, detail) {
    this.items.push({ rule, file, detail });
  }
  note(text) {
    this.notes.push(text);
  }
}

async function api(path, { accept = 'application/vnd.github+json', raw = false } = {}) {
  const url = path.startsWith('http') ? path : `${API}${path}`;
  const res = await fetch(url, {
    headers: {
      Authorization: `Bearer ${TOKEN}`,
      Accept: accept,
      'X-GitHub-Api-Version': '2022-11-28',
      'User-Agent': 'kestic-tripwire',
    },
  });
  if (!res.ok) {
    throw new Error(`GET ${path}: HTTP ${res.status}`);
  }
  return raw ? Buffer.from(await res.arrayBuffer()) : res.json();
}

async function prFiles(number) {
  const files = [];
  for (let page = 1; page <= 30; page += 1) {
    const batch = await api(`/repos/${REPO}/pulls/${number}/files?per_page=100&page=${page}`);
    files.push(...batch);
    if (batch.length < 100) {
      return files;
    }
  }
  throw new Error('pull request has 3000 files or more; the API does not list all of them');
}

// Text files of a commit, read from its tarball in memory: path → content.
async function treeTexts(sha, wanted) {
  const tar = gunzipSync(await api(`/repos/${REPO}/tarball/${sha}`, { raw: true }));
  const files = new Map();
  let offset = 0;
  let paxPath = null;
  while (offset + 512 <= tar.length) {
    const header = tar.subarray(offset, offset + 512);
    if (header.every((b) => b === 0)) {
      break;
    }
    const field = (start, len) => {
      const bytes = header.subarray(start, start + len);
      const end = bytes.indexOf(0);
      return bytes.subarray(0, end < 0 ? len : end).toString('utf8');
    };
    const size = parseInt(field(124, 12).trim() || '0', 8);
    const type = header[156];
    const data = tar.subarray(offset + 512, offset + 512 + size);
    offset += 512 + Math.ceil(size / 512) * 512;
    if (type === 0x78 /* x */) {
      paxPath = paxRecords(data).get('path') ?? null;
      continue;
    }
    if (type === 0x67 /* g */) {
      continue;
    }
    const prefix = field(345, 155);
    const name = paxPath ?? (prefix ? `${prefix}/${field(0, 100)}` : field(0, 100));
    paxPath = null;
    if (type !== 0x30 && type !== 0) {
      continue;
    }
    // The tarball wraps everything in one top-level directory `<owner>-<repo>-<sha>/`.
    const path = name.slice(name.indexOf('/') + 1);
    if (wanted(path)) {
      files.set(path, data.toString('utf8'));
    }
  }
  return files;
}

function paxRecords(data) {
  const records = new Map();
  let rest = data.toString('utf8');
  while (rest.length > 0) {
    const space = rest.indexOf(' ');
    const len = parseInt(rest.slice(0, space), 10);
    if (!(len > 0)) {
      break;
    }
    const record = rest.slice(space + 1, len - 1);
    const eq = record.indexOf('=');
    records.set(record.slice(0, eq), record.slice(eq + 1));
    rest = rest.slice(len);
  }
  return records;
}

const wantedText = (path) =>
  CRATE_SOURCE.test(path) || /^docs\/spec\/milestones\/m\d+-[^/]*\.md$/.test(path);

function isTestBearing(path, content) {
  return TEST_PATH.test(path) || (path.endsWith('.rs') && (content ?? '').includes(CFG_TEST));
}

// Test code of a file: whole test files, otherwise everything from the first `#[cfg(test)]` on.
function testPart(path, content) {
  if (TEST_PATH.test(path)) {
    return content;
  }
  const at = path.endsWith('.rs') ? content.indexOf(CFG_TEST) : -1;
  return at < 0 ? '' : content.slice(at);
}

const normalize = (text) =>
  text
    // Formatting (line breaks, trailing commas) must not count as a change.
    .replace(/\s+/g, '')
    .replace(/,(?=[)\]}])/g, '')
    // Changed `use` paths must not count as changed assertions.
    .replace(/\b(?:[a-z_][a-z0-9_]*::)+/g, '');

function assertionStatements(content) {
  const lines = content.split('\n');
  const out = [];
  for (let i = 0; i < lines.length; i += 1) {
    if (!ASSERT_START.test(lines[i])) {
      continue;
    }
    let depth = 0;
    let opened = false;
    let j = i;
    for (; j < lines.length && j < i + 80; j += 1) {
      for (const ch of lines[j]) {
        if (ch === '(' || ch === '[' || ch === '{') {
          depth += 1;
          opened = true;
        } else if (ch === ')' || ch === ']' || ch === '}') {
          depth -= 1;
        }
      }
      if (opened && depth <= 0) {
        break;
      }
    }
    const end = Math.min(j, lines.length - 1);
    const joined = lines.slice(i, end + 1).join(' ');
    out.push({ start: i + 1, end: end + 1, text: normalize(joined), shown: joined.trim().replace(/\s+/g, ' ') });
    i = end;
  }
  return out;
}

const countTests = (content) => content.split('\n').filter((l) => TEST_ATTR.test(l)).length;

// Removed lines with their line number in the base version, and added lines.
function parsePatch(patch) {
  const removed = [];
  const added = [];
  let oldLine = 0;
  for (const line of patch.split('\n')) {
    const hunk = /^@@ -(\d+)(?:,\d+)? \+\d+(?:,\d+)? @@/.exec(line);
    if (hunk) {
      oldLine = Number(hunk[1]);
    } else if (line.startsWith('-')) {
      removed.push({ line: oldLine, text: line.slice(1) });
      oldLine += 1;
    } else if (line.startsWith('+')) {
      added.push(line.slice(1));
    } else if (!line.startsWith('\\')) {
      oldLine += 1;
    }
  }
  return { removed, added };
}

function milestoneAcs(texts, milestone) {
  const acs = [];
  for (const [path, content] of texts) {
    if (!path.startsWith(`docs/spec/milestones/m${milestone}-`)) {
      continue;
    }
    for (const line of content.split('\n')) {
      const m = /^- \*\*(AC-M(\d+)-(\d{2})-\d+)\*\*.*·\s*Test:\s*(.+?)\s*$/.exec(line);
      if (m && m[2] === String(milestone)) {
        acs.push({ id: m[1], story: `M${milestone}-S${m[3]}`, manualOnly: /^manuell\.?$/i.test(m[4]) });
      }
    }
  }
  return acs;
}

function referencedAcs(texts, acs) {
  const corpus = [...texts]
    .filter(([path]) => CRATE_SOURCE.test(path))
    .map(([path, content]) => testPart(path, content))
    .join('\n');
  const found = new Set();
  for (const ac of acs) {
    const snake = ac.id.toLowerCase().replaceAll('-', '_');
    if (new RegExp(`(?:${ac.id}|${snake})(?![0-9])`, 'i').test(corpus)) {
      found.add(ac.id);
    }
  }
  return found;
}

function checkNextestConfig(file, findings) {
  const { removed, added } = parsePatch(file.patch ?? '');
  const terms = (text) => new Set(text.match(/binary\([^)]*\)|package\([^)]*\)/g) ?? []);
  const skeleton = (text) => text.replace(/binary\([^)]*\)/g, '').replace(/[\s|]/g, '');
  const ok =
    file.status === 'modified' &&
    removed.length === added.length &&
    removed.every(({ text }, i) => {
      const before = terms(text);
      const after = terms(added[i]);
      return (
        text.trimStart().startsWith('filter =') &&
        skeleton(text) === skeleton(added[i]) &&
        [...before].every((t) => after.has(t))
      );
    });
  if (!ok) {
    findings.add('protected-path', file.filename, 'nextest config: only adding `binary(…)` to existing filters is allowed');
  }
}

// A sync merge `main → m<n>`: files that equal main's version and were not changed on the
// milestone branch since the merge base came from main unchanged; they skip the story rules.
async function syncedFromMain(pr, files, findings) {
  const head = await api(`/repos/${REPO}/commits/${pr.head.sha}`);
  const parents = head.parents.map((p) => p.sha);
  if (parents.length !== 2 || parents[0] !== pr.base.sha) {
    return new Set();
  }
  const main = pr.base.repo.default_branch;
  const toMain = await api(`/repos/${REPO}/compare/${parents[1]}...${main}`);
  if (toMain.status !== 'ahead' && toMain.status !== 'identical') {
    return new Set();
  }
  const mergeBase = (await api(`/repos/${REPO}/compare/${parents[1]}...${pr.base.sha}`)).merge_base_commit.sha;
  const tree = async (sha) => {
    const t = await api(`/repos/${REPO}/git/trees/${sha}?recursive=1`);
    if (t.truncated) {
      throw new Error(`tree of ${sha} is truncated`);
    }
    return new Map(t.tree.filter((e) => e.type === 'blob').map((e) => [e.path, e.sha]));
  };
  const [headTree, mainTree, baseTree, mergeBaseTree] = await Promise.all(
    [pr.head.sha, parents[1], pr.base.sha, mergeBase].map(tree),
  );
  const synced = new Set();
  for (const f of files) {
    const fromMain = headTree.get(f.filename) === mainTree.get(f.filename);
    const untouchedHere = baseTree.get(f.filename) === mergeBaseTree.get(f.filename);
    if (fromMain && untouchedHere) {
      synced.add(f.filename);
    }
  }
  findings.note(`Sync from \`${main}\` (${parents[1].slice(0, 7)}): ${synced.size} of ${files.length} files taken unchanged from \`${main}\`.`);
  return synced;
}

function checkDiff(files, baseTexts, headTexts, findings) {
  const headStatements = new Map();
  for (const [path, content] of headTexts) {
    if (CRATE_SOURCE.test(path) && isTestBearing(path, content)) {
      for (const s of assertionStatements(testPart(path, content))) {
        headStatements.set(s.text, (headStatements.get(s.text) ?? 0) + 1);
      }
    }
  }
  let testDelta = 0;
  for (const file of files) {
    const path = file.filename;
    const oldPath = file.previous_filename ?? path;
    for (const p of new Set([path, oldPath])) {
      if (p === NEXTEST_CONFIG) {
        checkNextestConfig(file, findings);
      } else if (PROTECTED.some((re) => re.test(p)) && !ALWAYS_ALLOWED.some((re) => re.test(p))) {
        findings.add('protected-path', p, `status ${file.status}`);
      }
    }
    if (!CODE_FILE.test(path)) {
      continue;
    }
    if (file.patch === undefined && file.changes > 0) {
      findings.add('diff-unavailable', path, 'GitHub returns no diff for this file; split the change');
      continue;
    }
    const { removed, added } = parsePatch(file.patch ?? '');
    for (const line of added) {
      if (SUPPRESSION.test(line)) {
        findings.add('suppression', path, line.trim());
      }
    }
    const before = baseTexts.get(oldPath);
    const after = headTexts.get(path);
    if (!CRATE_SOURCE.test(path) || (!isTestBearing(oldPath, before) && !isTestBearing(path, after))) {
      continue;
    }
    if (before !== undefined) {
      testDelta -= countTests(testPart(oldPath, before));
      const touched = new Set(removed.map((r) => r.line));
      const offset = before.length - testPart(oldPath, before).length;
      const lineShift = offset === 0 ? 0 : before.slice(0, offset).split('\n').length - 1;
      for (const s of assertionStatements(testPart(oldPath, before))) {
        let hit = file.status === 'removed';
        for (let n = s.start + lineShift; !hit && n <= s.end + lineShift; n += 1) {
          hit = touched.has(n);
        }
        if (!hit) {
          continue;
        }
        const left = headStatements.get(s.text) ?? 0;
        if (left > 0) {
          headStatements.set(s.text, left - 1);
        } else {
          findings.add('assertion-changed', `${oldPath}:${s.start + lineShift}`, s.shown.slice(0, 160));
        }
      }
    }
    if (after !== undefined) {
      testDelta += countTests(testPart(path, after));
    }
    const skeleton = (t) => normalize(t).replace(/\d+(?:\.\d+)?/g, '#');
    const numbers = (t) => (t.match(/\d+(?:\.\d+)?/g) ?? []).map(Number);
    for (const r of removed.filter((x) => TIMEOUT_HINT.test(x.text))) {
      for (const a of added) {
        if (a !== r.text && skeleton(a) === skeleton(r.text)) {
          const was = numbers(r.text);
          if (numbers(a).some((v, i) => v > was[i])) {
            findings.add('timeout-raised', `${path}:${r.line}`, `${r.text.trim()} → ${a.trim()}`);
          }
        }
      }
    }
  }
  if (testDelta < 0) {
    findings.add('test-count', '(changed files)', `${-testDelta} test function(s) fewer than before`);
  }
}

function checkCoverage({ acs, baseRefs, headRefs, stories, findings }) {
  for (const ac of acs) {
    if (baseRefs?.has(ac.id) && !headRefs.has(ac.id)) {
      findings.add('ac-coverage', ac.id, 'no test names this AC any more');
    }
    if (!ac.manualOnly && stories.has(ac.story) && !headRefs.has(ac.id)) {
      findings.add('ac-coverage', ac.id, `story ${ac.story} is named in the title, but no test names this AC`);
    }
  }
}

async function main() {
  if (!REPO || !PR_NUMBER || !TOKEN) {
    throw new Error('GITHUB_REPOSITORY, PR_NUMBER and GITHUB_TOKEN are required');
  }
  const pr = await api(`/repos/${REPO}/pulls/${PR_NUMBER}`);
  const findings = new Findings();
  const baseMilestone = /^m(\d+)$/.exec(pr.base.ref);
  const closing = pr.base.ref === pr.base.repo.default_branch
    ? /^(?:m(\d+)|claude\/m(\d+)-closing[\w.-]*)$/.exec(pr.head.ref)
    : null;
  if (!baseMilestone && !closing) {
    report(pr, findings, 'not a milestone pull request: nothing to check');
    return 0;
  }
  const milestone = Number(baseMilestone ? baseMilestone[1] : closing[1] ?? closing[2]);
  const [baseTexts, headTexts] = await Promise.all([
    treeTexts(pr.base.sha, wantedText),
    treeTexts(pr.head.sha, wantedText),
  ]);
  const acs = milestoneAcs(baseTexts, milestone);
  if (acs.length === 0) {
    throw new Error(`no acceptance criteria found for M${milestone} in docs/spec/milestones/`);
  }
  const headRefs = referencedAcs(headTexts, acs);

  if (closing) {
    checkCoverage({ acs, baseRefs: null, headRefs, stories: new Set(acs.map((a) => a.story)), findings });
    report(pr, findings, `closing pull request of M${milestone}: every non-manual AC needs a test`);
    return findings.items.length ? 1 : 0;
  }

  const files = await prFiles(pr.number);
  const synced = await syncedFromMain(pr, files, findings);
  checkDiff(files.filter((f) => !synced.has(f.filename)), baseTexts, headTexts, findings);
  const stories = new Set([...pr.title.matchAll(new RegExp(`\\bM${milestone}-S\\d{2}\\b`, 'g'))].map((m) => m[0]));
  checkCoverage({ acs, baseRefs: referencedAcs(baseTexts, acs), headRefs, stories, findings });
  const covered = acs.filter((a) => headRefs.has(a.id)).length;
  findings.note(`AC coverage of M${milestone} on head: ${covered} of ${acs.length} ACs named by a test (${acs.filter((a) => a.manualOnly).length} manual only).`);
  report(pr, findings, `pull request into \`${pr.base.ref}\`; stories in the title: ${[...stories].join(', ') || 'none'}`);
  return findings.items.length ? 1 : 0;
}

function report(pr, findings, context) {
  const lines = [`## Tripwire: #${pr.number}`, '', context, ''];
  for (const note of findings.notes) {
    lines.push(`- ${note}`);
  }
  if (findings.items.length === 0) {
    lines.push('', 'No findings.');
  } else {
    lines.push('', `**${findings.items.length} finding(s).** The maintainer decides on these.`, '', '| Rule | Where | Detail |', '| --- | --- | --- |');
    for (const f of findings.items) {
      const cell = (s) => String(s).replaceAll('|', '\\|').replaceAll('\n', ' ');
      lines.push(`| ${f.rule} | \`${cell(f.file)}\` | ${cell(f.detail)} |`);
      if (process.env.GITHUB_ACTIONS) {
        console.log(`::error title=tripwire ${f.rule}::${f.file}: ${String(f.detail).replaceAll('\n', ' ')}`);
      }
    }
  }
  const text = `${lines.join('\n')}\n`;
  console.log(text);
  if (process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, text);
  }
}

main().then(
  (code) => process.exit(code),
  (err) => {
    console.error(`tripwire: ${err.message}`);
    process.exit(2);
  },
);
