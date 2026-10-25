# Rate limits de la API

**Objetivo:** cerrar el vector de abuso del registro por magic link con rate limits que viven en Redis, no en la memoria del proceso.
**Rama:** `feat/fase0-cimientos` · **Estrategia:** commits de unidad de trabajo (~400 líneas) · **Runner de tests:** `cargo test --workspace`, más Redis real para el adaptador.

---

## Specs

### S1 — Los grupos de rate limit

Del contrato, `docs/api-contract.md` §1.6, **verbatim**:

| Grupo | Límite | Ventana | Alcance |
|---|---|---|---|
| `auth` | 10 | 5 min | Por IP y por email |
| `auth:link` | 5 | 15 min | Por email |
| `raffles:read` | 120 | 1 min | Por token |
| `tickets:create` | 10 | 5 min | Por usuario y por IP |
| `ai:generate` | 20 | 1 h | Por business |
| `export` | 5 | 1 h | Por business |
| `public` | 300 | 1 min | Por IP |

**Alcance de la implementación:** los siete grupos quedan **definidos** con su límite, su ventana y su alcance, y el mecanismo los aplica. Los grupos **con endpoints en Fase 0** son `auth` y `auth:link`, y quedan **aplicados y probados contra un Redis real**. Los otros cinco grupos no tienen ningún endpoint todavía (`raffles:read` llega con `TSK-F1-03`, `tickets:create` con `TSK-F1-11`, `ai:generate` con `TSK-F1-08`, `export` y `public` con la Fase 1): su configuración queda fijada por un test, y se aplican cuando su endpoint exista.

### S2 — La respuesta del límite

Del contrato, `docs/api-contract.md` §1.6, **verbatim**:

> Respuesta: `429` con `Retry-After: <segundos>` y `X-RateLimit-Remaining`.

El código de error es `RATE_LIMITED`, del catálogo de `docs/api-contract.md` §1.5.

### S3 — `users.last_magic_link_at` en el grupo `auth:link`

Del criterio de aceptación de `TSK-F0-07`, **verbatim**:

> `users.last_magic_link_at` se usa para el grupo `auth:link`.

Y del modelo de datos, `docs/data-model.md` §3.18, **verbatim**:

> `last_magic_link_at` | `timestamptz` | Marca la última emisión de un magic link.

### S4 — Redis, no la memoria del proceso

Del criterio de aceptación de `TSK-F0-07`, **verbatim**:

> El rate limit vive en Redis, no en la memoria del proceso.

### S5 — El límite por IP y por email

Del criterio de aceptación de `TSK-F0-07`, **verbatim**:

> Los seis grupos de §1.6 funcionan, con el límite por IP y por email donde corresponde.

### S6 — El vector que se cierra

Del contexto de `TSK-F0-07`, **verbatim**:

> No hay ninguno implementado: sin ellos, el registro por magic link es un vector de spam de correo y de enumeración de emails.

### S7 — La regla de no doblar

De `AGENTS.md` §5.2, **verbatim**:

> **`business_id` es derivado del token de sesión por el middleware e inyectado en el contexto del handler.**

Y de `AGENTS.md` §5.1, **verbatim**:

> **`domain/` y `ports/` no deben importar `iii_sdk` ni `sqlx`.**

---

## Tasks

| ID | Qué | Specs | Ruta | Commit |
|---|---|---|---|---|
| T1 | El dominio del rate limit: los siete grupos, los sujetos, el formato de la clave y la decisión | S1, S5 | Inline | C1 |
| T2 | El puerto y el adaptador de Redis: ventana fija atómica | S4 | Inline | C2 |
| T3 | La capa HTTP de identidad: la IP del cliente y el `429` con sus cabeceras | S2 | Inline | C4 |
| T4 | La identidad: `auth` y `auth:link` sobre el magic link, con `last_magic_link_at` y enfriamiento durable | S3, S6, S7 | Inline | C3 + C4 |
| T5 | La documentación, el CI y la verificación | S1–S7 | Inline | C5 + C6 |

**Ruta:** todo inline. No hay razón de escritor delegado: son cinco unidades de comportamiento con superficies de edición disjuntas y de menos de un hundred de líneas cada una, y el trabajo de leer el contexto es el mismo que el de escribirlo.

---

## Log

### L1 — 2026-10-22 — El pedido

