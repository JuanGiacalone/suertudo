#!/usr/bin/env bash
# The test of the check of the focused tests.
#
# The guard of the focused tests broke in exactly the branch it was
# written to exempt, and the only thing that noticed was a review. A
# check whose branches are never executed is a check that only works on
# the day it was written, so every branch of
# `scripts/check-focused-tests.sh` is pinned here.
#
# Usage:  scripts/check-focused-tests_test.sh
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECK="$REPO_ROOT/scripts/check-focused-tests.sh"

failures=0

# Runs the check and compares its exit code with the expected one.
expect() {
  local name="$1" expected="$2" dir="$3"
  "$CHECK" "$dir" >/dev/null 2>&1
  local got=$?
  if [ "$got" = "$expected" ]; then
    echo "    OK: $name (codigo $got)"
  else
    echo "    FALLO: $name (se esperaba $expected, se obtuvo $got)"
    failures=$((failures + 1))
  fi
}

work="$(mktemp -d)"
trap 'chmod -R u+rwX "$work" 2>/dev/null; rm -rf "$work"' EXIT

# 1. A clean directory: no focused test.
mkdir -p "$work/clean"
cat > "$work/clean/a_test.dart" << 'DART'
void main() {
  test('algo', () {});
}
DART
expect "un directorio limpio pasa" 0 "$work/clean"

# 2. A focused test: it has to be caught.
mkdir -p "$work/focused"
cat > "$work/focused/a_test.dart" << 'DART'
void main() {
  test.only('algo', () {});
}
DART
expect "un test enfocado se detecta" 1 "$work/focused"

# 3. A comment that mentions the rule: it is exempt, and the check does
#    not die on it. The exemption is the branch that broke in the first
#    place: the filter was in a plain assignment and `bash -e` killed the
#    step before the verdict.
mkdir -p "$work/commented"
cat > "$work/commented/a_test.dart" << 'DART'
// este comentario menciona test.only y no puede romper el pipeline
void main() {
  test('algo', () {});
}
DART
expect "un comentario con la regla se ignora" 0 "$work/commented"

# 3b. The same, but invoked with `bash -e`. The filter is in the
#     condition of an `if`, where errexit does not apply, so the check
#     survives a caller that turns errexit on. The historical regression
#     was the plain assignment, which `bash -e` killed before the
#     verdict.
if bash -e "$CHECK" "$work/commented" >/dev/null 2>&1; then
  echo "    OK: el guard sobrevive a un llamador con bash -e"
else
  echo "    FALLO: el guard muere con bash -e, que es como se rompió una vez"
  failures=$((failures + 1))
fi

# 3c. A nested directory: the scan recurses, because a focused test in a
#     package that is not the mobile app is still a focused test.
mkdir -p "$work/nested/deep/deeper"
cat > "$work/nested/deep/deeper/a_test.dart" << 'DART'
void main() {
  group.only('un grupo', () {});
}
DART
expect "un test enfocado en un subdirectorio se detecta" 1 "$work/nested"

# 3d. The error annotation. GitHub Actions reads the workflow commands of
#     the STDOUT, so the annotation has to be there, and it has to be a
#     STATIC sentence: the content of the repository goes to the STDERR,
#     where it cannot inject a command.
annotation="$(GITHUB_ACTIONS=true "$CHECK" "$work/focused" 2>/dev/null)"
if printf '%s' "$annotation" | grep -q '^::error::'; then
  echo "    OK: el fallo anota el error para GitHub Actions"
else
  echo "    FALLO: el fallo no anota el error, y el pipeline no lo muestra"
  failures=$((failures + 1))
fi

# 3e. And the content of the repository does NOT reach the STDOUT, where
#     a workflow command could be injected.
if printf '%s' "$annotation" | grep -q 'test.only'; then
  echo "    FALLO: el contenido del repositorio llego al STDOUT"
  failures=$((failures + 1))
else
  echo "    OK: el contenido del repositorio no llega al STDOUT"
fi

# 3f. The variants of the focused test: `testWidgets.only` and
#     `group.only` are focused tests too.
mkdir -p "$work/widgets"
cat > "$work/widgets/a_test.dart" << 'DART'
void main() {
  testWidgets.only('un widget', (tester) {});
}
DART
expect "un testWidgets.only se detecta" 1 "$work/widgets"

mkdir -p "$work/groups"
cat > "$work/groups/a_test.dart" << 'DART'
void main() {
  group.only('un grupo', () {});
}
DART
expect "un group.only se detecta" 1 "$work/groups"

# 3g. A file that is NOT a Dart test file is not scanned, even if it
#     mentions a focused test: the guard is about the suite.
mkdir -p "$work/notatest"
cat > "$work/notatest/helper.dart" << 'DART'
// mentions test.only but is not a test file
void main() {}
DART
expect "un archivo que no es de test se ignora" 0 "$work/notatest"

# 4. A broken scan: it is NOT the same as a clean directory.
#
#    The premise is that the runner cannot read the directory, and root
#    ignores the permission bits of a directory. The case is therefore
#    omitted, and said so, when the runner is root: a case that fails for
#    the wrong reason teaches the reader that the branch is broken.
if [ "$(id -u)" -eq 0 ]; then
  echo "    OMITIDO: el directorio ilegible, porque root lo puede leer igual"
else
  mkdir -p "$work/unreadable"
  cat > "$work/unreadable/a_test.dart" << 'DART'
void main() {}
DART
  chmod 000 "$work/unreadable"
  expect "un escaneo roto se distingue del limpio" 2 "$work/unreadable"
  chmod 755 "$work/unreadable"
fi

# 5. A directory that does not exist: the scan failed.
expect "un directorio inexistente falla el escaneo" 2 "$work/nope"

if [ "$failures" -gt 0 ]; then
  echo "    FALLO: $failures comprobaciones fallaron"
  exit 1
fi

echo "    OK: todas las ramas del guard se comportan como deben"
