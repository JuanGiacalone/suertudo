#!/usr/bin/env bash
# Fails when there is a focused test in the given directory.
#
# Dart has no `forbidOnly`: a leftover `test.only` makes the suite run ONE
# test and exit 0. The pipeline goes green while the rest of the suite
# never ran, and nobody finds out until the bug that the skipped test
# would have caught shows up in production.
#
# The logic lives here, and not in the workflow, for one reason: a check
# whose branches are never executed is a check that only works on the
# day it was written. `scripts/check-focused-tests_test.sh` pins every
# branch of this script.
#
# Usage:  scripts/check-focused-tests.sh [directory]
#
# The directory defaults to the ROOT of the repository, and that is on
# purpose: a focused test in a Dart package that is not the mobile app is
# still a focused test. The scan skips the directories of the
# dependencies and of the builds, because they are not ours and they are
# enormous.
#
# Exit codes:
#   0  there is no focused test
#   1  there is a focused test
#   2  the scan itself failed
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="${1:-$REPO_ROOT}"

# The scan and the verdict are two separate things, and the exit status
# of the scan is checked on its own. grep exits 1 when it finds nothing
# and something above 1 when the scan itself broke, and a scan that
# never ran looks EXACTLY like a clean directory. AGENTS.md §9 says that
# a check which only looks for the absence of an error reports PASS when
# nothing ran.
scan="$(grep -rn --include='*_test.dart' \
  --exclude-dir=node_modules --exclude-dir=target --exclude-dir=.dart_tool \
  --exclude-dir=build --exclude-dir=.git --exclude-dir=.next \
  -E '\b(test|testWidgets|group)\.only\b' "$DIR" 2>&1)" && status=0 || status=$?
if [ "$status" -gt 1 ]; then
  # The annotation goes to the STDOUT, where GitHub Actions reads the
  # workflow commands, and it is a STATIC sentence. The content of the
  # repository goes to the STDERR, where it cannot inject a command.
  if [ -n "${GITHUB_ACTIONS:-}" ]; then
    echo "::error::the scan of the focused tests failed"
  fi
  echo "the scan of the focused tests failed" >&2
  echo "$scan" >&2
  exit 2
fi

# The lines that START with `//` are exempt: a test file that documents
# the rule would otherwise turn the pipeline red, and that is the kind of
# check people learn to work around. A mention inside a block comment, or
# at the end of a line of code, still fails the check, and that is
# deliberate: a false positive costs a red pipeline for a minute, and a
# false negative is a suite that silently does not run.
#
# `|| true` is there on purpose: with only exempt comments, the filter
# selects nothing and grep exits 1, and `bash -e` would kill the caller
# BEFORE the verdict below is evaluated. The guard broke exactly here
# once, because the filter was in a plain assignment.
#
# The verdict is NOT the exit status of the filter. An empty scan still
# produces a blank line, and the filter keeps it, so grep exits 0 and the
# status would say "there is a focused test" when there is none. The
# command substitution strips the trailing newline, so what is left is the
# empty string, and THAT is the verdict.
focused="$(printf '%s\n' "$scan" | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' || true)"
if [ -n "$focused" ]; then
  # The annotation is a STATIC sentence on the STDOUT, where GitHub
  # Actions reads the workflow commands. The content of the repository
  # goes to the STDERR: GitHub Actions does not read commands from there,
  # so a file of the repository cannot inject one into the log.
  if [ -n "${GITHUB_ACTIONS:-}" ]; then
    echo "::error::a focused test would make the pipeline run one test and pass"
  fi
  echo "a focused test would make the pipeline run one test and pass" >&2
  echo "$focused" >&2
  exit 1
fi

exit 0
