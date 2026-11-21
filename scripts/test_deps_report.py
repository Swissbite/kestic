#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
"""Offline tests for scripts/deps_report.py; run by `just verify`."""

from __future__ import annotations

import datetime as dt
import http.client
import json
import os
import tempfile
import unittest
from unittest import mock
from pathlib import Path
from typing import Any

import deps_report as dr

CARGO_UPDATE_OUTPUT = """\
    Updating crates.io index
    Updating git repository `https://invent.kde.org/libraries/cxx-kde-frameworks.git`
     Locking 3 packages to highest compatible versions
    Updating bytes v1.12.1 -> v1.12.2
    Updating toml_edit v0.25.19+spec-1.1.0 -> v0.26.0+spec-1.1.0
    Updating zbus v6.3.0 -> v6.5.0
      Adding newcrate v0.1.0
warning: not updating lockfile due to dry run
"""


class FakeSources(dr.Sources):
    """Answers from tables; names in `broken` fail like an unreachable source."""

    def __init__(self, **versions: Any) -> None:
        super().__init__("Swissbite/kestic", crates_delay=0)
        self.crates: dict[str, str] = versions.get("crates", {})
        self.releases: dict[str, str] = versions.get("releases", {})
        self.broken: set[str] = versions.get("broken", set())
        self.prs: list[dict[str, Any]] = versions.get("prs", [])

    def _answer(self, table: dict[str, str], name: str, default: str = "0.0.1") -> str:
        if name in self.broken:
            raise dr.SourceError("HTTP 503")
        return table.get(name, default)

    def crate_version(self, name: str) -> str:
        return self._answer(self.crates, name)

    def github_release(self, repo: str) -> str:
        return self._answer(self.releases, repo)

    def github_newest_tag(self, repo: str) -> str:
        return self._answer(self.releases, repo, "1")

    def npm_version(self, name: str) -> str:
        return self._answer(self.crates, name)

    def node_version(self) -> str:
        return self._answer(self.releases, "node")

    def ubuntu_lts(self) -> str:
        return self._answer(self.releases, "ubuntu", "20.04")

    def git_head(self, url: str, branch: str) -> str:
        return "1" * 40

    def gitlab_commit_titles(self, project: str, base: str, head: str) -> list[str]:
        return ["fix: thing for @someone in #12 | <b>x</b>"]

    def cxx_qt_fix_release(self, since: str) -> tuple[bool, str | None]:
        return True, self.releases.get("cxx-qt-fix")

    def cargo_update_dry_run(self) -> str:
        if "cargo" in self.broken:
            raise dr.SourceError("cargo fehlt")
        return CARGO_UPDATE_OUTPUT

    def dependabot_prs(self) -> list[dict[str, Any]]:
        return self.prs


FIXTURE_FILES = {
    "Cargo.toml": """\
[workspace]
members = ["app", "gui"]
[workspace.package]
rust-version = "1.100"
[workspace.dependencies]
zbus = "6.3.0"
cxx-qt = "=0.10.0"
cxx-qt-lib = { version = "=0.10.0", features = ["qt_full"] }
cxx-qt-build = "=0.10.0"
cxx-kde-frameworks = { git = "https://invent.kde.org/libraries/cxx-kde-frameworks.git", rev = "5555555555555555555555555555555555555555" }
""",
    "app/Cargo.toml": """\
[dependencies]
zbus = { workspace = true }
rusqlite = { version = "0.43", default-features = false }
serde = "1.0"
own = { path = "../own" }
[dev-dependencies]
tempfile = "3.27"
""",
    "gui/Cargo.toml": """\
[package]
name = "gui"
[dependencies]
cxx-qt = { workspace = true }
cxx-qt-lib = { workspace = true }
cxx-kde-frameworks = { workspace = true }
[build-dependencies]
cxx-qt-build = { workspace = true }
[package.metadata.deb]
depends = "$auto, qml6-module-org-kde-kirigami (>= 6.24)"
recommends = "restic (>= 0.18.0)"
""",
    "Cargo.lock": "version = 4\n"
    + "".join(
        f'[[package]]\nname = "{name}"\nversion = "{version}"\n'
        'source = "registry+https://github.com/rust-lang/crates.io-index"\n'
        for name, version in [
            ("zbus", "6.3.0"),
            ("rusqlite", "0.43.0"),
            ("serde", "1.0.228"),
            ("tempfile", "3.27.0"),
            ("cxx-qt", "0.10.0"),
            ("cxx-qt-lib", "0.10.0"),
            ("cxx-qt-build", "0.10.0"),
            ("qt-build-utils", "0.10.0"),
        ]
    ),
    "rust-toolchain.toml": '[toolchain]\nchannel = "1.100.0"\n',
    "scripts/bootstrap.sh": 'RESTIC_MIN_VERSION="0.18.0"\nRESTIC_LATEST_VERSION="0.19.1"\nRCLONE_VERSION="1.76.0"\n',
    ".github/workflows/verify.yml": """\
jobs:
  verify:
    uses: ./.github/workflows/ci-image.yml
    steps:
      - uses: actions/checkout@v7
      - name: Cache
        uses: actions/cache@v6
""",
    ".github/ci-image/Dockerfile": "FROM ubuntu:26.04\nRUN true\n",
    "site/package.json": """\
{"packageManager": "pnpm@12.21.0",
 "devEngines": {"runtime": {"name": "node", "version": "26.13.1"}},
 "dependencies": {"astro": "7.4.13"},
 "devDependencies": {"typescript": "6.0.3"}}
""",
}