> TSK-F0-07 arranquemos

### L2 — 2026-10-22 — Dónde vive el rate limiter

El `api` es la fontanería compartida y **ya es una dependencia de la identidad**: `suertudo-identity` depende de `suertudo-api` porque el `AuthContext` vive ahí (el `Cargo.toml` de la identidad lo dice). El rate limit es del mismo tipo de cosa: lo usan dos workers y no puede estar duplicado.

**Decisión:** el rate limit vive en el crate `api`, con la forma que exige `AGENTS.md` §5.1 (`domain/` y `ports.rs` sin `iii_sdk` ni `sqlx`). No es un crate nuevo: la convención del repo ya está establecida.

### L3 — 2026-10-22 — La IP del cliente, y por qué la última

El payload de un trigger HTTP de iii (`HttpCallRequest` del SDK 0.24.6) lleva `query_params`, `path_params`, `headers`, `path`, `method` y `body`. **No lleva la dirección del peer.** La única fuente de la IP es una cabecera, y una cabecera la puede mandar el cliente.

**Decisión:** la IP es la **última** entrada de `X-Forwarded-For`, no la primera. El borde (el reverse proxy) la **agrega** al final (`$proxy_add_x_forwarded_for`), así que lo que hay antes es del cliente y lo que hay después es del borde. Tomar la primera sería tomar lo que el cliente mandó.

La confianza en esa cabecera es **estructural**, no una configuración: `ADR-012` dice que el motor iii no está expuesto a internet y que la API es el único camino. Cuando no hay borde (desarrollo, smoke test) no hay cabecera, no hay IP, y **la dimensión por IP no se aplica** — lo que queda es la dimensión por email, que es la que cierra el vector de abuso.

### L4 — 2026-10-22 — "Seis grupos" son siete

El criterio de `TSK-F0-07` dice "los seis grupos de §1.6" y la tabla de §1.6 tiene **siete**. El contrato es la verdad del comportamiento y la tarea es la que cuenta mal.

**Decisión:** se implementan los **siete** grupos de §1.6. La discrepancia queda anotada en el roadmap.

### L5 — 2026-10-22 — El algoritmo: ventana fija

La ventana fija es la lectura literal de "10 en 5 minutos", y es la que se puede garantizar con un `INCR` y un `EXPIRE` atómicos en Redis. La ventana deslizante daría un límite más estricto en el borde de la ventana, a costa de un conjunto ordenado por consulta en cada request.

**Decisión:** ventana fija, con un script de Lua que hace el `INCR` y el `EXPIRE` **en la misma operación atómica**. Sin el script, dos requests que llegan al mismo tiempo pueden dejar el contador sin TTL, y un contador sin TTL es un contador eterno.

### L6 — 2026-10-22 — `last_magic_link_at` y el grupo `auth:link`

`last_magic_link_at` marca la última emisión de un magic link. Con **un solo timestamp** no se puede contar "5 en 15 minutos": eso necesita un contador, y el contador vive en Redis (S4).

Lo que el timestamp sí puede dar, y lo que le da sentido a la columna, es la **separación mínima** entre dos emisiones a la misma dirección: el grupo permite 5 en 15 minutos, así que dos enlaces no pueden salir al mismo tiempo.

**Decisión:** el grupo `auth:link` se aplica con **las dos cosas**, que componen:
- el **contador de Redis**: 5 en 15 minutos por email — el número del contrato;
- el **registro durable** `users.last_magic_link_at`: la separación mínima entre dos emisiones a la misma dirección, que es `ventana / límite` (15 min / 5 = 3 min) y se deriva de la configuración del grupo, no es un número mágico.

El contador solo permitiría una ráfaga de cinco correos en el mismo segundo. El registro durable la impide, y además sobrevive a un vaciado de Redis.

### L7 — 2026-10-22 — La respuesta y las cabeceras

El `429` lleva `Retry-After` (los segundos que faltan) y `X-RateLimit-Remaining` (lo que queda). El `Retry-After` sale del TTL que Redis ya tiene en el contador: no hace falta calcularlo.

El `Retry-After` tiene que ser **al menos 1**: un `Retry-After: 0` le dice al cliente "reintentá ya" y produce una ráfaga.

### L8 — 2026-10-22 — Lo que la revisión tiene que mirar

