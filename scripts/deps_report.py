#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
"""Weekly dependency report for Kestic.

Compares the versions Kestic uses (Cargo manifests and lockfile, the pinned CXX-Qt crates
and cxx-kde-frameworks commit, the Rust toolchain, the test tools in scripts/bootstrap.sh,
the CI image, GitHub Actions and the showcase site) with the newest releases and renders
a German Markdown report. Without options it prints the report; with --create-issue it
opens it as a GitHub issue and closes the previous reports. It never changes a file.

Every value from an outside source is validated before it reaches the report or another
request. A source that fails or answers unexpectedly turns its row into
"nicht prüfbar: <Grund>"; the run itself does not fail because of it.

Standard library only, so it runs on a stock `ubuntu-latest` runner and in `just verify`.
"""

from __future__ import annotations

import argparse
import datetime as dt
import functools
import html
import http.client
import json
import os
import re
import subprocess
import sys
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_REPO = "Swissbite/kestic"
USER_AGENT = "kestic-deps-report (https://github.com/Swissbite/kestic)"
TITLE_PREFIX = "Abhängigkeiten KW"
LABEL = "dependencies"
HTTP_TIMEOUT = 20
MAX_RESPONSE_BYTES = 4 * 1024 * 1024
# crates.io asks automated clients for at most one request per second.
CRATES_IO_DELAY = 1.0
# GitHub rejects issue bodies above 65536 characters.
MAX_BODY_CHARS = 60000

MAJOR, MINOR, PATCH, NONE = "Major", "Minor", "Patch", "–"
KIND_ORDER = {MAJOR: 0, MINOR: 1, PATCH: 2, NONE: 3}

PINNED_CRATES = ("cxx-qt", "cxx-qt-lib", "cxx-qt-lib-extras", "cxx-qt-build", "qt-build-utils")
CXX_QT_REPO = "KDAB/cxx-qt"
CXX_QT_CHANGELOG = "https://github.com/KDAB/cxx-qt/blob/main/CHANGELOG.md"
# Kestic's QML plugin ships a stale plugin.qmltypes with cxx-qt 0.10.0 (issue 1508); the
# report shows the first release with the fix (PR 1509) until the pin moves past it.
CXX_QT_FIX_PR = 1509
KDE_FRAMEWORKS_URL = "https://invent.kde.org/libraries/cxx-kde-frameworks.git"
KDE_FRAMEWORKS_PROJECT = "libraries/cxx-kde-frameworks"
KDE_FRAMEWORKS_BRANCH = "master"
MAX_COMMIT_TITLES = 30

# Version variable in scripts/bootstrap.sh -> (component, source kind, source name).
BOOTSTRAP_TOOLS = {
    "RESTIC_MIN_VERSION": ("restic (Mindestversion)", "github", "restic/restic"),
    "RESTIC_LATEST_VERSION": ("restic (aktuell)", "github", "restic/restic"),
    "REST_SERVER_VERSION": ("rest-server", "github", "restic/rest-server"),
    "RCLONE_VERSION": ("rclone", "github", "rclone/rclone"),
    "CARGO_DENY_VERSION": ("cargo-deny", "crate", "cargo-deny"),
    "CARGO_DEB_VERSION": ("cargo-deb", "crate", "cargo-deb"),
    "CARGO_NEXTEST_VERSION": ("cargo-nextest", "crate", "cargo-nextest"),
}
BOOTSTRAP_NOTES = {
    "RESTIC_MIN_VERSION": "bleibt: Tests laufen gegen die Mindestversion und die aktuelle Version",
    "CARGO_NEXTEST_VERSION": "SHA-256 in `bootstrap.sh` mitziehen",
}

VERSION_RE = re.compile(
    r"^v?(\d+)(?:\.(\d+))?(?:\.(\d+))?(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$"
)
SAFE_VERSION_RE = re.compile(r"^[0-9A-Za-z.+-]{1,64}$")
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
CRATE_RE = re.compile(r"^[A-Za-z0-9_-]{1,64}$")
NPM_RE = re.compile(r"^(@[a-z0-9._-]+/)?[a-z0-9._-]{1,214}$")
GH_REPO_RE = re.compile(r"^[A-Za-z0-9_-][A-Za-z0-9_.-]{0,99}/[A-Za-z0-9_-][A-Za-z0-9_.-]{0,99}$")
USES_RE = re.compile(r"^\s*(?:-\s*)?uses:\s*([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)[^@\s]*@(\S+)")
CARGO_UPDATE_RE = re.compile(r"^\s*(?:Updating|Downgrading)\s+([A-Za-z0-9_-]+) v(\S+) -> v(\S+)\s*$")
BOOTSTRAP_VERSION_RE = re.compile(r'^([A-Z][A-Z0-9_]*_VERSION)="([^"]*)"\s*$')
DEPENDABOT_BODY_RE = re.compile(r"^Updates `([^`\s]+)` from (\S+) to (\S+)", re.MULTILINE)
DEPENDABOT_TITLE_RE = re.compile(r"^Bump (\S+) from (\S+) to (\S+)")
# Credentials that only the GitHub CLI needs; cargo and git run without them.
GH_ONLY_ENV = ("GH_TOKEN", "GITHUB_TOKEN")
FAILED_CHECKS = {"FAILURE", "CANCELLED", "TIMED_OUT", "ACTION_REQUIRED", "ERROR", "STARTUP_FAILURE"}
PENDING_CHECKS = {"PENDING", "EXPECTED", "QUEUED", "IN_PROGRESS", "WAITING", "REQUESTED"}