def write_fixture(root: Path, skip: tuple[str, ...] = ()) -> Path:
    """A small repository with every file the report reads."""
    for name, content in FIXTURE_FILES.items():
        if name not in skip:
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
    return root


def report_for(sources: FakeSources, skip: tuple[str, ...] = ()) -> str:
    with tempfile.TemporaryDirectory() as tmp:
        return dr.build_report(write_fixture(Path(tmp), skip), sources, dt.date(2026, 11, 21)).body


def table_line(body: str, component: str) -> str:
    """The table row of `component` in the report."""
    return next(line for line in body.splitlines() if line.startswith(f"| {component} |"))


class ClassifyTest(unittest.TestCase):
    def test_semver_kinds_from_1_0(self) -> None:
        self.assertEqual(dr.classify("1.2.3", "2.0.0"), dr.MAJOR)
        self.assertEqual(dr.classify("1.2.3", "1.3.0"), dr.MINOR)
        self.assertEqual(dr.classify("1.2.3", "1.2.4"), dr.PATCH)
        self.assertEqual(dr.classify("1.2.3", "1.2.3"), dr.NONE)
        self.assertEqual(dr.classify("1.2.3", "1.2.2"), dr.NONE)

    def test_new_minor_below_1_0_is_major(self) -> None:
        self.assertEqual(dr.classify("0.43.0", "0.44.0"), dr.MAJOR)
        self.assertEqual(dr.classify("0.19.1", "v0.20.0"), dr.MAJOR)
        self.assertEqual(dr.classify("0.43.0", "0.43.1"), dr.PATCH)
        self.assertEqual(dr.classify("0.0.3", "0.0.4"), dr.MAJOR)
        self.assertEqual(dr.classify("0.9.0", "1.0.0"), dr.MAJOR)

    def test_short_prefixed_and_build_metadata_versions(self) -> None:
        self.assertEqual(dr.classify("1.100", "1.101.0"), dr.MINOR)
        self.assertEqual(dr.classify("v7", "7.1.0"), dr.MINOR)
        self.assertEqual(dr.classify("1.2.1+spec-1.1.0", "1.2.2+spec-1.1.0"), dr.PATCH)
        self.assertEqual(dr.classify("26.04", "28.04"), dr.MAJOR)

    def test_release_sorts_after_its_prerelease(self) -> None:
        self.assertEqual(dr.classify("1.0.0-rc.1", "1.0.0"), dr.PATCH)
        self.assertEqual(dr.classify("1.0.0", "1.0.0-rc.1"), dr.NONE)

    def test_unreadable_version_is_a_source_error(self) -> None:
        with self.assertRaises(dr.SourceError):
            dr.classify("1.0.0", "latest")

    def test_newest_stable_skips_prereleases_and_other_tags(self) -> None:
        tags = ["v4.31.2", "codeql-bundle-v2.23.0", "v4.32.0-rc.1", "v4", "v3.9.9"]
        self.assertEqual(dr.newest_stable(tags), "4.31.2")
        self.assertEqual(dr.newest_stable(["v27.0.0-alpha.0", "v26.13.1", "v24.11.0"]), "26.13.1")
        with self.assertRaises(dr.SourceError):
            dr.newest_stable(["nightly"])


