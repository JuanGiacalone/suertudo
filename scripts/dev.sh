#!/usr/bin/env bash
# Bring up the local Suertudo stack.
#
#   ./scripts/dev.sh up      start the data services and the iii engine
#   ./scripts/dev.sh down    stop everything
#   ./scripts/dev.sh status  show what is running
#   ./scripts/dev.sh test    run the whole test suite
#
# The iii engine and the registry workers run as host processes. They do not
# need /dev/kvm: see docs/spike-iii-health.md.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

III_NS="suertudo"
III_URL="ws://127.0.0.1:49134"
COMPOSE_FILE="infra/iii/worker-compose.yaml"

log() { printf '\033[1m==>\033[0m %s\n' "$1"; }

require() {
  command -v "$1" >/dev/null 2>&1 || { echo "missing: $1" >&2; exit 1; }
}

up() {
  require iii
  require cargo

  log "data services"
  if command -v docker >/dev/null 2>&1; then
    docker compose -f infra/docker-compose.yml up -d
  else
    echo "docker not available: assuming postgres, rabbitmq and redis are already running"
  fi

  log "database"
  psql "postgres://suertudo:suertudo@127.0.0.1:5432/postgres" -c \
    "SELECT 1 FROM pg_database WHERE datname='suertudo'" >/dev/null 2>&1 \
    || psql "postgres://suertudo:suertudo@127.0.0.1:5432/postgres" \
         -c "CREATE DATABASE suertudo OWNER suertudo"

  log "migrations"
  cargo run --release -p suertudo-db --bin suertudo-db -- migrate 2>/dev/null \
    || echo "no migrate binary yet: run the migrations with sqlx-cli"

  log "iii engine"
  (setsid iii --config infra/iii/config.yaml --no-update-check \
     > "$REPO_ROOT/.iii-engine.log" 2>&1 &)
  sleep 4

  log "compose daemon"
  (setsid iii compose --namespace "$III_NS" --engine "$III_URL" \
     > "$REPO_ROOT/.iii-compose.log" 2>&1 &)
  sleep 6

  log "workers"
  iii trigger -n "$III_NS" compose::up file="./$COMPOSE_FILE" >/dev/null 2>&1
  iii trigger -n "$III_NS" compose::status file="./$COMPOSE_FILE"

  log "done. logs: .iii-engine.log .iii-compose.log"
}

down() {
  log "stopping the workers and the engine"
  for pid in $(pgrep -f "$HOME/.iii/compose/packages/" 2>/dev/null); do
    kill -9 "$pid" 2>/dev/null
  done
  pkill -9 -x iii 2>/dev/null
  pkill -9 -x "iii:c:$III_NS" 2>/dev/null
  pkill -9 -x iii-health-spik 2>/dev/null
  pkill -9 -x suertudo-api 2>/dev/null
  pkill -9 -x suertudo-identity 2>/dev/null

  if command -v docker >/dev/null 2>&1; then
    log "data services"
    docker compose -f infra/docker-compose.yml down
  fi
}

status() {
  log "iii processes"
  iii ps 2>/dev/null || echo "iii not available"
  log "ports"
  for port in 49134 3111 5432 5672 6379; do
    if (exec 3<>/dev/tcp/127.0.0.1/"$port") 2>/dev/null; then
      echo "  $port open"
      exec 3<&- 2>/dev/null
    else
      echo "  $port closed"
    fi
  done
}

test_all() {
  log "rust"
  cargo test --workspace
  log "flutter"
  (cd apps/mobile && flutter test)
  log "next.js"
  (cd apps/business-web && npm run lint)
  log "smoke test"
  unshare -rm ./spikes/iii-health/verify.sh
}

case "${1:-up}" in
  up) up ;;
  down) down ;;
  status) status ;;
  test) test_all ;;
  *) echo "usage: $0 {up|down|status|test}" >&2; exit 1 ;;
esac
