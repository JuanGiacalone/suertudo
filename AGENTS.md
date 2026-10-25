# AGENTS.md — Suertudo

> This file is the contract for every agent and every human working on this
> repository. Read it before writing a line of code. It encodes the decisions
> that are expensive to rediscover and that break silently when ignored.

---

## 1. What this repository is

Suertudo is a platform for **promotional raffles for local businesses**. A
participant reserves numbers for free in a business's raffle; the business
funds the prize; the draw uses a commit–reveal mechanism that any third party
can verify.

The product has two clients: a **Flutter** mobile app for participants and a
**Next.js** web panel for businesses. The backend is **Rust**, orchestrated by
**iii**.

**There are no payments.** The participant never pays. The business pays a
subscription. This is a decision, not an omission.

---

## 2. Where the truth lives

| Question | Answer lives in |
|---|---|
| What the product is and why | [`docs/PRD.md`](docs/PRD.md) |
| What the tables are and why | [`docs/data-model.md`](docs/data-model.md) |
| What the endpoints are | [`docs/api-contract.md`](docs/api-contract.md) |
| How the AI features work | [`docs/ai-spec.md`](docs/ai-spec.md) |
| How the system is built and why | [`docs/architecture.md`](docs/architecture.md) |
| Why the code of a device is verified by SMS | [`docs/toma-control-dispositivo-sms.md`](docs/toma-control-dispositivo-sms.md) |
| What the missing SMS provider has to satisfy | [`docs/contrato-proveedor-sms.md`](docs/contrato-proveedor-sms.md) |
| How the CI pipeline is operated, and what breaks it | [`docs/ci-pipeline.md`](docs/ci-pipeline.md) |
| What the verification found and the rules it produced | [`docs/verificacion-fase0.md`](docs/verificacion-fase0.md) |
| What to build next | [`docs/roadmap.md`](docs/roadmap.md) |
| Why the stack was measured and not assumed | [`docs/spike-iii-health.md`](docs/spike-iii-health.md) |

**A requirement has an ID and lives in the PRD.** `FR-ID-04`, `FR-PA-03`,
`NFR-PL-02`, `ADR-013`, `TSK-F1-12`. When you implement one, reference the
ID in the PR description. When you change a behaviour, change the document in
the same PR. An ID is never reused and never renumbered.

---

## 3. Layout

```
suertudo/
├── apps/
│   ├── mobile/          Flutter 3.47 app for participants
│   └── business-web/    Next.js 16 panel for businesses
├── services/            Rust crates, one per iii worker
│   ├── api/             HTTP surface: health, public endpoints, middleware
│   ├── db/              pool, migrations, tenant context
│   ├── draws/           the draw: the commit-reveal derivation
│   └── identity/        registration, sessions, authorization
├── infra/
│   ├── iii/             engine config, worker-compose, VERSION
│   └── docker-compose.yml
├── packages/            shared generated clients (OpenAPI -> Dart, TS)
├── scripts/
│   ├── dev.sh           bring the local stack up and down
│   └── smoke.sh         the smoke test of the backend, and the core of the CI
├── spikes/              experiments that answered a question
├── docs/                the documents listed above
└── AGENTS.md
```

**`spikes/` is disposable by design. `services/` and `scripts/` are not.**
When an experiment produces something the product depends on, it is promoted
out of the spike: the commit–reveal derivation became `services/draws/`, and
the verification script became `scripts/smoke.sh`.

---

## 4. Commands