class CargoTest(unittest.TestCase):
    def test_locked_version_matching_the_requirement(self) -> None:
        versions = ["0.9.1", "1.0.5", "1.0.7", "2.0.0"]
        self.assertEqual(dr.locked_for("1.0", versions), "1.0.7")
        self.assertEqual(dr.locked_for("=0.9.1", versions), "0.9.1")
        self.assertEqual(dr.locked_for("0.9", versions), "0.9.1")
        self.assertIsNone(dr.locked_for("3", versions))
        self.assertEqual(dr.locked_for("1.0", ["1.2.1+spec-1.1.0"]), "1.2.1+spec-1.1.0")

    def test_direct_dependencies_of_workspace_and_members(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["app"]\n[workspace.dependencies]\n'
                'zbus = "6.3.0"\n'
                'kde = { git = "https://example.invalid/kde.git", rev = "abc" }\n'
            )
            (root / "app").mkdir()
            (root / "app" / "Cargo.toml").write_text(
                "[dependencies]\nzbus = { workspace = true }\nown = { path = \"../own\" }\n"
                'renamed = { package = "real-name", version = "1.2" }\n'
                '[dev-dependencies]\ntempfile = "3.27"\n'
                '[build-dependencies]\ncc = "1.0"\n'
                "[target.'cfg(unix)'.dependencies]\nlibc = \"0.2\"\n"
            )
            deps = dr.cargo_dependencies(root)
        self.assertEqual(sorted(deps), ["cc", "kde", "libc", "real-name", "tempfile", "zbus"])
        self.assertEqual(deps["zbus"].requirements, {"6.3.0"})
        self.assertEqual(deps["kde"].rev, "abc")
        self.assertEqual(deps["real-name"].requirements, {"1.2"})

    def test_cargo_update_output(self) -> None:
        self.assertEqual(
            dr.parse_cargo_update(CARGO_UPDATE_OUTPUT),
            [
                ("bytes", "1.12.1", "1.12.2"),
                ("toml_edit", "0.25.19+spec-1.1.0", "0.26.0+spec-1.1.0"),
                ("zbus", "6.3.0", "6.5.0"),
            ],
        )


class DependabotTest(unittest.TestCase):
    def test_updates_from_single_and_group_prs(self) -> None:
        prs = [
            {"number": 23, "title": "Bump actions/cache from 5 to 6", "body": ""},
            {
                "number": 24,
                "title": "Bump the cargo-minor-and-patch group with 2 updates",
                "body": "Bumps the group.\nUpdates `zbus` from 6.3.0 to 6.5.0\n"
                "Updates `jiff` from 1.0.1 to 1.0.2\n",
            },
        ]
        self.assertEqual(dr.dependabot_updates(prs), {"actions/cache": 23, "zbus": 24, "jiff": 24})

    def test_ci_status(self) -> None:
        green = [{"status": "COMPLETED", "conclusion": "SUCCESS"}, {"state": "SUCCESS"}]
        red = [{"status": "COMPLETED", "conclusion": "FAILURE"}, {"status": "IN_PROGRESS", "conclusion": ""}]
        running = [{"status": "IN_PROGRESS", "conclusion": ""}, {"state": "SUCCESS"}]
        self.assertEqual(dr.ci_status(green), "✅ grün")
        self.assertEqual(dr.ci_status(red), "❌ rot")
        self.assertEqual(dr.ci_status(running), "⏳ läuft")
        self.assertEqual(dr.ci_status([]), "keine Checks")
        self.assertEqual(dr.ci_status(None), "nicht prüfbar")


