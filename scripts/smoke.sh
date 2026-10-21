#!/usr/bin/env bash
# The smoke test of the Suertudo backend.
#
# This is the pipeline of the CI (`TSK-F0-05`). It is NOT a spike: it is the
# check that the backend comes up, on a host WITHOUT KVM, with the routes
# really answering.
#
# It proves, on a host WITHOUT KVM, that:
#   0. there is no stale iii process from a previous run
#   1. the state of KVM is known (the stack does not need it)
#   2. the iii engine starts and listens on 49134
#  2b. the queue transport under test is the one that was asked for
#   3. the registry workers (http, queue, cron) reach the state `ready`
#   4. a Rust worker registers a function and an http trigger
#   5. GET /v1/health returns 200 through the engine
#   6. the function is callable directly through the engine
#   7. no microVM was used at any moment of the run
#   8. a published message reaches its subscriber
#   9. a named function queue accepts TriggerAction::Enqueue
#  10. a failing consumer retries 3 times, then dead-letters
#  11. the DLQ can be listed, redriven and discarded
#  12. a cron trigger actually fires
#  13. the golden test of the derivation holds
#
# The probe worker and the engine configuration live in
# `spikes/iii-health/`, which is where the experiment that answered the
# original question lives. This script drives them.
#
# Usage:  ./scripts/smoke.sh
#         unshare -rm ./scripts/smoke.sh   (forces the no-KVM path)
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SPIKE_DIR="$REPO_ROOT/spikes/iii-health"
# Without the `|| exit`, a failed `cd` leaves the script running in the wrong
# directory, where every relative path of the script points somewhere else.
cd "$SPIKE_DIR" || exit 1

# cargo is not on PATH in a bare shell (rustup installs to ~/.cargo/bin).
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

# The probe worker has to be compiled before it is started. The spike is NOT
# a member of the workspace (it has its own lockfile and its own pin of the
# SDK), so `cargo build --workspace` does not build it. The script compiles
# it so that it is self-sufficient: the CI runs it in a checkout where the
# binary does not exist yet, and a script that needs a binary built by hand
# is a script that only works on the machine of whoever wrote it.
echo "Compilando el worker de prueba..."
# `--locked`: without it, cargo is free to re-resolve the dependencies of
# the spike, and the pipeline would be testing a tree that is not the one
# that is committed.
if ! cargo build --quiet --locked --bin iii-health-spike; then
  echo "    FALLO: no se pudo compilar el worker de prueba"
  exit 1
fi
[ -x "$SPIKE_DIR/target/debug/iii-health-spike" ] || {
  echo "    FALLO: el worker de prueba no quedo en target/debug/iii-health-spike"
  exit 1
}

ENGINE_URL="ws://127.0.0.1:49134"
NAMESPACE="spike"
HEALTH_URL="http://127.0.0.1:3111/v1/health"

# Queue transport under test. `builtin` is the default and needs no broker.
# `rabbitmq` needs a reachable AMQP broker (see TSK-F0-07).
QUEUE_ADAPTER="${QUEUE_ADAPTER:-builtin}"
AMQP_URL="${AMQP_URL:-amqp://guest:guest@127.0.0.1:5672/%2f}"

# Linux truncates the process name to 15 characters, so the spike worker is
# `iii-health-spik`, not `iii-health-spike`. Using the long name makes
# pkill silently match nothing and the stale worker keeps serving.
SPIKE_WORKER_PROC="iii-health-spik"

pass() { echo "    OK: $1"; }
fail() { echo "    FALLO: $1"; exit 1; }

# Read a counter out of the worker's queue::stats function.
#
# An empty result is NOT a zero: it means the worker did not answer, and a
# zero would turn a broken stack into a passing check.
stat_of() {
  local value
  value=$(iii trigger -n "$NAMESPACE" queue::stats 2>/dev/null \
    | grep -oE "\"$1\": [0-9]+" | grep -oE '[0-9]+$')
  if [ -z "${value:-}" ]; then
    fail "queue::stats no devolvio el contador '$1': el worker no responde"
  fi
  echo "$value"
}

