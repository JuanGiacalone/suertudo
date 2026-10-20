# Spike: iii health

Purpose: prove — before writing the monorepo — that the iii stack for Suertudo
works on a host **without KVM**, and that the commit–reveal derivation formula
is reproducible by a third party.

This is the answer to the two risks that could have sunk Fase 0:
`R1` (SDK in release candidate) and `R2` (workers need `/dev/kvm`).

**Status: PASS.** Both risks are resolved. See [Findings](#findings).

---

## Run it

```bash
# Full stack, on this machine
./spikes/iii-health/verify.sh

# Force the no-KVM path: mask /dev/kvm in a private mount namespace
unshare -rm ./spikes/iii-health/verify.sh
```

The script is idempotent: it cleans up every process it starts, so it can run
twice in a row.

**Prerequisites**

| Tool | Version used | Note |
|---|---|---|
| `iii` engine | 0.24.6 | `curl -fsSL https://install.iii.dev/iii/main/install.sh \| sh` |
| Rust | 1.99.1 stable | `curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs \| sh -s -- -y` |
| `iii-sdk` (Rust) | **0.24.6** | Pinned. See Finding 3. |

---

## What it proves

| # | Check | Result |
|---|---|---|
| 0 | No leftover iii process holds the namespace | Guard, fails fast with a clear message |
| 1 | `/dev/kvm` state, reported in hex (`a,e8` = real KVM) | Informational |
| 2 | Engine starts and listens on `ws://127.0.0.1:49134` | PASS |
| 3 | Registry `http` worker v0.22.7 reaches state `ready` | PASS |
| 4 | A Rust worker registers a function and an `http` trigger | PASS |
| 5 | `GET /v1/health` → `200` through the engine | PASS |
| 6 | `health::check` callable directly through the engine | PASS |
| 7 | No `iii-worker` process: no microVM was booted | PASS |
| 8 | Golden test of the derivation formula | PASS (7/7) |

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

The registry `http` worker **reconnects to the engine on its own** after a
drop. If one survives a run, the next `compose::up` fails with:

```
CONTAINER_NAME_TAKEN: a worker named 'http' is already registered in namespace 'spike'
```

The worker's process name is `http` (the package binary), not `iii-*`.
`verify.sh` kills it explicitly in its cleanup trap.

**Consequence for CI:** the smoke test must clean up by process name, or it
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

---

## What this spike does not cover

- Postgres, Redis, RabbitMQ and the `queue` worker. The queue path is what
  `notifications` and `draws` depend on, and it is **not** verified here.
- The `durable:subscriber` trigger type (the engine logs a warning that it is
  missing until the `queue` worker is added).
- Cron triggers.
- `iii-worker` binary workers and the `bundle` kind, which do need a VM.
- Auth, RBAC middleware and namespaces beyond a single one.

**Next step:** `TSK-F0-03` (schema) and `TSK-F0-04` (identity) can start, but
the queue path should be proven before building `draws` and `notifications` on
top of it. Suggest a `TSK-F0-06` for the queue.

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
