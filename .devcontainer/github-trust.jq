# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Whose GitHub text counts, by numeric user ID. One file for the agent
# container (root-owned copy in /usr/local/lib/kestic-agent/), the host watcher
# and the Leitstand. Prepend it to a jq filter:
#
#   jq "$(< github-trust.jq) .[] | .user | trust_class"
#   gh api … --jq "$(< github-trust.jq) …"
#
# The IDs are public (gh api users/<login> --jq .id).

def trust_ids: {maintainer: 205621, agent: 359840804};

# "maintainer", "agent" or "withheld" for a GitHub user object: a REST user
# ({id, type}) or a GraphQL actor ({__typename, databaseId}). Only the numeric
# ID and the account type decide; the login, `author_association`, a missing
# user (null, ghost), Mannequins and every other bot or app are "withheld".
def trust_class:
  if type != "object" then "withheld"
  else
    (if (.id | type) == "number" then {id: .id, kind: .type}
     else {id: .databaseId, kind: .__typename} end) as $u
    | if ($u.id | type) != "number" then "withheld"
      elif $u.id == trust_ids.maintainer and $u.kind == "User" then "maintainer"
      elif $u.id == trust_ids.agent and $u.kind == "Bot" then "agent"
      else "withheld" end
  end;

# A login safe to print, or "<invalid login>".
def safe_login:
  if type == "string" and test("^[A-Za-z0-9-]{1,39}(\\[bot\\])?$") then . else "<invalid login>" end;

# An author_association safe to print, or "UNKNOWN".
def safe_association:
  if type == "string" and IN("OWNER", "MEMBER", "COLLABORATOR", "CONTRIBUTOR",
      "FIRST_TIME_CONTRIBUTOR", "FIRST_TIMER", "MANNEQUIN", "NONE") then . else "UNKNOWN" end;

# An ISO 8601 UTC time safe to print, or "<invalid time>".
def safe_time:
  if type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$") then . else "<invalid time>" end;

# The pull request as the agent's own: in this repository (not a fork) and
# opened by the App. Takes a REST pull request object.
def agent_pull:
  (.head.repo.full_name // "") == (.base.repo.full_name // "-")
  and (.user | trust_class) == "agent";
