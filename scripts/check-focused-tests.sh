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
# Usage:  scripts/check-focused-tests.sh <directory>
#
# Exit codes:
#   0  there is no focused test
#   1  there is a focused test
#   2  the scan itself failed
set -uo pipefail

DIR="${1:-apps/mobile}"

# The scan and the verdict are two separate things, and the exit status
# of the scan is checked on its own. grep exits 1 when it finds nothing
# and something above 1 when the scan itself broke, and a scan that
# never ran looks EXACTLY like a clean directory. AGENTS.md §9 says that
# a check which only looks for the absence of an error reports PASS when
# nothing ran.
scan="$(grep -rn --include='*_test.dart' -E '\b(test|testWidgets|group)\.only\b' "$DIR" 2>&1)" && status=0 || status=$?
if [ "$status" -gt 1 ]; then
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
  echo "a focused test would make the pipeline run one test and pass" >&2
  # To stderr, on purpose: GitHub Actions parses the workflow commands of
  # the STDOUT, and the content of a file of the repository must not be
  # able to inject one into the log of the pipeline.
  echo "$focused" >&2
  exit 1
fi

exit 0