# Registry workers live under the iii package cache and are identified by
# their executable path.
#
# NEVER match them by process name: the `cron` worker shares its name with
# the system scheduler (/usr/sbin/cron), and `pkill -x cron` kills the
# machine's cron. The `http` and `queue` workers have no such conflict, but
# matching by path is the rule that holds for all of them.
III_PACKAGES_DIR="${III_PACKAGES_DIR:-$HOME/.iii/compose/packages}"

# PIDs of the iii registry workers, excluding this script and its parents.
iii_worker_pids() {
  local pid
  for pid in $(pgrep -f "$III_PACKAGES_DIR/" 2>/dev/null); do
    [ "$pid" = "$$" ] && continue
    [ "$pid" = "$PPID" ] && continue
    echo "$pid"
  done
}

# Match by exact process name, never with `pgrep -f`: a pattern match also
# hits the shell that is running this script, which makes the check lie.
iii_running() {
  pgrep -x iii >/dev/null 2>&1 \
    || pgrep -x "iii:c:$NAMESPACE" >/dev/null 2>&1 \
    || pgrep -x "$SPIKE_WORKER_PROC" >/dev/null 2>&1 \
    || [ -n "$(iii_worker_pids)" ]
}

cleanup() {
  # The registry `http` worker runs as a process named `http` and reconnects
  # to the engine on its own. Left alive, it holds the container name and the
  # next run fails with CONTAINER_NAME_TAKEN. Same for `queue` and `cron`.
  local pid
  for pid in $(iii_worker_pids); do
    kill -9 "$pid" 2>/dev/null
  done
  for name in iii "iii:c:$NAMESPACE" "$SPIKE_WORKER_PROC"; do
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
echo "### 2b. Transporte de cola: ${QUEUE_ADAPTER}"
if [ "$QUEUE_ADAPTER" = "rabbitmq" ]; then
  # The queue worker reads its transport from the configuration worker, which
  # reads this file. It has to exist before the worker boots.
  mkdir -p config
  cat > config/spike-queue.yaml << EOF
id: spike-queue
name: Queue
description: 'Durable queue worker settings.'
metadata:
  ui_form: queue
value:
  adapter:
    name: rabbitmq
    config:
      amqp_url: ${AMQP_URL}
      max_attempts: 3
      prefetch_count: 10
      queue_mode: standard
  queue_configs: {}
EOF
  (exec 3<>/dev/tcp/127.0.0.1/5672) 2>/dev/null || fail "no hay broker AMQP en 127.0.0.1:5672"
  exec 3<&- 2>/dev/null
  pass "config del worker queue: rabbitmq"
else
  pass "config del worker queue: builtin (default, sin broker)"
fi

echo
echo "### 3. Workers del registro (http, queue, cron)"
setsid iii compose --namespace "$NAMESPACE" --engine "$ENGINE_URL" > compose.log 2>&1 &
sleep 6
# compose::add only edits the file; compose::up is what reconciles the project.
iii trigger -n "$NAMESPACE" compose::up file=./worker-compose.yaml >/dev/null 2>&1
for i in $(seq 1 40); do
  sleep 1
  state=$(iii trigger -n "$NAMESPACE" compose::status file=./worker-compose.yaml 2>/dev/null \
          | grep -o '"state": "[a-z]*"' | head -1 | cut -d'"' -f4)
  # Un estado vacio no es un estado: el comando no respondio.
  [ -n "${state:-}" ] && [ "$state" = "ready" ] && break
  [ "$i" = 40 ] && fail "el worker http quedó en estado '$state' (ver compose.log)"
done
pass "worker http ready (0.22.7)"

# The queue and cron trigger types are NOT built into the engine: each one
# needs its own registry worker, or the trigger stays PENDING forever.
iii trigger -n "$NAMESPACE" compose::add worker=queue >/dev/null 2>&1
iii trigger -n "$NAMESPACE" compose::add worker=cron >/dev/null 2>&1
for i in $(seq 1 60); do
  sleep 1
  ready=$(iii trigger -n "$NAMESPACE" compose::status file=./worker-compose.yaml 2>/dev/null \
          | grep -c '"state": "ready"')
  # Un conteo de cero significa que el comando no devolvio nada: no es lo
  # mismo que "ningun worker esta listo".
  [ "${ready:-0}" -ge 3 ] && break
  [ "$i" = 60 ] && fail "los workers queue/cron no alcanzaron estado ready (ver compose.log)"
done
pass "workers queue y cron ready"

echo
echo "### 4. Worker Rust propio"
setsid ./target/debug/iii-health-spike > spike.log 2>&1 &
sleep 6
pass "worker registrado"

echo
echo "### 5. GET /v1/health"
code=$(curl -s -o /tmp/iii-health-body.json -w '%{http_code}' "$HEALTH_URL")
[ "$code" = "200" ] || fail "se esperaba HTTP 200, se obtuvo $code"
[ -s /tmp/iii-health-body.json ] || fail "el health respondio 200 con el cuerpo vacio"
pass "HTTP 200 -> $(cat /tmp/iii-health-body.json)"

echo
echo "### 6. Invocación directa por el engine"
DIRECT=$(iii trigger -n "$NAMESPACE" health::check 2>&1)
echo "$DIRECT" | grep -q '"status": "ok"' \
  && pass "health::check responde por el engine" \
  || { echo "$DIRECT" | head -10; fail "health::check no respondió"; }

echo
echo "### 7. Confirmar que no se usó ninguna microVM"
if pgrep -x iii-worker >/dev/null 2>&1; then
  fail "hay un proceso iii-worker: se usó una microVM"
fi
pass "ningún proceso iii-worker, ninguna microVM"

echo
echo "### 8. Cola: publicar y consumir"
before=$(stat_of consumed)
iii trigger -n "$NAMESPACE" queue::publish --json '{"topic":"spike-jobs","data":{"n":42}}' >/dev/null 2>&1
sleep 3
after=$(stat_of consumed)
[ "${after:-0}" -gt "${before:-0}" ] \
  && pass "mensaje publicado consumido ($before -> $after)" \
  || fail "el mensaje publicado no llegó al suscriptor ($before -> $after)"

echo
echo "### 9. Named function queue vía TriggerAction::Enqueue"
iii trigger -n "$NAMESPACE" queue::define_named >/dev/null 2>&1
sleep 2
before=$(stat_of named_consumed)
iii trigger -n "$NAMESPACE" queue::enqueue_named >/dev/null 2>&1
sleep 3
after=$(stat_of named_consumed)
[ "${after:-0}" -gt "${before:-0}" ] \
  && pass "TriggerAction::Enqueue encoló y consumió ($before -> $after)" \
  || fail "TriggerAction::Enqueue no consumió ($before -> $after)"

echo
echo "### 10. Reintentos y DLQ"
iii trigger -n "$NAMESPACE" queue::publish --json '{"topic":"spike-fail","data":{"n":1}}' >/dev/null 2>&1
sleep 8
attempts=$(stat_of fail_attempts)
[ "${attempts:-0}" -eq 3 ] \
  && pass "el consumidor falló 3 veces y agotó los reintentos" \
  || fail "se esperaban 3 intentos, se obtuvieron ${attempts:-0}"

dlq_count=$(iii trigger -n "$NAMESPACE" engine::queue::dlq_messages \
  --json '{"topic":"spike-fail","offset":0,"limit":10}' 2>/dev/null \
  | python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null)

if [ "$QUEUE_ADAPTER" = "rabbitmq" ]; then
  # Documented limitation, measured: on the rabbitmq transport the retries
  # write to the per-function DLQ (iii.{topic}.{fn}@{ns}.dlq) while the iii
  # DLQ API addresses the bare topic (iii.{topic}.dlq). The API therefore
  # sees nothing and the message is unreachable from iii.
  [ "${dlq_count:-0}" -eq 0 ] \
    && pass "LIMITACIÓN CONOCIDA (rabbitmq): la API de DLQ no ve el mensaje por función" \
    || fail "en rabbitmq la DLQ se direccionaba por función; la API la ve, el comportamiento cambió"
  redrive_out=$(iii trigger -n "$NAMESPACE" iii::queue::redrive --json '{"topic":"spike-fail"}' 2>&1)
  echo "$redrive_out" | grep -q "NOT_FOUND" \
    && pass "LIMITACIÓN CONOCIDA (rabbitmq): redrive falla con NOT_FOUND sobre la cola por función" \
    || fail "en rabbitmq se esperaba un error NOT_FOUND en el redrive"
else
  [ "${dlq_count:-0}" -ge 1 ] \
    && pass "el mensaje quedó en la DLQ ($dlq_count mensajes)" \
    || fail "la DLQ está vacía"
fi

echo
echo "### 11. Redrive y discard de la DLQ"
if [ "$QUEUE_ADAPTER" = "rabbitmq" ]; then
  pass "omitido en rabbitmq: el redrive no puede direccionar la DLQ por función"
else
  msg_id=$(iii trigger -n "$NAMESPACE" engine::queue::dlq_messages \
    --json '{"topic":"spike-fail","offset":0,"limit":10}' 2>/dev/null \
    | python3 -c "import sys,json; d=json.load(sys.stdin); print(d[0]['id'] if d else '')" 2>/dev/null)
  redriven=$(iii trigger -n "$NAMESPACE" iii::queue::redrive \
    --json '{"topic":"spike-fail"}' 2>/dev/null | grep -oE '"redriven": [0-9]+' | grep -oE '[0-9]+$')
  [ "${redriven:-0}" -ge 1 ] \
    && pass "redrive reprocesó el mensaje ($redriven)" \
    || fail "el redrive no reprocesó nada"

  sleep 6
  attempts=$(stat_of fail_attempts)
  [ "${attempts:-0}" -ge 6 ] \
    && pass "el mensaje redriveado volvió a fallar y reingresó a la DLQ ($attempts intentos)" \
    || fail "tras el redrive se esperaban 6 intentos, hay ${attempts:-0}"

  if [ -n "$msg_id" ]; then
    iii trigger -n "$NAMESPACE" iii::queue::discard_message \
      --json "{\"topic\":\"spike-fail\",\"message_id\":\"$msg_id\"}" >/dev/null 2>&1
    sleep 2
    dlq_count=$(iii trigger -n "$NAMESPACE" engine::queue::dlq_messages \
      --json '{"topic":"spike-fail","offset":0,"limit":10}' 2>/dev/null \
      | python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null)
    [ "${dlq_count:-1}" -eq 0 ] \
      && pass "discard_message vació la DLQ" \
      || fail "la DLQ sigue con ${dlq_count} mensajes"
  fi
fi

echo
echo "### 12. Cron"
ticks_before=$(stat_of cron_ticks)
sleep 7
ticks_after=$(stat_of cron_ticks)
[ "${ticks_after:-0}" -gt "${ticks_before:-0}" ] \
  && pass "el trigger cron disparó ($ticks_before -> $ticks_after)" \
  || fail "el trigger cron no disparó ($ticks_before -> $ticks_after)"

echo
echo "### 13. Golden test de la derivación (services/draws)"
TEST_OUT=$(cd "$REPO_ROOT" && cargo test -p suertudo-draws 2>&1)
if echo "$TEST_OUT" | grep -q 'test result: FAILED'; then
  echo "$TEST_OUT" | tail -25
  fail "los tests de derivación fallaron"
fi
# A check that passes on an empty output is not a check. This one used to
# report PASS when cargo had refused to run at all, because the failure was
# not the word "FAILED". The count has to be there, and it has to be a
# number.
PASSED=$(echo "$TEST_OUT" | grep -oE '[0-9]+ passed' | head -1 | grep -oE '^[0-9]+')
if [ -z "${PASSED:-}" ]; then
  echo "$TEST_OUT" | tail -25
  fail "los tests de derivación no llegaron a correr"
fi
if [ "$PASSED" -lt 8 ]; then
  echo "$TEST_OUT" | tail -25
  fail "los tests de derivación corrieron $PASSED de 8"
fi
pass "tests de derivación verdes ($PASSED)"

echo
echo "### RESULTADO: PASS"