class SourceError(Exception):
    """A source could not be read; the message becomes the reason in the report."""


# --- Versions -------------------------------------------------------------------------


@dataclass(frozen=True)
class Version:
    major: int
    minor: int
    patch: int
    pre: str = ""

    def key(self) -> tuple[int, int, int, bool, str]:
        # A release sorts after its pre-releases.
        return (self.major, self.minor, self.patch, self.pre == "", self.pre)

    def compat_key(self) -> tuple[int, ...]:
        """Cargo's compatibility range: the leftmost non-zero component."""
        if self.major > 0:
            return (self.major,)
        if self.minor > 0:
            return (0, self.minor)
        return (0, 0, self.patch)


def parse_version(text: str) -> Version | None:
    match = VERSION_RE.match(text.strip())
    if not match:
        return None
    major, minor, patch, pre = match.groups()
    return Version(int(major), int(minor or 0), int(patch or 0), pre or "")


def require_version(text: str) -> Version:
    version = parse_version(text)
    if version is None:
        raise SourceError(f"Version «{text}» nicht lesbar")
    return version


def classify(current: str, newest: str) -> str:
    """Semver kind of the step from `current` to `newest`.

    Below 1.0, the leftmost non-zero component carries breaking changes: a new minor
    of 0.x (and any change of 0.0.x) counts as Major, a new patch of 0.x as Patch.
    """
    cur, new = require_version(current), require_version(newest)
    if new.key() <= cur.key():
        return NONE
    if new.compat_key() != cur.compat_key():
        return MAJOR
    if new.minor != cur.minor:
        return MINOR
    return PATCH


def checked_version(value: Any) -> str:
    if isinstance(value, str) and SAFE_VERSION_RE.match(value) and parse_version(value):
        return value
    raise SourceError("unerwartete Versionsangabe")


def newest_stable(candidates: list[Any]) -> str:
    """Highest valid version without pre-release part, without a leading `v`."""
    stable = [
        (version, text)
        for text in candidates
        if isinstance(text, str) and SAFE_VERSION_RE.match(text)
        for version in [parse_version(text)]
        if version and not version.pre
    ]
    if not stable:
        raise SourceError("keine stabile Version gefunden")
    return max(stable, key=lambda pair: pair[0].key())[1].removeprefix("v")


def lowest(requirements: set[str]) -> str:
    """Lowest of several version requirements or refs, compared as versions."""
    zero = Version(0, 0, 0)
    return min(requirements, key=lambda t: ((parse_version(t.lstrip("=^~")) or zero).key(), t), default="")


def checked_repo(repo: str) -> str:
    if not GH_REPO_RE.match(repo):
        raise SourceError("ungültiger Repository-Name")
    return repo


def checked_sha(value: Any) -> str:
    if isinstance(value, str) and SHA_RE.match(value):
        return value
    raise SourceError("unerwartete Commit-ID")


def locked_for(requirement: str, versions: list[str]) -> str | None:
    """Locked version that satisfies a Cargo requirement (caret or `=`); None if unknown."""
    req = requirement.strip()
    if req.startswith("="):
        exact = req[1:].strip()
        return exact if exact in versions else None
    base = parse_version(req.removeprefix("^"))
    if base is None:
        return None
    matching = [
        v
        for v in (parse_version(text) for text in versions)
        if v and v.compat_key() == base.compat_key() and v.key() >= base.key()
    ]
    if not matching:
        return None
    best = max(matching, key=Version.key)
    return next(text for text in versions if parse_version(text) == best)


# --- Markdown -------------------------------------------------------------------------


def md_text(value: object, limit: int = 160) -> str:
    """Plain text for Markdown: no HTML, no formatting, no @mentions or #references."""
    text = "".join(ch if ch.isprintable() else " " for ch in str(value))
    if len(text) > limit:
        text = text[: limit - 1] + "…"
    text = html.escape(text, quote=False)
    text = re.sub(r"([\\`*_\[\]|~])", r"\\\1", text)
    return text.replace("@", "@\u200b").replace("#", "#\u200b")


def md_code(value: str) -> str:
    return f"`{value}`" if value and value != NONE else NONE


def link(label: str, url: str) -> str:
    if not re.fullmatch(r"https://[A-Za-z0-9._~:/?&=%+@-]+", url):
        raise ValueError(f"unsafe link target: {url!r}")
    return f"[{md_text(label)}]({url})"


@dataclass
class Row:
    component: str
    used: str
    newest: str = NONE
    kind: str = NONE
    source: str = ""
    note: str = ""
    pinned: bool = False
    name: str = ""
    failed: bool = False
    update: bool | None = None
    pr: int | None = None

    def __post_init__(self) -> None:
        if self.update is None:
            self.update = self.kind != NONE


def unchecked(component: str, used: str, reason: object, **extra: Any) -> Row:
    return Row(component, used, note=f"nicht prüfbar: {md_text(reason)}", failed=True, **extra)


def join_notes(*notes: str) -> str:
    return "; ".join(note for note in notes if note)


def compare_row(
    component: str,
    used: str,
    fetch: Callable[[], str],
    note: str | Callable[[str, str], str] = "",
    **extra: Any,
) -> Row:
    """Row comparing `used` with the version `fetch` returns; failures become «nicht prüfbar».

    `note` is the Hinweis text, or a function of (newest, kind) that returns it.
    """
    try:
        newest = fetch()
        kind = classify(used, newest)
    except SourceError as err:
        return unchecked(component, used, err, **extra)
    text = note(newest, kind) if callable(note) else note
    return Row(component, used, newest, kind, note=text, **extra)