### Rust

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo audit
```

Database tests need a live Postgres:

```bash
createdb suertudo_test
DATABASE_URL=postgres://suertudo:suertudo@127.0.0.1:5432/suertudo_test cargo test -p suertudo-db
```

### Flutter

```bash
cd apps/mobile
flutter analyze
flutter test
dart format --set-exit-if-changed .
```

### Next.js

```bash
cd apps/business-web
npm run lint
npm run build
npx tsc --noEmit
```

### The whole stack, without KVM

```bash
unshare -rm ./scripts/smoke.sh
```

That is the smoke test of the backend. It starts the engine, the registry
workers and the project workers, and checks the HTTP, queue and cron paths.
**Read [`docs/spike-iii-health.md`](docs/spike-iii-health.md) before changing
it** — it explains every assertion and why it exists.

---

## 5. Rules that do not bend

These are the ones that break silently. Each one cost a real bug.

### 5.1 The layering rule

Every service crate has this shape:

```
src/domain/     pure logic. No iii, no sqlx, no clock, no randomness.
src/ports.rs    traits for everything the domain needs from the outside.
src/adapters/   implementations of those traits.
src/handlers.rs  the iii surface: functions and their schemas.
src/main.rs     the worker: registration of functions and triggers.
```

**`domain/` and `ports/` must not import `iii_sdk` or `sqlx`.** The domain
takes the clock, the randomness and the database through traits. This is what
makes the rules testable without infrastructure and what keeps the backend
portable if the orchestrator changes.

### 5.2 The tenant never comes from the caller

**`business_id` is derived from the session token by the middleware and
injected into the handler context.** A handler that reads `business_id` from
a body or a path parameter is a bug, even if it looks harmless. The isolation
test exists to catch exactly that.

Every transaction sets the tenant context with `SET LOCAL`
(`app.business_id`, `app.user_id`). Without it the Row Level Security
policies fail **closed**: the query sees no rows. A missing context is a
bug, not an open door.

**The role the application connects with must not own the schema.** Postgres
does not apply the RLS policies to the owner of a table. If the API
connects as the role that created the schema, the second barrier does not
exist. The application role is created at deploy time, never in a
migration.

### 5.3 Migrations are never edited

A migration that has been merged is immutable. To change the schema you add a
new migration. Editing a merged migration makes the applied state and the
repository diverge, and the divergence is invisible until it breaks a deploy.

**The three invariants that must live in the database, not in the
application:**

| Invariant | Mechanism |
|---|---|
| A number can only be reserved once per raffle | `UNIQUE (raffle_id, number)` on `ticket_numbers` |
| A raffle is drawn exactly once | `UNIQUE (raffle_id)` on `draws` |
| A participant has at most N tickets per raffle | trigger `tickets_max_per_user_trg` — a unique index cannot express a configurable limit |

If you need a new invariant, add it as a constraint. A check in the handler
is a suggestion; a constraint is a guarantee.

**A table with a `business_id` is only written by the tenant that owns the
row.** The paths that legitimately cross that boundary are `SECURITY DEFINER`
functions that **bind the caller**: a function that mutates a tenant row
without checking who is calling is a cross-tenant write without a limit.

**`sqlx::migrate!` embeds the migration files at compile time.** Adding a
`.sql` file does not invalidate the build. After adding or changing a
migration, `touch services/db/src/lib.rs` and rebuild.

### 5.4 The commit–reveal formula is frozen

```
winner_index = u64_be(HMAC-SHA256(seed, "suertudo/v1|raffle:{id}|draw:{i}")[0..8]) mod pool_size
```

The golden test in `spikes/iii-health/src/derivation.rs` freezes the value
`144` for a known seed. **If you change the formula, that test fails on
purpose.** Update the PRD, `data-model.md` and the golden test in the same
commit. The formula is the trust promise of the product: a third party has to
be able to recompute the winner with four public values.

### 5.5 iii versions move together

The engine and the SDK are pinned in `infra/iii/VERSION` and they move in the
**same PR, one version at a time**. The 0.25 RC deprecated `register_trigger`;
against a 0.24 engine the function registers but its trigger is rejected and
the route never exists. That failure is silent: the worker looks healthy in
`engine::workers::list`.

### 5.6 The trigger types need their workers

The engine ships **no** queue or cron trigger type. `http`, `queue` and
`cron` are separate registry workers. Without them every trigger stays
`PENDING` forever and the only symptom is a log line.

`draws::close_due_raffles` and `draws::execute_draw` depend on `cron`. A
backend without that worker **never draws**.

### 5.7 The SDK needs an explicit `.await`

```rust
// Does not compile: the return type cannot be inferred.
RegisterFunction::new_async(move |p: Value| async move {
    publisher.trigger(TriggerRequest { .. })
})

