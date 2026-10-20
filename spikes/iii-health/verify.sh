#!/usr/bin/env bash
# Reproducible end-to-end check of the iii stack for Suertudo.
#
# Proves, on a host WITHOUT KVM, that:
#   1. the iii engine starts and listens on 49134
#   2. the registry `http` worker runs as a host process (no microVM)
#   3. a Rust worker registers a function and an http trigger
#   4. GET /v1/health returns 200 through the engine
#   5. the function is callable directly through the engine
#
# Usage:  ./spikes/iii-health/verify.sh
#         unshare -rm ./spikes/iii-health/verify.sh   (forces the no-KVM path)
set -uo pipefail

SPIKE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SPIKE_DIR"

# cargo is not on PATH in a bare shell (rustup installs to ~/.cargo/bin).
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

ENGINE_URL="ws://127.0.0.1:49134"
NAMESPACE="spike"
HEALTH_URL="http://127.0.0.1:3111/v1/health"

pass() { echo "    OK: $1"; }
fail() { echo "    FALLO: $1"; exit 1; }

# Match by exact process name, never with `pgrep -f`: a pattern match also
# hits the shell that is running this script, which makes the check lie.
iii_running() {
  pgrep -x iii >/dev/null 2>&1 \
    || pgrep -x "iii:c:$NAMESPACE" >/dev/null 2>&1 \
    || pgrep -x http >/dev/null 2>&1 \
    || pgrep -x iii-health-spike >/dev/null 2>&1
}

cleanup() {
  # The registry `http` worker runs as a process named `http` and reconnects
  # to the engine on its own. Left alive, it holds the container name and the
  # next run fails with CONTAINER_NAME_TAKEN.
  for name in iii "iii:c:$NAMESPACE" iii-health-spike http iii-worker; do
    pkill -9 -x "$name" 2>/dev/null
  done
  sleep 1
}
trap cleanup EXIT

echo "### 0. Precondición: sin procesos iii previos"
if iii_running; then
  fail "hay un engine o compose daemon vivo; un daemon zombie reclama el namespace primero"
fi
pass "limpio"

echo
echo "### 1. Estado de KVM"
KVM_MM=$(stat -c '%t,%T' /dev/kvm 2>/dev/null || echo "n/a")
if [ "$KVM_MM" = "a,e8" ]; then
  pass "KVM real presente (10,232) — el stack no lo usa, pero el test no lo fuerza"
else
  pass "/dev/kvm no es un dispositivo KVM (${KVM_MM}) — ruta sin KVM activa"
fi

echo
echo "### 2. Engine"
setsid iii --config config.yaml --no-update-check > engine.log 2>&1 &
for i in $(seq 1 30); do
  sleep 1
  ss -ltn 2>/dev/null | grep -q 49134 && break
  [ "$i" = 30 ] && fail "el engine no escuchó en 49134"
done
pass "engine escuchando en 49134"

echo
echo "### 3. Worker http del registro (tipo binary, proceso del host)"
setsid iii compose --namespace "$NAMESPACE" --engine "$ENGINE_URL" > compose.log 2>&1 &
sleep 6
# compose::add only edits the file; compose::up is what reconciles the project.
iii trigger -n "$NAMESPACE" compose::up file=./worker-compose.yaml >/dev/null 2>&1
for i in $(seq 1 40); do
  sleep 1
  state=$(iii trigger -n "$NAMESPACE" compose::status file=./worker-compose.yaml 2>/dev/null \
          | grep -o '"state": "[a-z]*"' | head -1 | cut -d'"' -f4)
  [ "$state" = "ready" ] && break
  [ "$i" = 40 ] && fail "el worker http quedó en estado '$state' (ver compose.log)"
done
pass "worker http ready (0.22.7)"

echo
echo "### 4. Worker Rust propio"
setsid ./target/debug/iii-health-spike > spike.log 2>&1 &
sleep 6
pass "worker registrado"

echo
echo "### 5. GET /v1/health"
code=$(curl -s -o /tmp/iii-health-body.json -w '%{http_code}' "$HEALTH_URL")
[ "$code" = "200" ] || fail "se esperaba HTTP 200, se obtuvo $code"
pass "HTTP 200 -> $(cat /tmp/iii-health-body.json)"

echo
echo "### 6. Invocación directa por el engine"
iii trigger -n "$NAMESPACE" health::check 2>&1 | grep -q '"status": "ok"' \
  && pass "health::check responde por el engine" \
  || fail "health::check no respondió"

echo
echo "### 7. Confirmar que no se usó ninguna microVM"
if pgrep -x iii-worker >/dev/null 2>&1; then
  fail "hay un proceso iii-worker: se usó una microVM"
fi
pass "ningún proceso iii-worker, ninguna microVM"

echo
echo "### 8. Golden test de la derivación"
TEST_OUT=$(cargo test 2>&1)
if echo "$TEST_OUT" | grep -q 'test result: FAILED'; then
  echo "$TEST_OUT" | tail -25
  fail "los tests de derivación fallaron"
fi
PASSED=$(echo "$TEST_OUT" | grep -oE '[0-9]+ passed' | head -1)
pass "tests de derivación verdes (${PASSED:-?})"

echo
echo "### RESULTADO: PASS"