def with_dependabot(row: Row, updates: dict[str, int]) -> Row:
    """Points an update row at the open Dependabot PR that already covers it."""
    number = updates.get(row.name)
    if row.update and number is not None:
        row.pr = number
        row.note = join_notes(row.note, f"→ #{number}")
    return row


def table(rows: list[Row]) -> str:
    lines = [
        "| Komponente | verwendet | neuste | Art | Quelle/Link | Hinweis |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    for row in rows:
        kind = f"**{row.kind}**" if row.kind == MAJOR else row.kind
        component = f"**{md_text(row.component)}**" if row.pinned else md_text(row.component)
        cells = [component, md_code(row.used), md_code(row.newest), kind, row.source, row.note]
        lines.append("| " + " | ".join(c.replace("\n", " ") for c in cells) + " |")
    return "\n".join(lines)


def sort_rows(rows: list[Row]) -> list[Row]:
    return sorted(rows, key=lambda r: (not r.failed, KIND_ORDER[r.kind], r.component))


def section(title: str, rows: list[Row], intro: str = "", show_current: bool = False) -> str:
    """Table of rows with updates or failures; current rows follow as a folded list."""
    shown = rows if show_current else [r for r in rows if r.update or r.failed]
    current = [] if show_current else [r for r in rows if not (r.update or r.failed)]
    parts = [f"## {title}"]
    if intro:
        parts.append(intro)
    parts.append(table(sort_rows(shown)) if shown else "Alles aktuell.")
    if current:
        listed = ", ".join(f"{md_text(r.component)} {md_code(r.used)}" for r in current)
        parts.append(f"<details><summary>{len(current)} aktuell</summary>\n\n{listed}\n\n</details>")
    return "\n\n".join(parts)


# --- Sources --------------------------------------------------------------------------


class Sources:
    """All network and tool access of the report; tests replace it with a fake."""

    def __init__(self, repo: str, root: Path = ROOT, crates_delay: float = CRATES_IO_DELAY):
        self.repo = repo
        self.root = root
        self.crates_delay = crates_delay
        self._cache: dict[str, Any] = {}
        self._last_crates_request = 0.0

    def run(self, args: list[str], timeout: int = 60, stdin: str | None = None, with_stderr: bool = False) -> str:
        env = os.environ.copy()
        if args[0] != "gh":
            for name in GH_ONLY_ENV:
                env.pop(name, None)
        try:
            proc = subprocess.run(
                args,
                cwd=self.root,
                env=env,
                input=stdin,
                capture_output=True,
                text=True,
                errors="replace",
                timeout=timeout,
                check=False,
            )
        except FileNotFoundError as err:
            raise SourceError(f"{args[0]} fehlt") from err
        except subprocess.TimeoutExpired as err:
            raise SourceError(f"{args[0]}: Zeitüberschreitung") from err
        except OSError as err:
            raise SourceError(f"{args[0]}: {err.strerror or 'nicht ausführbar'}") from err
        if proc.returncode != 0:
            detail = (proc.stderr.strip().splitlines() or ["ohne Meldung"])[-1]
            raise SourceError(f"{' '.join(args[:2])}: {detail[:120]}")
        return proc.stdout + proc.stderr if with_stderr else proc.stdout

    def _http(self, url: str) -> bytes:
        request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
        try:
            with urllib.request.urlopen(request, timeout=HTTP_TIMEOUT) as response:
                return response.read(MAX_RESPONSE_BYTES)
        except urllib.error.HTTPError as err:
            raise SourceError(f"HTTP {err.code}") from err
        except (urllib.error.URLError, http.client.HTTPException, OSError) as err:
            raise SourceError("nicht erreichbar") from err

    def _json(self, url: str) -> Any:
        if url not in self._cache:
            try:
                self._cache[url] = json.loads(self._http(url))
            except ValueError as err:
                raise SourceError("ungültige Antwort") from err
        return self._cache[url]

    def gh_api(self, path: str) -> Any:
        if path not in self._cache:
            try:
                self._cache[path] = json.loads(self.run(["gh", "api", path]))
            except ValueError as err:
                raise SourceError("ungültige Antwort der GitHub-API") from err
        return self._cache[path]

    def crate_version(self, name: str) -> str:
        if not CRATE_RE.match(name):
            raise SourceError("ungültiger Crate-Name")
        url = f"https://crates.io/api/v1/crates/{name}"
        if url not in self._cache:
            time.sleep(max(0.0, self._last_crates_request + self.crates_delay - time.monotonic()))
            self._last_crates_request = time.monotonic()
        data = self._json(url)
        return checked_version(field_of(data, "crate", "max_stable_version"))

    def github_release(self, repo: str) -> str:
        path = f"repos/{checked_repo(repo)}/releases/latest"
        tag = checked_version(field_of(self.gh_api(path), "tag_name"))
        return tag.removeprefix("v")

    def github_newest_tag(self, repo: str) -> str:
        """Highest stable semver tag; for actions whose releases also carry other tags."""
        url = f"https://github.com/{checked_repo(repo)}.git"
        output = self.run(["git", "ls-remote", "--tags", "--refs", url])
        tags = [line.rpartition("refs/tags/")[2] for line in output.splitlines()]
        # Only dotted tags: date tags such as `20240101` would otherwise win.
        return newest_stable([tag for tag in tags if "." in tag])

    def npm_version(self, name: str) -> str:
        if not NPM_RE.match(name):
            raise SourceError("ungültiger Paketname")
        quoted = urllib.parse.quote(name, safe="@")
        return checked_version(field_of(self._json(f"https://registry.npmjs.org/{quoted}/latest"), "version"))

    def node_version(self) -> str:
        """Newest LTS release; odd-numbered majors never become LTS."""
        releases = self._json("https://nodejs.org/dist/index.json")
        if not isinstance(releases, list):
            raise SourceError("ungültige Antwort")
        return newest_stable([r.get("version") for r in releases if isinstance(r, dict) and r.get("lts")])

    def ubuntu_lts(self) -> str:
        text = self._http("https://changelogs.ubuntu.com/meta-release-lts").decode("utf-8", "replace")
        versions = re.findall(r"^Version:\s*(\d+\.\d+)", text, re.MULTILINE)
        if not versions:
            raise SourceError("keine LTS-Version gefunden")
        return max(versions, key=lambda v: require_version(v).key())

    def git_head(self, url: str, branch: str) -> str:
        output = self.run(["git", "ls-remote", url, f"refs/heads/{branch}"])
        return checked_sha(output.split()[0] if output.split() else "")

    def gitlab_commit_titles(self, project: str, base: str, head: str) -> list[str]:
        quoted = urllib.parse.quote(project, safe="")
        url = (
            f"https://invent.kde.org/api/v4/projects/{quoted}/repository/compare"
            f"?from={checked_sha(base)}&to={checked_sha(head)}"
        )
        commits = field_of(self._json(url), "commits")
        if not isinstance(commits, list):
            raise SourceError("ungültige Antwort")
        return [str(commit.get("title", "")) for commit in commits if isinstance(commit, dict)]

    def cxx_qt_fix_release(self, since: str) -> tuple[bool, str | None]:
        """(merged, first release newer than `since` that contains the fix PR)."""
        pr = self.gh_api(f"repos/{CXX_QT_REPO}/pulls/{CXX_QT_FIX_PR}")
        if not field_of(pr, "merged"):
            return False, None
        sha = checked_sha(field_of(pr, "merge_commit_sha"))
        releases = self.gh_api(f"repos/{CXX_QT_REPO}/releases?per_page=30")
        if not isinstance(releases, list):
            raise SourceError("ungültige Antwort der GitHub-API")
        tags = [
            checked_version(r.get("tag_name"))
            for r in releases
            if isinstance(r, dict) and not r.get("draft") and not r.get("prerelease")
        ]
        newer = [t for t in tags if classify(since, t) != NONE]
        for tag in sorted(newer, key=lambda t: require_version(t).key()):
            status = field_of(self.gh_api(f"repos/{CXX_QT_REPO}/compare/{tag}...{sha}"), "status")
            if status in ("behind", "identical"):
                return True, tag
        return True, None

    def cargo_update_dry_run(self) -> str:
        # cargo reports the planned updates on stderr.
        return self.run(["cargo", "update", "--dry-run"], timeout=600, with_stderr=True)

    def dependabot_prs(self) -> list[dict[str, Any]]:
        """Open Dependabot PRs; `statusCheckRollup` is None when the token cannot read checks."""
        base = ["gh", "pr", "list", "--repo", self.repo, "--author", "app/dependabot"]
        base += ["--state", "open", "--limit", "100", "--json"]
        try:
            output = self.run(base + ["number,title,body,statusCheckRollup"])
        except SourceError:
            output = self.run(base + ["number,title,body"])
        try:
            prs = json.loads(output)
        except ValueError as err:
            raise SourceError("ungültige Antwort") from err
        if not isinstance(prs, list):
            raise SourceError("ungültige Antwort")
        return [pr for pr in prs if isinstance(pr, dict) and isinstance(pr.get("number"), int)]


def field_of(data: Any, *keys: str) -> Any:
    for key in keys:
        if not isinstance(data, dict) or key not in data:
            raise SourceError(f"Feld «{key}» fehlt")
        data = data[key]
    return data


# --- Dependabot -----------------------------------------------------------------------


def dependabot_updates(prs: list[dict[str, Any]]) -> dict[str, int]:
    """Dependency name -> number of the open Dependabot PR that updates it."""
    updates: dict[str, int] = {}
    for pr in prs:
        names = [m[0] for m in DEPENDABOT_BODY_RE.findall(str(pr.get("body") or ""))]
        title = DEPENDABOT_TITLE_RE.match(str(pr.get("title") or ""))
        if title:
            names.append(title.group(1))
        for name in names:
            updates.setdefault(name, pr["number"])
    return updates


def ci_status(rollup: Any) -> str:
    if rollup is None:
        return "nicht prüfbar"
    if not rollup:
        return "keine Checks"
    states = set()
    for check in rollup:
        if isinstance(check, dict):
            states.add(str(check.get("conclusion") or check.get("state") or "").upper())
            states.add(str(check.get("status") or "").upper())
    if states & FAILED_CHECKS:
        return "❌ rot"
    if states & PENDING_CHECKS:
        return "⏳ läuft"
    return "✅ grün"


def dependabot_section(prs: list[dict[str, Any]] | SourceError) -> str:
    parts = ["## Offene Dependabot-PRs"]
    if isinstance(prs, SourceError):
        parts.append(f"nicht prüfbar: {md_text(prs)}")
    elif not prs:
        parts.append("Keine.")
    else:
        parts.append(
            "\n".join(
                f"- #{pr['number']} {md_text(pr.get('title', ''))} – CI: {ci_status(pr.get('statusCheckRollup'))}"
                for pr in sorted(prs, key=lambda pr: pr["number"])
            )
        )
    return "\n\n".join(parts)


# --- Repository files -----------------------------------------------------------------


@dataclass
class CrateDep:
    name: str
    requirements: set[str] = field(default_factory=set)
    git: str = ""
    rev: str = ""


def load_toml(path: Path) -> dict[str, Any]:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def dependency_tables(manifest: dict[str, Any]) -> list[dict[str, Any]]:
    kinds = ("dependencies", "dev-dependencies", "build-dependencies")
    scopes = [manifest, *manifest.get("target", {}).values()]
    return [scope.get(kind, {}) for scope in scopes for kind in kinds]


def add_dependency(deps: dict[str, CrateDep], key: str, spec: Any) -> None:
    if isinstance(spec, str):
        spec = {"version": spec}
    if "path" in spec:
        return
    name = spec.get("package", key)
    dep = deps.setdefault(name, CrateDep(name))
    if spec.get("version"):
        dep.requirements.add(spec["version"])
    if spec.get("git"):
        dep.git, dep.rev = spec["git"], spec.get("rev", "")


def cargo_dependencies(root: Path) -> dict[str, CrateDep]:
    """Direct registry and git dependencies of the workspace and its members."""
    workspace = load_toml(root / "Cargo.toml").get("workspace", {})
    shared = workspace.get("dependencies", {})
    deps: dict[str, CrateDep] = {}
    for key, spec in shared.items():
        add_dependency(deps, key, spec)
    for member in workspace.get("members", []):
        for deps_table in dependency_tables(load_toml(root / member / "Cargo.toml")):
            for key, spec in deps_table.items():
                if isinstance(spec, dict) and spec.get("workspace"):
                    spec = shared.get(key, {})
                add_dependency(deps, key, spec)
    return deps


def locked_versions(root: Path) -> dict[str, list[str]]:
    versions: dict[str, list[str]] = {}
    for package in load_toml(root / "Cargo.lock").get("package", []):
        if str(package.get("source", "")).startswith("registry+"):
            versions.setdefault(package["name"], []).append(package["version"])
    return versions


def parse_cargo_update(output: str) -> list[tuple[str, str, str]]:
    return [m.groups() for m in map(CARGO_UPDATE_RE.match, output.splitlines()) if m]


def bootstrap_versions(root: Path) -> dict[str, str]:
    text = (root / "scripts" / "bootstrap.sh").read_text(encoding="utf-8")
    return dict(m.groups() for m in map(BOOTSTRAP_VERSION_RE.match, text.splitlines()) if m)


def workflow_actions(root: Path) -> dict[str, set[str]]:
    """GitHub repository of each action used in a workflow -> refs it is pinned to."""
    actions: dict[str, set[str]] = {}
    for path in sorted((root / ".github" / "workflows").glob("*.y*ml")):
        for line in path.read_text(encoding="utf-8").splitlines():
            match = USES_RE.match(line)
            if match:
                actions.setdefault(match.group(1), set()).add(match.group(2))
    return actions


def base_images(root: Path) -> list[str]:
    text = (root / ".github" / "ci-image" / "Dockerfile").read_text(encoding="utf-8")
    return re.findall(r"^FROM\s+(\S+)", text, re.MULTILINE)


# --- Report sections ------------------------------------------------------------------


def crates_io(name: str) -> str:
    return link("crates.io", f"https://crates.io/crates/{name}")


def npm_link(name: str) -> str:
    return link("npm", f"https://www.npmjs.com/package/{name}") if NPM_RE.match(name) else ""


def releases_link(repo: str) -> str:
    return link("Releases", f"https://github.com/{repo}/releases")


def crate_row(dep: CrateDep, locked: dict[str, list[str]], sources: Sources) -> Row:
    requirement = lowest(dep.requirements)
    used = locked_for(requirement, locked.get(dep.name, [])) or requirement.lstrip("=^")

    def note(newest: str, kind: str) -> str:
        if kind == NONE:
            return ""
        allowed = not requirement.startswith("=") and (
            require_version(newest).compat_key() == require_version(used).compat_key()
        )
        return "per `cargo update`" if allowed else f"Anforderung {md_code(requirement)} anheben"

    return compare_row(
        dep.name,
        used,
        lambda: sources.crate_version(dep.name),
        note,
        name=dep.name,
        source=crates_io(dep.name) if CRATE_RE.match(dep.name) else "",
    )


def direct_crate_rows(root: Path, sources: Sources, updates: dict[str, int]) -> list[Row]:
    locked = locked_versions(root)
    deps = cargo_dependencies(root)
    return [
        with_dependabot(crate_row(dep, locked, sources), updates)
        for name, dep in sorted(deps.items())
        if name not in PINNED_CRATES and not dep.git
    ]


def lockfile_rows(root: Path, sources: Sources, updates: dict[str, int]) -> list[Row]:
    """Transitive lockfile updates; direct crates already have their own rows."""
    output = sources.cargo_update_dry_run()
    direct = set(cargo_dependencies(root))
    rows = []
    for name, old, new in parse_cargo_update(output):
        if name in direct:
            continue
        try:
            row = Row(name, old, new, classify(old, new), crates_io(name), name=name)
        except SourceError as err:
            row = unchecked(name, old, err, name=name)
        rows.append(with_dependabot(row, updates))
    return rows


def cxx_qt_rows(root: Path, sources: Sources) -> list[Row]:
    locked = locked_versions(root)
    source = link("Changelog", CXX_QT_CHANGELOG)
    rows = []
    for name in PINNED_CRATES:
        if name not in locked:
            rows.append(unchecked(name, NONE, "nicht in Cargo.lock", source=source, pinned=True, name=name))
            continue
        fetch = functools.partial(sources.crate_version, name)
        rows.append(
            compare_row(name, locked[name][0], fetch, "gepinnt, nur Stop & Ask", source=source, pinned=True, name=name)
        )
    rows.append(cxx_qt_fix_row(locked.get(PINNED_CRATES[0], ["0.0.0"])[0], sources))
    return rows


def cxx_qt_fix_row(pinned: str, sources: Sources) -> Row:
    component = f"Fix für Issue 1508 / PR {CXX_QT_FIX_PR} (veraltete plugin.qmltypes)"
    source = link("Changelog", CXX_QT_CHANGELOG)
    try:
        merged, release = sources.cxx_qt_fix_release(pinned)
    except SourceError as err:
        return unchecked(component, NONE, err, source=source, pinned=True)
    if release:
        note = f"in {md_code(release)} enthalten"
    elif merged:
        note = "gemerged, aber noch in keinem Release"
    else:
        note = "noch nicht gemerged"
    return Row(component, NONE, release or NONE, source=source, note=note, pinned=True, update=bool(release))


def kde_frameworks_row(root: Path, sources: Sources) -> tuple[Row, list[str]]:
    dep = cargo_dependencies(root).get("cxx-kde-frameworks", CrateDep("cxx-kde-frameworks"))
    component, used = "cxx-kde-frameworks", dep.rev[:8] or NONE
    try:
        rev = checked_sha(dep.rev)
        head = sources.git_head(KDE_FRAMEWORKS_URL, KDE_FRAMEWORKS_BRANCH)
    except SourceError as err:
        return unchecked(component, used, err, pinned=True), []
    source = link("Vergleich", f"https://invent.kde.org/{KDE_FRAMEWORKS_PROJECT}/-/compare/{rev}...{head}")
    if head == rev:
        return Row(component, used, head[:8], source=source, note="gepinnt, aktuell", pinned=True), []
    try:
        titles = sources.gitlab_commit_titles(KDE_FRAMEWORKS_PROJECT, rev, head)
        count = f"{len(titles)} Commit(s) neuer auf `{KDE_FRAMEWORKS_BRANCH}`"
    except SourceError as err:
        titles, count = [], f"Anzahl Commits nicht prüfbar: {md_text(err)}"
    note = f"{count}; gepinnt, nur Stop & Ask"
    return Row(component, used, head[:8], source=source, note=note, pinned=True, update=True), titles


def commit_list(titles: list[str]) -> str:
    if not titles:
        return ""
    shown = titles[:MAX_COMMIT_TITLES]
    lines = "\n".join(f"- {md_text(title, 120)}" for title in shown)
    more = f"\n- … und {len(titles) - len(shown)} weitere" if len(titles) > len(shown) else ""
    summary = f"Commits in cxx-kde-frameworks seit dem gepinnten Stand ({len(titles)})"
    return f"<details><summary>{summary}</summary>\n\n{lines}{more}\n\n</details>"


def toolchain_rows(root: Path, sources: Sources) -> list[Row]:
    channel = load_toml(root / "rust-toolchain.toml")["toolchain"]["channel"]
    msrv = load_toml(root / "Cargo.toml")["workspace"]["package"]["rust-version"]
    source = releases_link("rust-lang/rust")
    fetch = functools.partial(sources.github_release, "rust-lang/rust")
    return [
        compare_row(
            "Rust-Toolchain (rust-toolchain.toml)",
            channel,
            fetch,
            "gepinnt, nur Stop & Ask; `rust-version` mitziehen",
            source=source,
            pinned=True,
        ),
        compare_row(
            "rust-version (Cargo.toml)",
            msrv,
            fetch,
            "gepinnt, folgt der Toolchain",
            source=source,
            pinned=True,
        ),
    ]


def bootstrap_rows(root: Path, sources: Sources) -> list[Row]:
    rows = []
    for variable, used in sorted(bootstrap_versions(root).items()):
        if variable not in BOOTSTRAP_TOOLS:
            rows.append(unchecked(variable, used, "keine Quelle im Report hinterlegt"))
            continue
        component, kind, name = BOOTSTRAP_TOOLS[variable]
        if kind == "github":
            fetch, source = (lambda name=name: sources.github_release(name)), releases_link(name)
        else:
            fetch, source = (lambda name=name: sources.crate_version(name)), crates_io(name)
        row = compare_row(component, used, fetch, source=source, name=name)
        if variable == "RESTIC_MIN_VERSION" and not row.failed:
            row.kind, row.update = NONE, False
        row.note = join_notes(row.note, BOOTSTRAP_NOTES.get(variable, ""))
        rows.append(row)
    return rows


def ci_image_rows(root: Path, sources: Sources) -> list[Row]:
    rows = []
    for image in base_images(root):
        name, _, tag = image.partition(":")
        if name != "ubuntu":
            rows.append(unchecked(f"Basis-Image {image}", tag or NONE, "nur ubuntu wird geprüft"))
            continue
        rows.append(
            compare_row(
                "Basis-Image ubuntu (CI-Image)",
                tag,
                sources.ubuntu_lts,
                "neuste LTS-Version; ändert den Tag des CI-Images",
                source=link("Ubuntu-Releases", "https://changelogs.ubuntu.com/meta-release-lts"),
            )
        )
    return rows


def action_row(repo: str, refs: set[str], sources: Sources, updates: dict[str, int]) -> Row:
    ref = lowest(refs)
    fetch = functools.partial(sources.github_newest_tag, repo)
    row = compare_row(repo, ref, fetch, name=repo, source=releases_link(repo))
    # A major tag such as `@v7` follows every minor and patch release by itself.
    if len(refs) == 1 and re.fullmatch(r"v?\d+", ref) and row.kind in (MINOR, PATCH):
        row.kind, row.update = NONE, False
    row.used = ", ".join(sorted(refs))
    return with_dependabot(row, updates)


def action_rows(root: Path, sources: Sources, updates: dict[str, int]) -> list[Row]:
    return [
        action_row(repo, refs, sources, updates)
        for repo, refs in sorted(workflow_actions(root).items())
        if GH_REPO_RE.match(repo)
    ]


def site_rows(root: Path, sources: Sources, updates: dict[str, int]) -> list[Row]:
    package = json.loads((root / "site" / "package.json").read_text(encoding="utf-8"))
    rows = []
    packages = {**package.get("dependencies", {}), **package.get("devDependencies", {})}
    for name, used in sorted(packages.items()):
        row = compare_row(
            name,
            used.lstrip("^~="),
            lambda name=name: sources.npm_version(name),
            name=name,
            source=npm_link(name),
        )
        rows.append(with_dependabot(row, updates))
    manager, _, manager_version = str(package.get("packageManager", "")).partition("@")
    if manager:
        rows.append(
            compare_row(
                f"{manager} (packageManager)",
                manager_version,
                lambda: sources.npm_version(manager),
                source=npm_link(manager),
            )
        )
    node = package.get("devEngines", {}).get("runtime", {})
    if node.get("name") == "node":
        rows.append(
            compare_row(
                "Node (devEngines.runtime)",
                node.get("version", ""),
                sources.node_version,
                "neuste LTS-Version; nicht von Dependabot abgedeckt",
                source=link("Node-Releases", "https://nodejs.org/en/about/previous-releases"),
            )
        )
    return rows


DEB_TITLE = "Mindestversionen im `.deb`"


def deb_section(root: Path) -> str:
    deb = load_toml(root / "gui" / "Cargo.toml")["package"]["metadata"]["deb"]
    items = []
    for kind in ("depends", "recommends"):
        for entry in str(deb.get(kind, "")).split(","):
            entry = entry.strip()
            if entry == "$auto":
                items.append("- `$auto`: Qt- und KF6-Bibliotheken berechnet `dpkg-shlibdeps` beim Bau")
            elif entry:
                items.append(f"- {kind.capitalize()}: {md_code(entry)}")
    intro = "Nur Information, kein Vergleich: Mindestversionen in `gui/Cargo.toml` (`[package.metadata.deb]`)."
    return "\n\n".join([f"## {DEB_TITLE}", intro, "\n".join(items)])


# --- Report ---------------------------------------------------------------------------


@dataclass
class Report:
    title: str
    body: str


def summary(rows: list[Row], prs: list[dict[str, Any]] | SourceError) -> str:
    updates = [r for r in rows if r.update and not r.failed]
    majors = [r for r in updates if r.kind == MAJOR and not r.pinned]
    pinned = [r for r in updates if r.pinned]
    lines = []
    if majors:
        names = ", ".join(f"**{md_text(r.component)}** {r.used} → {r.newest}" for r in majors)
        lines.append(f"- **Handlung:** {len(majors)} Major-Update(s): {names}")
    else:
        lines.append("- Keine Major-Updates ausserhalb der gepinnten Komponenten.")
    if pinned:
        names = ", ".join(f"**{md_text(r.component)}** → {r.newest}" for r in pinned)
        lines.append(f"- **Gepinnt, nur Stop & Ask:** {names}")
    else:
        lines.append("- Gepinnte Komponenten: nichts Neues.")
    minor = sum(r.kind == MINOR for r in updates)
    patch = sum(r.kind == PATCH for r in updates)
    covered = sum(r.pr is not None for r in updates)
    lines.append(f"- {minor} Minor, {patch} Patch; {covered} Update(s) mit offenem Dependabot-PR.")
    if isinstance(prs, SourceError):
        lines.append("- Offene Dependabot-PRs: nicht prüfbar.")
    else:
        red = sum(ci_status(pr.get("statusCheckRollup")) == "❌ rot" for pr in prs)
        lines.append(f"- Offene Dependabot-PRs: {len(prs)}" + (f", davon {red} mit rotem CI." if red else "."))
    failed = sum(r.failed for r in rows)
    if failed:
        lines.append(f"- Nicht prüfbar: {failed} Zeile(n), Grund in der Spalte «Hinweis».")
    return "\n".join(lines)


def issue_title(day: dt.date) -> str:
    year, week, _ = day.isocalendar()
    return f"{TITLE_PREFIX} {week} {year}"


@dataclass
class Part:
    markdown: str
    rows: list[Row] = field(default_factory=list)


def table_part(
    title: str, rows: Callable[[], list[Row]], intro: str = "", show_current: bool = False
) -> tuple[str, Callable[[], Part]]:
    def build() -> Part:
        built = rows()
        return Part(section(title, built, intro, show_current), built)

    return title, build


def guarded(title: str, build: Callable[[], Part]) -> Part:
    """Builds one section; a failure there leaves the other sections intact."""
    try:
        return build()
    except Exception as err:  # also a malformed repository file must not stop the report
        reason = err if isinstance(err, SourceError) else f"{type(err).__name__}: {err}"
        return Part(f"## {title}\n\nnicht prüfbar: {md_text(reason)}", [unchecked(title, NONE, reason)])


def pinned_part(root: Path, sources: Sources) -> Part:
    kde_row, commits = kde_frameworks_row(root, sources)
    rows = [*cxx_qt_rows(root, sources), kde_row]
    table_md = section(PINNED_TITLE, rows, show_current=True)
    return Part("\n\n".join(part for part in (table_md, commit_list(commits)) if part), rows)


def fit_body(parts: list[str]) -> str:
    """Joins the parts; whole sections that no longer fit GitHub's limit are left out."""
    body = ""
    for index, part in enumerate(parts):
        candidate = f"{body}\n\n{part}" if body else part
        if len(candidate) > MAX_BODY_CHARS:
            return f"{body}\n\n… gekürzt: {len(parts) - index} Abschnitt(e) fehlen (Grenze von GitHub)."
        body = candidate
    return body


PINNED_TITLE = "Gepinnt: CXX-Qt und cxx-kde-frameworks"
FOOTER = (
    "---\nErzeugt von `scripts/deps_report.py` (Workflow `dependency-report.yml`). "
    "Der Report ändert keine Datei und öffnet keinen PR."
)


def build_report(root: Path, sources: Sources, day: dt.date) -> Report:
    try:
        prs: list[dict[str, Any]] | SourceError = sources.dependabot_prs()
    except SourceError as err:
        prs = err
    updates = dependabot_updates(prs) if isinstance(prs, list) else {}
    specs = [
        table_part(
            "Rust-Crates (direkt)",
            lambda: direct_crate_rows(root, sources, updates),
            "Vergleich mit der neusten stabilen Version auf crates.io.",
        ),
        table_part(
            "Lockfile (`cargo update --dry-run`)",
            lambda: lockfile_rows(root, sources, updates),
            "Transitive Crates mit neuerer kompatibler Version; die direkten stehen oben.",
        ),
        (PINNED_TITLE, lambda: pinned_part(root, sources)),
        table_part("Rust-Toolchain", lambda: toolchain_rows(root, sources), show_current=True),
        table_part(
            "restic und Testwerkzeuge (`scripts/bootstrap.sh`)",
            lambda: bootstrap_rows(root, sources),
            "Eine Änderung an `scripts/bootstrap.sh` baut das CI-Image neu.",
            show_current=True,
        ),
        table_part(
            "CI-Image und GitHub Actions",
            lambda: ci_image_rows(root, sources) + action_rows(root, sources, updates),
        ),
        table_part("Showcase-Seite (`site/`)", lambda: site_rows(root, sources, updates)),
        (DEB_TITLE, lambda: Part(deb_section(root))),
    ]
    parts = [guarded(title, build) for title, build in specs]
    rows = [row for part in parts for row in part.rows]
    head = ["## Zusammenfassung", summary(rows, prs), dependabot_section(prs)]
    return Report(issue_title(day), fit_body([*head, *(p.markdown for p in parts), FOOTER]))


# --- Issue ----------------------------------------------------------------------------


def superseded_issues(issues: list[dict[str, Any]], new_number: int) -> list[int]:
    """Older open reports that the new issue replaces."""
    return sorted(
        issue["number"]
        for issue in issues
        if isinstance(issue.get("number"), int)
        and issue["number"] < new_number
        and str(issue.get("title", "")).startswith(TITLE_PREFIX + " ")
    )


def publish(report: Report, sources: Sources) -> int:
    """Opens the report as an issue and closes the previous ones; returns the exit code.

    Only a failed issue creation fails the run; leftover old reports become a warning.
    """
    try:
        number = create_issue(report, sources)
    except (SourceError, ValueError) as err:
        print(f"Issue nicht angelegt: {err}", file=sys.stderr)
        return 1
    try:
        listing = json.loads(sources.run(
            ["gh", "issue", "list", "--repo", sources.repo, "--state", "open", "--limit", "200",
             "--json", "number,title"]
        ))
        old_reports = superseded_issues(listing if isinstance(listing, list) else [], number)
    except (SourceError, ValueError) as err:
        print(f"::warning::Ältere Reports nicht geprüft: {err}")
        return 0
    for old in old_reports:
        try:
            sources.run(
                ["gh", "issue", "close", str(old), "--repo", sources.repo, "--reason", "not planned",
                 "--comment", f"Abgelöst durch #{number}"]
            )
            print(f"Issue #{old} geschlossen")
        except SourceError as err:
            print(f"::warning::Issue #{old} nicht geschlossen: {err}")
    return 0


def create_issue(report: Report, sources: Sources) -> int:
    labels = [LABEL] if label_exists(sources, LABEL) else []
    payload = json.dumps({"title": report.title, "body": report.body, "labels": labels})
    output = sources.run(["gh", "api", "-X", "POST", f"repos/{sources.repo}/issues", "--input", "-"], stdin=payload)
    created = json.loads(output)
    number = field_of(created, "number")
    if not isinstance(number, int):
        raise SourceError("ungültige Issue-Nummer")
    print(f"Issue #{number}: {field_of(created, 'html_url')}")
    return number


def label_exists(sources: Sources, name: str) -> bool:
    try:
        sources.gh_api(f"repos/{sources.repo}/labels/{name}")
    except SourceError:
        return False
    return True


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--create-issue", action="store_true", help="open the report as a GitHub issue")
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY") or DEFAULT_REPO)
    args = parser.parse_args(argv)
    if not GH_REPO_RE.match(args.repo):
        parser.error("--repo must be <owner>/<name>")
    sources = Sources(args.repo)
    report = build_report(ROOT, sources, dt.datetime.now(dt.timezone.utc).date())
    if not args.create_issue:
        print(f"# {report.title}\n\n{report.body}")
        return 0
    return publish(report, sources)


if __name__ == "__main__":
    sys.exit(main())