El cambio toca la autenticación (el endpoint del magic link) y el contrato de la respuesta HTTP. Eso son los ítems de riesgo **2** (seguridad) y **3** (contratos que otros consumen) de la lista de riesgo alto. La verificación independiente es obligatoria antes de cerrar la tarea.

**Lo que la verificación tiene que comprobar, en particular:**
- que el límite por email del grupo `auth` no se puede eludir cambiando el formato del email (el `citext` del esquema no normaliza mayúsculas y minúsculas a nivel de SQL, así que la clave del rate limit tiene que usar la **misma** normalización que la del login);
- que el grupo `auth:link` no se puede eludir con un email distinto cada vez (para eso está la dimensión por IP);
- que un error de Redis **no** se traduce a "permitido" (fail closed).

### L9 — 2026-10-25 — El 429 real y el enfriamiento, probados e2e

La prueba end-to-end quedó en `services/identity/tests/identity_http.rs`:

- `the_request_after_the_limit_of_the_address_gets_the_429`: once pedidos con el mismo `X-Forwarded-For` y diez emails distintos — el onceavo recibe `429 RATE_LIMITED` con `Retry-After` dentro de la ventana de 5 min y `X-RateLimit-Remaining: 0`.
- `the_second_link_of_an_address_waits_for_its_cooling`: el enfriamiento dura solo para una dirección **registrada** (sin fila en `users`, `touch_last_magic_link` no tiene nada que marcar — es lo acordado en L6). El test hace: link → verify (crea el usuario) → segundo link (202, y deja la marca) → tercer link (429 con `retry-after: 180`, la separación es la ventana sobre el límite).

Verificación completa en verde: **23 suites, 165 tests** (16 del dominio + 6 del adaptador + 37 de identity_http ahora con estos dos), `cargo fmt --check` y `cargo clippy --workspace --all-targets -- -D warnings` limpios.

### L10 — 2026-10-25 — El test del sorteo se contaminaba a sí mismo (preexistente)

`the_worker_only_takes_the_raffles_that_are_due` fallaba **en la base** (`b0560d9`), sin este cambio. Causa: cada corrida del test deja atrás su propio raffle vencido sin sortear (el test nunca lo sortea), y con `ORDER BY draws_at LIMIT 100`, al acumular más de 100 la raffle más nueva —la del test— queda cortada de la ventana. Confirmado con probes SQL: 145 raffles vencidas sin draw en `suertudo_test`. El test conecta como dueño del esquema, así que la RLS no le aplica: era datos, no permisos. Arreglo mínimo: la raffle del test es la más vieja de la cola (`draws_at = now() - interval '365 days'`), que es exactamente el orden en que el worker sortea. La filosofía es la de `AGENTS.md` §9: un test que solo funciona con la tabla limpia es un test que no funciona. Filas de probe borradas a mano.

### L11 — 2026-10-25 — Config, CI y el arranque local

- `REDIS_URL` es **obligatoria** en la config de identidad: los ocho tests de config de `domain_rules.rs` la llevan en sus mapas de variables, y el `Deps` de los tests conecta con el Redis real (los tests e2e usan IP aleatoria por corrida, porque los contadores de Redis sobreviven la suite).
- CI (`ci.yml`): job `rust` con servicio `redis:7-alpine` (health `redis-cli ping`) y `REDIS_URL` al nivel del workflow, junto a `DATABASE_URL_TEST`.
- `scripts/dev.sh`: exporta `REDIS_URL` con default local antes de levantar el compose — el worker de identidad no arranca sin el store y el compose hereda el env del shell.
- El smoke (`smoke.yml`, spike) no toca a identity: sin Redis ahí, no falta nada.

### L12 — 2026-10-25 — Documentación de cierre

`docs/api-contract.md` §1.6 con **cómo está implementado** (store en Redis, ventana fija, store caído ⇒ 500, dónde aplica hoy, el enfriamiento durable); `docs/data-model.md` §3.18 con el uso real de `last_magic_link_at`; `docs/roadmap.md` con la tarea **completa** (los cuatro criterios marcados y la discrepancia "seis → siete" anotada en el propio criterio); `docs/architecture.md` §8.3 con `REDIS_URL` como obligatoria; `AGENTS.md` §9 con la trampa `text` vs `citext` (nueva, confirmada con probes).