// Compiles.
RegisterFunction::new_async(move |p: Value| async move {
    publisher.trigger(TriggerRequest { .. }).await
})
```

The compiler error (`IntoAsyncHandler` not satisfied) does not point at the
cause.

### 5.8 A worker has to stay alive

Rust threads do not keep the process running. A worker that registers its
functions and returns from `main` dies immediately. End with
`std::thread::park()` or `shutdown_async().await`.

### 5.9 Function names

```
<namespace>::<verb>_<resource>
```

Namespaces: `identity`, `businesses`, `raffles`, `participation`, `draws`,
`notifications`, `ai`, `crm`, `audit`, `admin`. The namespace is the one the
worker registers in; a function is only reachable inside it.

---

## 6. Commits

**Conventional Commits.** One commit is one reviewable unit of work: the
behaviour, its tests and the documentation change together.

```
feat(raffles): add the publish transition with a seed commitment
fix(participation): release the numbers of a cancelled ticket
docs(architecture): record the queue transport decision
chore(ci): run the smoke test without KVM
```

Never add `Co-Authored-By` or any AI attribution.

**A PR over 400 changed lines is split.** Chained PRs, each one reviewable on
its own, merged in order. A PR that changes behaviour without updating the
contract or the docs is incomplete.

---

## 7. Definition of done

- [ ] The acceptance criteria of the task are met and **verified**, not assumed.
- [ ] The new tests pass and the old ones did not break.
- [ ] `cargo fmt`, `cargo clippy -D warnings`, `flutter analyze` and
      `npm run lint` are clean.
- [ ] If the change touches the API contract, the OpenAPI is regenerated and
      the diff is in the PR.
- [ ] If the change touches the schema, there is a **new** migration.
- [ ] The affected document is updated in the same PR.
- [ ] The PR says **what** changed and **why**, not just the title.

---

## 8. Test-first

When there is a runnable deterministic test and a clear expected outcome,
observe it **failing** before implementing it, then make it pass, then
refactor with the tests green. One failing test per requested rule.

For every existing command or option the change touches, add one test proving
the previous behaviour still holds. No other cases.

A test that depends on the network, the real clock or an external provider is
not a unit test: inject the clock, mock the provider, use a test database.

---

## 9. Traps measured in this repository

These are not folklore. Each one was hit and fixed.

| Trap | What happens | What to do |
|---|---|---|
| The registry `cron` worker shares its process name with `/usr/sbin/cron` | `pkill -x cron` kills the system scheduler | Identify the workers by their **executable path** under `~/.iii/compose/packages/` |
| Linux truncates process names to 15 characters | `pkill -x iii-health-spike` matches nothing and the stale worker keeps serving | Use the truncated name, or match by path |
| `pgrep -f` matches the command line of the script that runs it | The precondition check lies and the teardown kills the shell | Use `pgrep -x`, or match by executable path |
| A registry worker reconnects to the engine on its own | The next run fails with `CONTAINER_NAME_TAKEN` | The teardown kills every worker by path |
| `compose::add` only edits the YAML | The worker stays `stopped` | `compose::up` is what reconciles the project |
| The engine logs `trigger type X is not registered` | The trigger is `PENDING` forever and the symptom is a feature that never runs | Assert on that log line in the smoke test |
| A golden value written by hand | The test fails for the wrong reason | Compute it with a second implementation |
| The queue DLQ on the `rabbitmq` transport | Dead letters are unreachable from iii | The production transport is `builtin` (`ADR-013`) |
| `sqlx::migrate!` embeds the migration files at compile time | A new `.sql` file is applied silently as if it did not exist | `touch services/db/src/lib.rs` and rebuild before applying |
| The owner of a Postgres table bypasses its RLS policies | The isolation looks enabled and is not | Connect with a role that does not own the schema |
| `#[tokio::test]` runs on a single thread | A test that pretends to reproduce a race does not reproduce it | Reproduce the state the loser finds, or test the contract the fix relies on |
| An RLS policy without `WITH CHECK` | The `USING` expression becomes the write permission | Every write policy needs its own `WITH CHECK` |
| A trigger that reads another table | That read is filtered by the RLS of the table, and the invariant goes silent | `SECURITY DEFINER` with a fixed `search_path`, plus a composite foreign key |
| A `SET LOCAL` leaves an empty placeholder on the session | A pooled connection reports the system context forever after | The system context has its own marker (`'system'`), never the empty string |
| A test that connects as the owner of the schema | Every RLS policy is bypassed and the test proves nothing | Run the isolation tests with the role of the deployment |
| A write policy with `OR user_id = current_user_id()` on a table that also has `business_id` | The branch is not bound to the business: a participant writes into another tenant's rows | Bind every write policy to the owner of the row, and route the legitimate paths through a `SECURITY DEFINER` function |
| A `SECURITY DEFINER` function that does not bind the caller | It is an unbounded cross-tenant write | Check `current_user_id()` inside the function, or refuse |
| A `SECURITY DEFINER` function that increments a counter | It drifts from the truth and it is not idempotent | Recompute the counter from the truth |
| A `RAISE EXCEPTION` without `ERRCODE` | The refusal becomes `P0001` and nobody translates it: the client gets a `500` | Every refusal carries a mapped SQLSTATE, and a test scans `pg_proc` for the ones that do not |
| A participant can write the platform columns of its own row | It can clear its own moderation or resurrect a deleted account | Freeze them in a trigger, with the exception the product grants |
| A check that only looks for the absence of an error | It cannot tell "everything is fine" from "nothing ran", and it reports PASS | Every check demands the mark of success: a number, a code, a body |
| A crate inside the workspace directory that is not a member | `cargo` refuses to build it from its own directory | Add it to `members` or to `exclude`; a spike with its own lockfile belongs in `exclude` |
| A refusal that the database answers with `false` instead of an exception | The transaction is rolled back and the attempt counter is lost with it | A refusal is a result: commit the transaction |
| A `text` parameter compared with a `citext` column | The operator resolves to the one of `text` and the comparison is **case-sensitive**: an address is found or not depending on how it was written, and the failure is silent (`202` with a link that never arrives) | Cast the parameter explicitly (`p_email::citext`) in every comparison against a `citext` column |
| The SMS of a verification goes to the participant that claims the device | An attacker who copied the token receives their own code and the transfer succeeds | The SMS goes to the participant that **owns** the device today |
| A CI step that pipes two `grep`s | The exit status of the first one is lost, and a scan that never ran looks exactly like a clean repository | Separate the scan from the verdict and check both exit statuses |
| A `grep` filter inside an assignment, under `bash -e` | An empty result exits 1, `set -e` kills the step before the verdict, and an empty result is exactly what "nothing found" means | Put the filter in the condition of an `if`, or add `|| true` |
| The verdict of a filter taken from its exit status | An empty scan still produces a blank line, the filter keeps it, and the status says "found" when there is nothing | The verdict is the **content**: the command substitution eats the trailing newline |
| A check whose branches are never executed | It only works on the day it was written: the guard that finds focused tests broke in the very branch it was written to exempt, and only a review noticed | The logic goes in a script, and the script has a test; the pipeline runs the test before the check |
| `cargo build` after changing the schema | It is incremental, it does not re-expand the macros, and the offline cache of sqlx stays empty while the command says `Finished` | `touch services/db/src/lib.rs` before building, and regenerate the cache from zero |
| The expansion of the sqlx macros never prunes | A deleted query leaves its cache file behind forever, and nothing in the pipeline notices | Regenerate the cache from zero and prune by hand: `docs/ci-pipeline.md` §5.2 |
| `flutter test` with an empty `test/` directory | It exits 1: the job is red on its first day, before anyone wrote a test | A smoke test of the app, as `apps/mobile/test/app_smoke_test.dart` |
| A script that starts a binary it never compiled | The spike is not a member of the workspace, so `cargo build --workspace` does not build it, and in CI the binary does not exist | The script compiles what it starts, with `--locked` |
| `cargo` without `--locked` | It re-resolves the dependencies in silence, and the pipeline audits and tests a tree that is not the committed one | `--locked` on every cargo command of the pipeline |
| The content of a repository file echoed to the stdout of a CI step | GitHub Actions reads the stdout for workflow commands, and the file can inject one into the log | The content goes to the stderr; the `::error::` annotation is a static sentence on the stdout |

---

## 10. Before you start

1. Read this file.
2. Read the section of the PRD that covers your task, and the contract it
   references.
3. Check that the task's dependencies are merged.
4. Write the failing test first, if the task calls for it.
5. Ask one question if a product decision is genuinely open. Do not invent one.
