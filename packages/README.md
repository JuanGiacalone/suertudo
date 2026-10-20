# packages

Shared code generated from the API contract.

The source of truth is the OpenAPI document that the Rust API exposes. The
clients for Dart and TypeScript are **generated**, not written by hand: a
hand-written client drifts from the contract and nobody notices until a
field is renamed in production.

| Package | Consumer | Generator |
|---|---|---|
| `api-client-dart` | `apps/mobile` | `openapi-generator` (dart) |
| `api-client-ts` | `apps/business-web` | `openapi-generator` (typescript-fetch) |

Both are generated in CI and committed, so a mobile build does not need the
generator installed. Regenerate them when the API contract changes, in the
same PR as the contract change.

**This directory is empty on purpose until the first endpoint is
implemented.** `TSK-F1-01` creates the first generated client.
