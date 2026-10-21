# Spike: iii stack

**What this is:** the experiment that answered the question *does the iii
stack work for Suertudo, and on what does it depend?* It is the answer to the
risks that could have sunk Fase 0: `R1` (the SDK in release candidate), `R2`
(workers that need `/dev/kvm`) and the queue path that `draws::execute_draw`
and `notifications::dispatch` depend on.

**Status: PASS.** All three are resolved. See [Findings](#findings).

## What lives here and what does not

This folder is a **spike**: it is allowed to be disposable. But two of its
artifacts turned out to be product, and they were promoted out:

| Was here | Is now | Why |
|---|---|---|
| `src/derivation.rs` | [`services/draws/src/derivation.rs`](../../services/draws/src/derivation.rs) | The commit–reveal formula is the **trust promise** of the product. Its golden test has to live with the code that runs it, not in a folder named "spikes" that the next person cleans up. |
| `verify.sh` | [`scripts/smoke.sh`](../../scripts/smoke.sh) | It is the pipeline of the CI (`TSK-F0-05`), not an experiment. |

**What stays is the experiment:** the probe worker (`src/main.rs`,
`src/queue_probe.rs`) and the engine configuration that drives it. The probe
worker exists to exercise the engine; it is not product code and it is not
supposed to grow.

This crate is **excluded from the Rust workspace** on purpose: it has its own
`Cargo.lock` and its own pin of `iii-sdk`, and it must not inherit the
versions of the project.

> **Por qué existe este spike, cómo funciona mecánicamente y qué hay que
> recordar para futuros cambios:** [`docs/spike-iii-health.md`](../../docs/spike-iii-health.md).
> Este README es el manual operativo; aquel documento es el rationale.

---

## Run it

```bash
# Full stack, on this machine
./scripts/smoke.sh

# Force the no-KVM path: mask /dev/kvm in a private mount namespace
unshare -rm ./scripts/smoke.sh

# Test the rabbitmq transport instead (needs a broker on 127.0.0.1:5672)
QUEUE_ADAPTER=rabbitmq unshare -rm ./scripts/smoke.sh
```

The script is idempotent: it cleans up every process it starts, so it can run
twice in a row.

**Prerequisites**

| Tool | Version used | Note |
|---|---|---|
| `iii` engine | 0.24.6 | `curl -fsSL https://install.iii.dev/iii/main/install.sh \| sh` |
| Rust | 1.99.1 stable | `curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs \| sh -s -- -y` |
| `iii-sdk` (Rust) | **0.24.6** | Pinned. See Finding 3. |
| Registry workers | `http` 0.22.7, `queue` 0.21.22, `cron` 0.21.26 | Pulled by `compose::add` |

---

## What it proves

| # | Check | Result |
|---|---|---|
| 0 | No leftover iii process holds the namespace | Guard, fails fast with a clear message |
| 1 | `/dev/kvm` state, reported in hex (`a,e8` = real KVM) | Informational |
| 2 | Engine starts and listens on `ws://127.0.0.1:49134` | PASS |
| 3 | Registry workers `http`, `queue`, `cron` reach state `ready` | PASS |
| 4 | A Rust worker registers a function and an `http` trigger | PASS |
| 5 | `GET /v1/health` → `200` through the engine | PASS |
| 6 | `health::check` callable directly through the engine | PASS |
| 7 | No `iii-worker` process: no microVM was booted | PASS |
| 8 | A published message reaches its subscriber | PASS |
| 9 | A named function queue accepts `TriggerAction::Enqueue` | PASS |
| 10 | A failing consumer retries 3 times, then dead-letters | PASS |
| 11 | The DLQ can be listed, redriven and discarded | PASS |
| 12 | A cron trigger actually fires | PASS |
| 13 | Golden test of the derivation formula | PASS (7/7) |

---

## Findings

### 1. The stack runs without KVM — `R2` resolved

Every worker used here is a **host process**. The registry `http` worker is
`Type: binary`, and a `path://` worker runs as a child process of the compose
daemon. No microVM is ever booted: check 7 asserts that no `iii-worker` process
exists at any point.

This is what `ADR-011` assumed, now verified. The CI pipeline does not need
`/dev/kvm`.

### 2. A leftover worker blocks the next run — operational trap

The registry workers (`http`, `queue`, `cron`) **reconnect to the engine on
their own** after a drop. If one survives a run, the next `compose::up` fails
with:

```
CONTAINER_NAME_TAKEN: a worker named 'http' is already registered in namespace 'spike'
```

Three details that cost time:

- Their **process name is the package name** (`http`, `queue`, `cron`), not
  `iii-*`. And `cron` **collides with the system scheduler** (`/usr/sbin/cron`):
  a blanket `pkill -x cron` kills the machine's cron. `verify.sh` therefore
  identifies registry workers by their **executable path** under
  `~/.iii/compose/packages/`, never by name.
- Linux truncates process names to 15 characters, so the spike worker is
  `iii-health-spik`, not `iii-health-spike`. `pkill -x` with the long name
  matches nothing, silently, and the stale worker keeps serving.
- `compose::add` **does not restart** a worker that is already declared in the
  YAML: it only edits the file. `compose::up` is what reconciles the project.

**Consequence for CI:** the smoke test must clean up by executable path, or it
will pass locally and fail on the second run. `architecture.md` §9.2 needs this
detail.

### 3. The SDK version must match the engine — `R1` scoped

The engine installed is **0.24.6**; `iii-sdk` 0.24.6 is the matching stable
crate. Measured against the engine:

| SDK crate | Compiles | Function registration | `http` trigger registration |
|---|---|---|---|
| `0.24.6` | yes | works | works — route served |
| `0.25.0-rc.4` | yes | **works** | **REJECTED** |

With `0.25.0-rc.4` the function registers fine, but the trigger is refused:

```
[WARN] iii::trigger Trigger registration rejected by provider;
       parked as pending until the trigger type re-registers
```

The route never appears in the HTTP worker. The RC SDK also deprecates
`register_trigger` in favour of `iii.observe(Source::function(..).args(..), target)`,
which the 0.24.6 engine does not implement.

**Conclusion:** a 0.25 RC SDK against a 0.24.6 engine is a **silent
half-broken** state — functions work, routes do not. This is worse than a
compile error, and it justifies pinning both to `0.24.6`.

`architecture.md` §8.6 already requires the engine and SDK to move together,
in the same PR, one version at a time. This spike gives that rule teeth: the
smoke test must assert a **route responds 200**, not just that a worker
registered. A worker that registers its function but not its trigger looks
healthy in `engine::workers::list`.

### 4. The derivation formula is reproducible by a third party

`src/derivation.rs` implements the commit–reveal formula from
`docs/PRD.md` §7.2:

```
winner_index = u64_be(HMAC-SHA256(seed, "suertudo/v1|raffle:{id}|draw:{i}")[0..8]) % pool_size
```

The golden values were computed with an **independent implementation** (Python's
`hmac`/`hashlib`), not with the Rust code, and then frozen:

| Input | Value |
|---|---|
| `commitment(0xAB * 32)` | `9a2db2e23f1504cd056606553ac049c5e718e8f9ce9233876df1a7a1821af885` |
| `seed = 0..31`, raffle `3f8a1c22-…`, draw 0, pool 1000 | `144` |
| Redraw sequence (draws 0..3) | `144, 820, 74, 885` |

7 tests cover: commitment correctness, determinism, the golden value, the
redraw sequence, raffle-id separation, and pool bounds.

**Note on process:** the first run of these tests failed, because the golden
values had been written by hand instead of computed. That is the exact failure
mode the golden test exists to catch, and it is why the values are now derived
from an independent implementation.

### 5. The engine ships no queue or cron trigger type

Neither `durable:subscriber` nor `cron` is built into the engine 0.24.6. Each
needs its own registry worker:

| Trigger type | Registry worker | Version |
|---|---|---|
| `http` | `http` | 0.22.7 |
| `durable:subscriber` | `queue` | 0.21.22 |
| `cron` | `cron` | 0.21.26 |

Without them the trigger stays `PENDING` forever, and the engine says so:

```
[WARN] iii::trigger Trigger ... was NOT activated: trigger type cron is not registered.
If this persists, the cron worker is missing — run:
iii trigger -n <compose-daemon-namespace> compose::add worker=cron
```

`draws::close_due_raffles` and `draws::execute_draw` depend on `cron`. A
backend missing the worker **never draws**, and the symptom is a raffle that
never closes, not an error.

### 6. `TriggerAction::Enqueue` works, but the queue must exist first

The `queue` worker docs warn that `TriggerAction::Enqueue` needs an engine
that routes named enqueue actions through `engine::queue::enqueue`. **The
0.24.6 engine has that cut.** Verified: enqueueing returns a
`messageReceiptId` and the target function receives the message.

The named queue has to be defined first, and its schema is strict:

| Trap | Error |
|---|---|
| `type: concurrent` | `queue 'X' type must be either 'standard' or 'fifo'` |
| `type: fifo` without `message_group_field` | `queue 'X' with type 'fifo' requires message_group_field` |
| `function_id` inside the config | `unknown field 'function_id', expected one of 'max_retries', 'concurrency', ...` |

**The target function travels with the message, not with the queue.** A named
queue defines the transport; each enqueue carries its `function_id`.

---

## What this spike does not cover

- Postgres and Redis.
- `iii-worker` binary workers and the `bundle` kind, which do need a VM.
- Auth, RBAC middleware and namespaces beyond a single one.
- Crash behaviour mid-retry (what happens to in-flight messages if the queue
  worker dies).
- The upstream iii fix `MOT-3904` for RabbitMQ DLQ addressing.

**Next step:** `TSK-F0-01` (monorepo skeleton), `TSK-F0-03` (schema) and
`TSK-F0-04` (identity) can start. The production queue transport is decided:
`builtin` (`ADR-013`).

---

## Files

| File | Purpose |
|---|---|
| `verify.sh` | The end-to-end check. Idempotent, self-cleaning. |
| `src/main.rs` | Minimal Rust worker: one function, one `http` trigger. |
| `src/derivation.rs` | The commit–reveal formula and its golden tests. |
| `src/lib.rs` | Library root, so the derivation tests run independently. |
| `config.yaml` | Engine config. `workers: []` keeps the legacy built-in HTTP service off. |
| `worker-compose.yaml` | Declares the registry `http` worker. |