class MarkdownTest(unittest.TestCase):
    def test_outside_text_cannot_mention_reference_or_format(self) -> None:
        text = dr.md_text("hi @octocat see #12 | <script>`x`</script> [a](b)")
        self.assertNotIn("@octocat", text)
        self.assertNotIn("#12", text)
        self.assertNotIn("<script>", text)
        self.assertIn("\\|", text)
        self.assertIn("\\`", text)
        self.assertIn("\\[a\\]", text)

    def test_link_rejects_unsafe_targets(self) -> None:
        self.assertEqual(dr.link("npm", "https://www.npmjs.com/package/@a/b"), "[npm](https://www.npmjs.com/package/@a/b)")
        with self.assertRaises(ValueError):
            dr.link("x", "javascript:alert(1)")
        with self.assertRaises(ValueError):
            dr.link("x", "https://example.org/a) [b](c")


class IssueTest(unittest.TestCase):
    def test_title_uses_iso_week_and_iso_year(self) -> None:
        self.assertEqual(dr.issue_title(dt.date(2026, 11, 21)), "Abhängigkeiten KW 47 2026")
        self.assertEqual(dr.issue_title(dt.date(2027, 1, 1)), "Abhängigkeiten KW 53 2026")

    def test_only_older_open_reports_are_superseded(self) -> None:
        issues = [
            {"number": 10, "title": "Abhängigkeiten KW 46 2026"},
            {"number": 12, "title": "Abhängigkeiten KW 47 2026"},
            {"number": 9, "title": "Abhängigkeiten KWx"},
            {"number": 8, "title": "M2: Backup Configuration"},
            {"number": 30, "title": "Abhängigkeiten KW 48 2026"},
        ]
        self.assertEqual(dr.superseded_issues(issues, 20), [10, 12])


class ReportTest(unittest.TestCase):
    def test_unreachable_source_marks_only_its_row(self) -> None:
        body = report_for(FakeSources(crates={"zbus": "6.5.0"}, broken={"serde", "cargo"}))
        self.assertIn("nicht prüfbar: HTTP 503", table_line(body, "serde"))
        self.assertIn("| zbus | `6.3.0` | `6.5.0` | Minor |", body)
        self.assertIn("## Lockfile (`cargo update --dry-run`)\n\nnicht prüfbar: cargo fehlt", body)
        self.assertIn("- Nicht prüfbar:", body)

    def test_update_with_dependabot_pr_points_to_it(self) -> None:
        prs = [{"number": 23, "title": "Bump zbus from 6.3.0 to 6.5.0", "body": "", "statusCheckRollup": []}]
        body = report_for(FakeSources(crates={"zbus": "6.5.0"}, prs=prs))
        self.assertIn("→ #23", table_line(body, "zbus"))
        self.assertIn("- #23 Bump zbus from 6.3.0 to 6.5.0 – CI: keine Checks", body)
        self.assertIn("1 Update(s) mit offenem Dependabot-PR", body)

    def test_requirement_decides_between_cargo_update_and_manifest_change(self) -> None:
        body = report_for(FakeSources(crates={"zbus": "6.5.0", "rusqlite": "0.44.0"}))
        self.assertIn("per `cargo update`", table_line(body, "zbus"))
        self.assertIn("**Major**", table_line(body, "rusqlite"))
        self.assertIn("Anforderung `0.43` anheben", table_line(body, "rusqlite"))

    def test_lockfile_lists_transitive_updates_only(self) -> None:
        body = report_for(FakeSources())
        self.assertIn("| toml\\_edit | `0.25.19+spec-1.1.0` | `0.26.0+spec-1.1.0` | **Major** |", body)
        self.assertIn("| bytes | `1.12.1` | `1.12.2` | Patch |", body)
        lockfile = body.split("## Lockfile")[1].split("## Gepinnt")[0]
        self.assertNotIn("| zbus |", lockfile)

    def test_summary_highlights_major_and_pinned_updates(self) -> None:
        sources = FakeSources(
            crates={"cxx-qt": "0.11.0"},
            releases={"restic/restic": "0.20.0", "cxx-qt-fix": "v0.11.0"},
        )
        summary = report_for(sources).split("## Offene Dependabot-PRs")[0]
        self.assertIn("**Handlung:**", summary)
        self.assertIn("**restic (aktuell)** 0.19.1 → 0.20.0", summary)
        self.assertNotIn("restic (Mindestversion)", summary)
        self.assertIn("**Gepinnt, nur Stop & Ask:**", summary)
        self.assertIn("**cxx-qt** → 0.11.0", summary)
        self.assertIn("**cxx-kde-frameworks** → 11111111", summary)

    def test_cxx_qt_fix_release_and_commit_titles(self) -> None:
        body = report_for(FakeSources(releases={"cxx-qt-fix": "v0.11.0"}))
        self.assertIn("in `v0.11.0` enthalten", body)
        self.assertIn("1 Commit(s) neuer auf `master`", body)
        self.assertIn("- fix: thing for @​someone in #​12 \\| &lt;b&gt;x&lt;/b&gt;", body)

    def test_minimum_restic_version_stays(self) -> None:
        body = report_for(FakeSources(releases={"restic/restic": "0.20.0"}))
        line = table_line(body, "restic (Mindestversion)")
        self.assertIn("| `0.18.0` | `0.20.0` | – |", line)
        self.assertIn("bleibt", line)

    def test_major_tag_of_an_action_follows_minor_releases(self) -> None:
        body = report_for(FakeSources(releases={"actions/checkout": "7.2.0", "actions/cache": "7.0.0"}))
        self.assertIn("**Major**", table_line(body, "actions/cache"))
        self.assertNotIn("| actions/checkout |", body.split("<details>")[0])

    def test_unknown_bootstrap_version_is_reported(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            script = Path(tmp) / "scripts" / "bootstrap.sh"
            script.parent.mkdir()
            script.write_text('RESTIC_MIN_VERSION="0.18.0"\nNEW_TOOL_VERSION="1.0.0"\n')
            rows = dr.bootstrap_rows(Path(tmp), FakeSources())
        self.assertEqual([r.component for r in rows], ["NEW_TOOL_VERSION", "restic (Mindestversion)"])
        self.assertTrue(rows[0].failed)


class RecordingSources(FakeSources):
    """Answers `gh` calls of the issue workflow and records them."""

    def __init__(self, open_issues: list[dict[str, Any]], failing_close: int | None = None) -> None:
        super().__init__()
        self.open_issues = open_issues
        self.failing_close = failing_close
        self.calls: list[tuple[list[str], str | None]] = []

    def gh_api(self, path: str) -> Any:
        raise dr.SourceError("HTTP 404")

    def run(self, args: list[str], timeout: int = 60, stdin: str | None = None, with_stderr: bool = False) -> str:
        self.calls.append((args, stdin))
        if args[:3] == ["gh", "api", "-X"]:
            return json.dumps({"number": 20, "html_url": "https://github.com/Swissbite/kestic/issues/20"})
        if args[:3] == ["gh", "issue", "list"]:
            return json.dumps(self.open_issues)
        if args[:3] == ["gh", "issue", "close"] and args[3] == str(self.failing_close):
            raise dr.SourceError("HTTP 502")
        return ""


class PublishTest(unittest.TestCase):
    OPEN = [
        {"number": 10, "title": "Abhängigkeiten KW 46 2026"},
        {"number": 5, "title": "CI: run verify in a prebuilt container image"},
    ]

    def test_creates_issue_without_missing_label_and_closes_old_report(self) -> None:
        sources = RecordingSources(self.OPEN)
        with mock.patch("sys.stdout"):
            status = dr.publish(dr.Report("Abhängigkeiten KW 47 2026", "Inhalt"), sources)
        payload = json.loads(sources.calls[0][1] or "{}")
        closes = [args for args, _ in sources.calls if args[:3] == ["gh", "issue", "close"]]
        self.assertEqual(status, 0)
        self.assertEqual(payload, {"title": "Abhängigkeiten KW 47 2026", "body": "Inhalt", "labels": []})
        self.assertEqual(len(closes), 1)
        self.assertEqual(closes[0][3], "10")
        self.assertEqual(closes[0][closes[0].index("--comment") + 1], "Abgelöst durch #20")

    def test_failed_close_is_a_warning_only(self) -> None:
        sources = RecordingSources(self.OPEN, failing_close=10)
        with mock.patch("sys.stdout") as stdout:
            status = dr.publish(dr.Report("t", "b"), sources)
        printed = "".join(call.args[0] for call in stdout.write.call_args_list)
        self.assertEqual(status, 0)
        self.assertIn("::warning::Issue #10 nicht geschlossen", printed)


class SourcesTest(unittest.TestCase):
    def setUp(self) -> None:
        self.sources = dr.Sources("Swissbite/kestic", root=Path(tempfile.gettempdir()), crates_delay=0)

    def test_tool_failures_become_source_errors(self) -> None:
        with self.assertRaisesRegex(dr.SourceError, "fehlt"):
            self.sources.run(["kestic-no-such-tool"])
        with self.assertRaisesRegex(dr.SourceError, "^sh -c: boom$"):
            self.sources.run(["sh", "-c", "echo boom >&2; exit 3"])
        with self.assertRaisesRegex(dr.SourceError, "Zeitüberschreitung"):
            self.sources.run(["sleep", "5"], timeout=0.1)

    def test_github_token_reaches_only_gh(self) -> None:
        with mock.patch.dict(os.environ, {"GH_TOKEN": "secret", "GITHUB_TOKEN": "secret"}):
            output = self.sources.run(["sh", "-c", 'echo "${GH_TOKEN:-none} ${GITHUB_TOKEN:-none}"'])
        self.assertEqual(output.strip(), "none none")

    def test_broken_http_response_becomes_source_error(self) -> None:
        with mock.patch("urllib.request.urlopen", side_effect=http.client.IncompleteRead(b"")):
            with self.assertRaisesRegex(dr.SourceError, "nicht erreichbar"):
                self.sources.crate_version("serde")

    def test_node_compares_with_lts_only(self) -> None:
        releases = [{"version": "v27.1.0", "lts": False}, {"version": "v26.13.1", "lts": "Krypton"}]
        with mock.patch.object(dr.Sources, "_json", return_value=releases):
            self.assertEqual(self.sources.node_version(), "26.13.1")

    def test_repository_names_cannot_climb_paths(self) -> None:
        with self.assertRaises(dr.SourceError):
            dr.checked_repo("actions/..")
        self.assertEqual(dr.checked_repo("actions/checkout"), "actions/checkout")


class RobustnessTest(unittest.TestCase):
    def test_broken_section_leaves_the_others(self) -> None:
        body = report_for(FakeSources(crates={"zbus": "6.5.0"}), skip=("site/package.json",))
        self.assertIn("## Showcase-Seite (`site/`)\n\nnicht prüfbar: FileNotFoundError", body)
        self.assertIn("| zbus | `6.3.0` | `6.5.0` | Minor |", body)
        self.assertIn("## Mindestversionen im `.deb`", body)

    def test_too_long_body_drops_whole_sections(self) -> None:
        with mock.patch.object(dr, "MAX_BODY_CHARS", 25):
            body = dr.fit_body(["## A\n\nkurz", "## B\n\n" + "x" * 30, "## C"])
        self.assertEqual(body, "## A\n\nkurz\n\n… gekürzt: 2 Abschnitt(e) fehlen (Grenze von GitHub).")

    def test_deb_minimum_versions_are_listed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            text = dr.deb_section(write_fixture(Path(tmp)))
        self.assertIn("- Depends: `qml6-module-org-kde-kirigami (>= 6.24)`", text)
        self.assertIn("- Recommends: `restic (>= 0.18.0)`", text)
        self.assertIn("`$auto`", text)

    def test_lowest_requirement_compares_versions(self) -> None:
        self.assertEqual(dr.lowest({"0.10", "0.9"}), "0.9")
        self.assertEqual(dr.lowest({"v6", "v10"}), "v6")

    def test_pinned_crate_missing_from_lockfile_is_not_checkable(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            rows = dr.cxx_qt_rows(write_fixture(Path(tmp)), FakeSources())
        missing = next(r for r in rows if r.component == "cxx-qt-lib-extras")
        self.assertTrue(missing.failed)
        self.assertEqual(missing.kind, dr.NONE)

    def test_pipe_in_reason_stays_inside_its_cell(self) -> None:
        line = dr.table([dr.unchecked("x", "1.0.0", "a | b")])
        self.assertTrue(line.endswith("nicht prüfbar: a \\| b |"))


if __name__ == "__main__":
    unittest.main()
