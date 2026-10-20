# Roadmap ejecutable — Suertudo

> Backlog del producto convertido en **unidades de trabajo que un agente de IA puede ejecutar sin reinterpretar la decisión de producto**. Cada tarea tiene contexto, archivos, dependencias y criterios de aceptación verificables.
>
> Los requisitos de donde salen estas tareas están en [`PRD.md`](./PRD.md). Los contratos, en [`api-contract.md`](./api-contract.md).

---

## 1. Cómo usar este documento

### 1.1 Regla de oro

> **Una tarea = un PR = un work unit.** Código, tests y documentación del comportamiento van juntos en el mismo commit. Un PR que cambia comportamiento sin actualizar el contrato o los docs está incompleto.

Esto sigue las convenciones de `work-unit-commits`: un commit es una unidad de trabajo revisable, no un fragmento de un trabajo más grande.

### 1.2 Antes de arrancar una tarea

Todo agente que ejecute una tarea de este roadmap debe:

1. Leer el `AGENTS.md` del repo (convenciones, comandos, estilo).
2. Leer el PRD y el contrato de API **de la sección que toca**. No todo el documento, la sección.
3. Verificar que las dependencias están mergeadas.
4. Escribir los tests **antes** del código cuando la tarea lo pida (la mayoría).

### 1.3 Definición de terminado

Una tarea está terminada cuando **todas** estas cosas son ciertas:

- [ ] Los criterios de aceptación de la tarea están cumplidos y verificados.
- [ ] Los tests nuevos pasan y los viejos no rompieron.
- [ ] `cargo fmt`, `cargo clippy -D warnings`, `flutter analyze` y `npm run lint` pasan limpios.
- [ ] Si la tarea cambia un contrato de API, el OpenAPI está regenerado y el diff está en el PR.
- [ ] Si la tarea cambia el esquema, hay migración nueva (nunca se edita una migración ya mergeada).
- [ ] La documentación afectada está actualizada en el mismo PR.
- [ ] El PR describe **qué** cambió y **por qué**, en el cuerpo, no solo en el título.

### 1.4 Tamaño de PR

| Tamaño | Acción |
|---|---|
| < 400 líneas | PR directo. |
| 400–800 líneas | PR directo, pero con descripción más detallada. |
| > 800 líneas | **Partir en PRs encadenados.** El segundo PR se apila sobre el primero y cada uno es revisable por separado. |

Los PRs encadenados se apilan así: `PR-1` (base) → `PR-2` (apila sobre `PR-1`) → `PR-3`. Cada uno se mergea en orden. La convención de nombres es `feat(raffles): <descripción> [1/3]`.

### 1.5 Etiquetas de tipo de tarea

| Etiqueta | Significado |
|---|---|
| `feat` | Funcionalidad nueva |
| `fix` | Corrección de bug |
| `refactor` | Cambio sin cambio de comportamiento |
| `chore` | Tooling, CI, configuración |
| `docs` | Solo documentación |
| `test` | Solo tests (raro: los tests van con el código) |

---

## 2. Fase 0 — Cimientos

**Objetivo:** un monorepo que compila, un motor iii que levanta, un esquema de base de datos, un usuario que se registra y entra, y un CI que pasa.

**Duración estimada:** 1 sprint (2 semanas).

**Criterio de salida:** `GET /v1/health` responde 200, un usuario puede registrarse por magic link y obtener un token, y el CI está verde en los tres stacks.

> **Estado:** el spike `spikes/iii-health` ya validó el engine, los workers del registro (`http`, `queue`, `cron`), un worker Rust propio, la ruta HTTP completa, **la ruta de colas y cron completa** y **los dos transportes de cola**, **sin KVM**. También fijó las versiones de iii (engine 0.24.6 + SDK 0.24.6), decidió el transporte de producción (`builtin`, `ADR-013`) y entregó el golden test de la derivación commit–reveal. Ver [`spike-iii-health.md`](./spike-iii-health.md) para el porqué, la mecánica y los hallazgos.

---

### `TSK-F0-01` — Esqueleto del monorepo

**Épica:** E0.1 · **Depende de:** — · **Esfuerzo:** 0,5 d

**Contexto:** el repo tiene solo `idea.md` y `docs/`. Hay que crear la estructura de carpetas, los manifests de los tres stacks y los archivos de convención.

**Archivos:**
```
Cargo.toml                 # workspace de Rust
rust-toolchain.toml        # channel stable
rustfmt.toml, clippy.toml
apps/mobile/               # flutter create --org app.suertudo
apps/business-web/         # create-next-app
packages/api-client/       # generado desde OpenAPI (placeholder por ahora)
infra/iii/config.yaml
infra/docker-compose.yml
AGENTS.md
README.md
.gitignore
```

**Criterios de aceptación:**
- [ ] `cargo build --workspace` compila.
- [ ] `flutter analyze` no reporta nada.
- [ ] `npm run build` de Next.js funciona.
- [ ] `AGENTS.md` documenta: estructura de carpetas, comandos de build/test/lint de cada stack, convención de commits, convención de nombres de función iii, y la regla de que las migraciones nunca se editan.
- [ ] `.gitignore` cubre `target/`, `.dart_tool/`, `node_modules/`, `.env`, `*.sqlx` (el cache de sqlx es generado, no commiteado, salvo que se use `SQLX_OFFLINE` en CI).
- [ ] `docker compose up` levanta Postgres, RabbitMQ, Redis, MinIO y Mailpit.

**Verificación:** `docker compose up -d && cargo build && npm --prefix apps/business-web run build`

---

### `TSK-F0-02` — Motor iii y worker mínimo

**Épica:** E0.2 · **Depende de:** `TSK-F0-01` · **Esfuerzo:** 0,5 d

**Contexto:** probar que el motor levanta en este entorno **sin KVM** (riesgo `R2`) y que un worker Rust se registra y responde.

**Archivos:** `services/api/` (crate nuevo), `infra/iii/config.yaml`

**Criterios de aceptación:**
- [ ] `iii --config infra/iii/config.yaml` levanta y escucha en `ws://127.0.0.1:49134`.
- [ ] El worker `api` se registra y expone `GET /v1/health` → `200 {"status":"ok"}`.
- [ ] El worker se conecta con `register_worker` + `InitOptions` y registra un trigger `http`.
- [ ] El `iii.worker.yaml` del worker existe pero **no es necesario** para que funcione (documentado en el README del crate).
- [ ] Hay un test que verifica que el motor levanta sin `/dev/kvm` accesible.

**Verificación:** el smoke test del CI (§2.5) es la prueba de esta tarea.

---

### `TSK-F0-03` — Esquema base y migraciones

**Épica:** E0.3 · **Depende de:** `TSK-F0-01` · **Esfuerzo:** 1,5 d

**Contexto:** crear el esquema completo de [`data-model.md`](./data-model.md) en migraciones. **Es la base de todo lo demás**; conviene hacerlo entero ahora y no incrementalmente, porque cambiar el esquema después es más caro.

**Archivos:** `services/db/migrations/0001..0012_*.sql`, `services/db/`

**Criterios de aceptación:**
- [ ] Las 16 tablas de §3 de `data-model.md` existen con sus constraints e índices.
- [ ] El trigger `tickets_business_match_trg` está activo.
- [ ] Las reglas append-only de `audit_log` están activas.
- [ ] `UNIQUE (raffle_id, number)` en `ticket_numbers` está activa.
- [ ] El índice parcial `tickets_user_raffle_limit_idx` está activo.
- [ ] `cargo sqlx prepare` funciona y el cache se regenera.
- [ ] Hay un test que verifica que un ticket con `business_id` distinto al del sorteo falla.

**Verificación:** `sqlx migrate run && cargo test -p suertudo-db`

---

### `TSK-F0-04` — Identidad: registro, magic link, sesión

**Épica:** E0.4 · **Depende de:** `TSK-F0-02`, `TSK-F0-03` · **Esfuerzo:** 2 d

**Contexto:** el primer vertical de la API. Es también el que define el patrón de todos los handlers.

**Archivos:** `services/identity/`, `services/api/src/handlers/auth.rs`

**Criterios de aceptación:**
- [ ] `POST /v1/auth/magic-link` responde `202` siempre, exista o no el email.
- [ ] El enlace expira en 15 minutos y es de un solo uso (`400 AUTH_LINK_CONSUMED` al segundo uso).
- [ ] `POST /v1/auth/magic-link/verify` devuelve access token (15 min) y refresh token (30 días).
- [ ] `POST /v1/auth/refresh` rota el refresh token; usar uno ya rotado invalida la familia completa.
- [ ] El access token es un JWT firmado HS256 con `sub`, `role`, `exp`, `iat`, `jti`.
- [ ] El middleware `identity::authorize` está registrado en el puerto RBAC de iii e inyecta el contexto de sesión.
- [ ] En `ENV=development`, la respuesta de magic link incluye `dev_magic_link`. En staging y producción, **el arranque falla** si esa opción está activa.
- [ ] Test: un token expirado da `401 AUTH_EXPIRED`.

**Verificación:** `cargo test -p suertudo-identity` + test de contrato HTTP.

---

### `TSK-F0-05` — Pipeline de CI

**Épica:** E0.5 · **Depende de:** `TSK-F0-01` · **Esfuerzo:** 1 d

**Contexto:** el pipeline tiene que ser la red de seguridad de todo lo que viene. Un PR que rompe el build no llega a main.

**Archivos:** `.github/workflows/ci.yml`, `.github/workflows/smoke.yml`

**Criterios de aceptación:**
- [ ] Job de Rust: `fmt --check`, `clippy -D warnings`, `test --workspace`, `sqlx prepare --check`, `cargo audit`.
- [ ] Job de Flutter: `analyze`, `test`, `format --set-exit-if-changed`.
- [ ] Job de Next.js: `lint`, `typecheck`, `test`, `build`.
- [ ] Job de contrato: compara el OpenAPI generado contra el commiteado y falla si hay cambios no intencionales.
- [ ] Smoke test: levanta el motor iii, registra un worker, invoca una función por HTTP y verifica la respuesta.
- [ ] El job de evals de IA corre solo si el PR toca `services/ai-branding/`.

---

### `TSK-F0-06` — Cola y cron de iii (path sin verificar)

**Épica:** E0.6 · **Depende de:** `TSK-F0-02` · **Esfuerzo:** 1 d · **Estado: COMPLETADA**

**Contexto:** el spike `spikes/iii-health` verificó el engine, los triggers `http` y el registro de funciones, pero **no** la ruta de colas. El engine registra un warning explícito: `trigger type durable:subscriber is not registered ... If this persists, the queue worker is missing`. `draws::execute_draw` y `notifications::dispatch` dependen de esa ruta, así que hay que probarla antes de construir encima.

**Archivos:** `spikes/iii-health/verify.sh` (extender), `spikes/iii-health/src/queue_probe.rs` (nuevo), `docs/api-contract.md` §7.3

**Criterios de aceptación:**
- [x] El worker `queue` del registro alcanza estado `ready` y provee `durable:subscriber` y `engine::queue::*`.
- [x] Un mensaje publicado con `iii::durable::publish` llega a su suscriptor.
- [x] Un mensaje encolado con `TriggerAction::Enqueue { queue }` es consumido por la función destino.
- [x] Un consumidor que falla reintenta 3 veces con backoff y después va a la DLQ.
- [x] `engine::queue::dlq_topics` y `engine::queue::dlq_messages` muestran el mensaje muerto.
- [x] `iii::queue::redrive` reprocesa el mensaje; `iii::queue::discard_message` lo descarta.
- [x] Un trigger `cron` se dispara efectivamente (contador de ticks).
- [x] Todo el stack sigue levantando **sin KVM**.
- [x] `verify.sh` queda como el smoke test del CI, con teardown por nombre de proceso exacto.
- [x] El adaptador `rabbitmq` se verificó contra un broker real y dio lugar a `ADR-013`.

**Verificación:** `unshare -rm ./spikes/iii-health/verify.sh` → `RESULTADO: PASS` (13 secciones)

**Hallazgos que cambiaron los docs:**
- **El engine no incluye ningún tipo de trigger de cola ni de cron.** Cada uno necesita su worker del registro: `http`, `queue` y `cron`. Sin ellos, el trigger queda `PENDING` para siempre.
- **`TriggerAction::Enqueue` sí funciona en el engine 0.24.6** (el corte `QueueEnqueuer` está presente). La cola tiene que definirse antes con `queue::define`, y su schema es estricto: `type` solo admite `standard` o `fifo`, `fifo` exige `message_group_field`, y `function_id` **no** es parte de la config (viaja con el mensaje).
- **El worker `cron` del registro tiene el mismo nombre de proceso que el cron del sistema** (`/usr/sbin/cron`). Un `pkill -x cron` mata el scheduler del SO. El teardown matchea por ruta de ejecutable.
- **RabbitMQ queda como decisión de transporte pendiente**: el adaptador `builtin` (default) cubre reintentos, DLQ y redrive sin broker externo, pero el adaptador `rabbitmq` no está verificado en este entorno.

**Pendiente:** cerrado. La decisión de transporte quedó tomada en [`ADR-013`](./architecture.md): **`builtin`**, sin broker externo, porque en RabbitMQ la API de DLQ de iii no puede direccionar la DLQ por función y los mensajes muertos quedan inaccesibles. Los dos transportes están verificados y el script acepta `QUEUE_ADAPTER=builtin|rabbitmq`.

---

**Objetivo:** el bucle completo funcionando. Un negocio crea un sorteo con IA, un participante reserva números, el sistema sortea de forma verificable, el ganador se entera y el negocio ve su lista.

**Duración estimada:** 3–4 sprints.

**Criterio de salida:** los 7 ítems de §4.3 del PRD.

---

### Épica E1.1 — Onboarding de negocios

#### `TSK-F1-01` — Modelo de negocio y miembros

**Depende de:** `TSK-F0-04` · **Esfuerzo:** 1,5 d

**Criterios de aceptación:**
- [ ] `POST /v1/businesses` crea negocio + membresía `owner` en una sola transacción.
- [ ] `GET/PATCH /v1/businesses/me` expone el negocio con sus tokens de marca y sus límites de plan.
- [ ] `GET/POST/PATCH/DELETE /v1/businesses/me/members` gestionan miembros con roles `admin`, `staff`, `analyst`.
- [ ] Un `analyst` que intenta `POST /v1/businesses/me/raffles` recibe `403 RBAC_FORBIDDEN`.
- [ ] El `owner` no puede ser removido ni degradado.
- [ ] `business_id` se deriva **siempre** del token. Un `business_id` en el body se ignora silenciosamente.

#### `TSK-F1-02` — Planes y límites

**Depende de:** `TSK-F1-01` · **Esfuerzo:** 0,5 d

**Criterios de aceptación:**
- [ ] `businesses::check_plan_limit` expone los límites del plan (`starter`, `pro`, `business`).
- [ ] Exceder sorteos activos devuelve `402 PLAN_LIMIT_REACHED` con `details.limit` y `details.used`.
- [ ] Exceder participantes del mes devuelve `402` con el mismo formato.
- [ ] Los contadores de uso se actualizan de forma atómica con la operación que los consume.

---

### Épica E1.2 — Sorteos

#### `TSK-F1-03` — CRUD de sorteos y máquina de estados

**Depende de:** `TSK-F1-01` · **Esfuerzo:** 2 d

**Criterios de aceptación:**
- [ ] `POST/GET/PATCH /v1/businesses/me/raffles[/{id}]` implementados con el contrato de §5.3 de `api-contract.md`.
- [ ] La máquina de estados de §4.1 de `data-model.md` está implementada como código de dominio, con tests de **todas** las transiciones válidas e inválidas.
- [ ] Una transición inválida devuelve `409 INVALID_STATE_TRANSITION` con `details.valid_transitions`.
- [ ] `PATCH` en un estado no editable devuelve `409` con `details.editable_fields`.
- [ ] El dominio no importa `iii_sdk` (verificado por un test de arquitectura con `grep` o `cargo-lint`).
- [ ] Los conteos `tickets_count` y `numbers_reserved` se mantienen dentro de la transacción que los modifica.

#### `TSK-F1-04` — Publicación con compromiso criptográfico

**Depende de:** `TSK-F1-03` · **Esfuerzo:** 1,5 d

**Contexto:** es el corazón de la confianza. Ver `ADR-008` y §7.2 del PRD.

**Criterios de aceptación:**
- [ ] `publish` genera una semilla de 32 bytes con CSPRNG (`getrandom`), calcula `SHA-256(seed)` y lo publica.
- [ ] La semilla se guarda **cifrada** con AES-256-GCM. Un test inspecciona la fila y verifica que el valor no es el binario en claro.
- [ ] El `commitment` es inmutable: un intento de modificarlo devuelve `409 IMMUTABLE_FIELD`.
- [ ] El `commitment` es visible en el detalle público del sorteo.
- [ ] Publicar un sorteo que ya estaba publicado es un no-op idempotente.
- [ ] Un test verifica que el `commitment` es igual a `hex(sha256(seed_descifrado))` (el test conoce la clave).

#### `TSK-F1-05` — Cierre automático y job de cron

**Depende de:** `TSK-F1-03` · **Esfuerzo:** 1 d

**Criterios de aceptación:**
- [ ] `draws::close_due_raffles` se registra con trigger `cron` de 1 minuto.
- [ ] Cierra los sorteos `published` con `closes_at` vencido, dentro de una ventana de 5 min.
- [ ] Un ticket creado después del cierre devuelve `409 RAFFLE_CLOSED`.
- [ ] Al cerrar se libera la reserva de los tickets cancelados y se actualizan los contadores.
- [ ] Si la cobertura es menor al umbral, se genera un aviso al comercio (no bloquea el sorteo).
- [ ] El job es idempotente: correrlo dos veces no cambia el resultado.

---

### Épica E1.3 — Motor de tema

#### `TSK-F1-06` — `theme_spec` v1: schema, validador, derivación

**Depende de:** `TSK-F0-03` · **Esfuerzo:** 2 d

**Contexto:** es la pieza que hace que la IA sea barata de agregar. El schema, el validador y el render existen **antes** de que exista la llamada al modelo.

**Archivos:** `services/ai-branding/src/schema.rs`, `src/validate.rs`, `src/contrast.rs`, `packages/theme-spec/v1.schema.json`

**Criterios de aceptación:**
- [ ] El JSON Schema de `ai-spec.md` §5 está en `packages/theme-spec/v1.schema.json` y se valida en runtime.
- [ ] `additionalProperties: false` en todos los niveles. Un campo extra falla.
- [ ] La validación de anclas de marca (§6.1) rechaza la generación si cambió un token de marca.
- [ ] La validación de contraste (§6.2) implementa el algoritmo de luminancia relativa y WCAG tal cual está escrito, y corrige automáticamente `text` y `text_muted`; si no puede corregir, rechaza.
- [ ] Las reglas de copy (§6.3) aplican límites de longitud, quitan emojis, ajustan mayúsculas y descartan claims engañosos.
- [ ] Los tokens derivados (`primary_soft`, `gradient_from`, `gradient_to`) los calcula la plataforma, no el modelo.
- [ ] Hay tests con los pares de contraste conocidos (blanco/negro = 21:1; gris medio = falla) y con casos de ancla violada.
- [ ] El tema por defecto (§7 de `ai-spec.md`) está versionado en el repo con sus placeholders.

#### `TSK-F1-07` — Render del tema en Flutter y Next.js

**Depende de:** `TSK-F1-06` · **Esfuerzo:** 2 d

**Contexto:** **el mismo `theme_spec` tiene que verse igual en los dos clientes.** Esta es la parte que más se desincroniza si no se ataca con disciplina.

**Criterios de aceptación:**
- [ ] `apps/mobile/lib/core/theme/` traduce el `theme_spec` a `ThemeData` de Flutter.
- [ ] `apps/business-web/src/theme/` traduce el `theme_spec` a variables CSS.
- [ ] Ambos renderizadores implementan **las mismas** reglas de derivación que el backend.
- [ ] Hay un **golden test** con 5 `theme_spec` fijos: el mismo input produce el mismo output en ambos renderizadores (se comparan los tokens derivados, no los píxeles).
- [ ] Las 5 variantes de `layout` se renderizan sin overflow ni texto cortado (test de widget en Flutter, test de componente en Next.js).
- [ ] Un `theme_spec` inválido que llega al cliente (por un bug) no rompe la app: se cae al tema por defecto con un log.

---

### Épica E1.4 — Generación con IA

#### `TSK-F1-08` — Cliente de LLM y pipeline de generación

**Depende de:** `TSK-F1-06` · **Esfuerzo:** 2 d

**Criterios de aceptación:**
- [ ] `ai::generate_theme` está registrada y devuelve `202` con `generation_id`.
- [ ] El prompt del sistema es `theme-system/v3`, versionado en `services/ai-branding/prompts/`.
- [ ] El payload enviado al modelo contiene **solo** `prompt`, `brand_tokens` y `raffle_context` (allow-list explícita; test que verifica que no sale ningún otro campo).
- [ ] La respuesta se valida con el pipeline completo de §5 de `ai-spec.md`.
- [ ] Un fallo de schema reintenta **una** vez con el error de validación en el prompt.
- [ ] Un segundo fallo, o un timeout, aplica el tema por defecto con `fallback_applied: true`.
- [ ] Timeout duro de 15 s. Un modelo de respaldo se usa si el principal falla dos veces.
- [ ] `ai_generations` registra: modelo, tokens, costo, latencia, errores de validación.
- [ ] Test con un mock del modelo: respuesta inválida → reintento → válido; dos respuestas inválidas → fallback.

#### `TSK-F1-09` — Cache, versioning y overrides

**Depende de:** `TSK-F1-08` · **Esfuerzo:** 1,5 d

**Criterios de aceptación:**
- [ ] La cache key es `sha256(prompt_normalizado + brand + contexto + previous_spec + modelo + versión de prompt)`.
- [ ] Un hit de cache devuelve la versión existente sin llamar al modelo (verificable por el contador de llamadas).
- [ ] Cada generación exitosa crea un `theme_version` con `version` incremental.
- [ ] Solo hay un tema activo por sorteo (garantizado por el índice parcial, no por la aplicación).
- [ ] `POST /theme/versions/{v}/activate` hace rollback y queda auditado.
- [ ] `PATCH /theme` guarda overrides como nueva versión `source = 'manual'`.
- [ ] Un override manual sobrevive a una regeneración posterior.

---

### Épica E1.5 — Feed y detalle (app móvil)

#### `TSK-F1-10` — Feed público

**Depende de:** `TSK-F1-03` · **Esfuerzo:** 1,5 d

**Criterios de aceptación:**
- [ ] `GET /v1/public/raffles` con filtros y paginación por cursor.
- [ ] La app muestra el feed con la card del sorteo usando el `theme_spec`.
- [ ] Un sorteo sin tema usa el tema por defecto sin crackear.
- [ ] El feed se cachea localmente y muestra "actualizado hace X" cuando está offline.
- [ ] La navegación a un deep link `suertudo://raffle/{id}` abre el detalle correcto.

#### `TSK-F1-11` — Detalle de sorteo y motor de reserva

**Depende de:** `TSK-F1-10` · **Esfuerzo:** 2 d

**Criterios de aceptación:**
- [ ] El detalle muestra premio, reglas, fecha de cierre, números disponibles, compromiso y tema.
- [ ] El selector de números valida en cliente: cantidad en rango, números distintos, dentro del pool, y muestra cuáles ya están tomados (vía `GET /public/raffles/{id}/numbers/{n}`).
- [ ] La reserva manual y la automática (`auto_pick: true`) funcionan.
- [ ] La reserva requiere conexión: offline, el botón está deshabilitado con un mensaje claro.
- [ ] Un `409 NUMBER_TAKEN` devuelve al selector con los númerosconflictivos marcados.
- [ ] Al completar, muestra el código del ticket (`SU-XXXXXXX`) con opción de compartir.

---

### Épica E1.6 — Participación en el backend

#### `TSK-F1-12` — Reserva transaccional de números

**Depende de:** `TSK-F1-05` · **Esfuerzo:** 2,5 d

**Contexto:** la operación más importante del producto. Un bug acá significa números duplicados o tickets a medias.

**Criterios de aceptación:**
- [ ] `POST /v1/raffles/{public_id}/tickets` implementa la transacción de §6.2 de `data-model.md`.
- [ ] La reserva es **atómica**: si un número está tomado, no queda ticket creado.
- [ ] `SELECT ... FOR UPDATE` sobre la fila del sorteo serializa reservas concurrentes del mismo sorteo.
- [ ] La reserva automática elige N números libres distintos bajo el mismo lock.
- [ ] El límite de tickets por usuario lo garantiza el índice único parcial (test de concurrencia: 10 requests simultáneos con límite 1 → 1 ticket).
- [ ] El límite de números por ticket se valida contra `numbers_per_ticket_min/max`.
- [ ] La idempotencia con `Idempotency-Key` funciona: reenviar la misma key devuelve el mismo ticket.
- [ ] El `business_id` del ticket se copia del sorteo (y el trigger lo verifica).

#### `TSK-F1-13` — Cancelación, códigos de validación y anti-abuso

**Depende de:** `TSK-F1-12` · **Esfuerzo:** 1,5 d

**Criterios de aceptación:**
- [ ] `DELETE /v1/tickets/{id}` cancela mientras el sorteo esté `published` y libera los números.
- [ ] `POST /v1/businesses/me/validation-codes` genera códigos de un solo uso.
- [ ] En `participation_mode = validated`, la reserva sin código o con código usado falla con el código de error correcto.
- [ ] Rate limit por usuario y por IP en la creación de tickets.
- [ ] Bloqueo de emails desechables (lista configurable).
- [ ] Detección de altas en masa: 3 cuentas nuevas desde la misma IP en 10 minutos quedan en `pending_review` y no pueden reservar.

---

### Épica E1.7 — Sorteo

#### `TSK-F1-14` — Derivación y ejecución

**Depende de:** `TSK-F1-04`, `TSK-F1-05` · **Esfuerzo:** 2 d

**Contexto:** acá se juega la promesa central del producto.

**Criterios de aceptación:**
- [ ] `draws::execute_draw` corre por cron cada minuto y busca sorteos `closed` con `draws_at` vencido y sin `draw`.
- [ ] La inserción en `draws` con `ON CONFLICT (raffle_id) DO NOTHING` garantiza **exactamente un** draw. Test con 5 ejecuciones concurrentes → 1 fila.
- [ ] La derivación es exactamente `u64_be(HMAC-SHA256(seed, "suertudo/v1|raffle:{id}|draw:{i}")[0..8]) mod pool_size`.
- [ ] Si el número no está reservado, se aplica la `no_winner_policy` (`redraw` incrementa el índice hasta 100 intentos; después `void`).
- [ ] La semilla se revela (`seed_revealed = true`) y queda pública de forma permanente.
- [ ] Se crean los `prizes` correspondientes, con `unassigned_reason` si sobran premios.
- [ ] Los tickets ganadores pasan a `winner`; el resto a `not_winner`.
- [ ] El outbox escribe el evento de notificación **en la misma transacción** que confirma el draw.
- [ ] **Test golden:** con una semilla y un `raffle_id` fijos, el número derivado es un valor fijo hardcodeado en el test. Si alguien cambia la fórmula, el test falla.

#### `TSK-F1-15` — Verificación pública

**Depende de:** `TSK-F1-14` · **Esfuerzo:** 1 d

**Criterios de aceptación:**
- [ ] `GET /v1/public/raffles/{public_id}/verify` devuelve exactamente la forma de §2.4 del contrato.
- [ ] La respuesta incluye los `steps` en texto legible, con el resultado esperado.
- [ ] Un tercero puede recalcular el resultado con la respuesta. **Test end-to-end:** un script en Rust independiente (sin usar el código del worker) recalcula y compara.
- [ ] La respuesta es cacheable (el resultado es inmutable).
- [ ] Para un sorteo no sorteado, devuelve `seed_revealed: false` y **no** expone la semilla.

---

### Épica E1.8 — Notificaciones

#### `TSK-F1-16` — Worker de notificaciones

**Depende de:** `TSK-F1-04` · **Esfuerzo:** 2 d

**Criterios de aceptación:**
- [ ] `notifications::dispatch` está suscrito a la cola `notifications` con reintentos (3 intentos, backoff exponencial) y DLQ.
- [ ] Envío por push (FCM HTTP v1), email e in-app.
- [ ] La `dedupe_key` única en la base garantiza que un reintento no duplica la notificación.
- [ ] Un push sin token del usuario queda `skipped` y el email se envía igual.
- [ ] Un token que FCM marca como inválido se desactiva en `push_devices`.
- [ ] Los mensajes fallidos van a la DLQ y hay una herramienta para redrive.
- [ ] Las plantillas de email tienen versión (para poder cambiarlas sin romper a los que ya las recibieron).

#### `TSK-F1-17` — Eventos de notificación del ciclo de vida

**Depende de:** `TSK-F1-16` · **Esfuerzo:** 1,5 d

**Criterios de aceptación:**
- [ ] Eventos implementados: `ticket_confirmed`, `raffle_closing_soon` (24 h antes), `raffle_voided`, `you_won`, `prize_claimed`, `prize_delivered`, `prize_expired`.
- [ ] El recordatorio de cierre solo se envía a quienes tienen ticket (`FR-NO-02`).
- [ ] La notificación de ganador llega dentro de los 60 s del sorteo (métrica con alerta).
- [ ] El participante puede desactivar notificaciones de marketing sin perder las operativas.

---

### Épica E1.9 — Panel del negocio

#### `TSK-F1-18` — Shell del panel y autenticación

**Depende de:** `TSK-F1-01` · **Esfuerzo:** 2 d

**Criterios de aceptación:**
- [ ] Layout del panel con navegación, y rutas privadas con middleware de Next.js.
- [ ] Login, logout y refresh funcionando con cookie `httpOnly` + access token en memoria.
- [ ] Un usuario sin membresía en el negocio recibe `404` (no `403`) al entrar al panel de ese negocio.
- [ ] Rutas de negocio: dashboard, sorteos, leads, settings.
- [ ] Skeleton de carga, estados vacíos y estados de error en todas las pantallas.

#### `TSK-F1-19` — Editor de sorteo con preview de tema

**Depende de:** `TSK-F1-18`, `TSK-F1-07` · **Esfuerzo:** 2,5 d

**Criterios de aceptación:**
- [ ] Formulario de creación/edición con validación Zod compartida con la API.
- [ ] Panel de generación IA: input de prompt, botón de generar, estado `generating`, y preview en vivo del resultado.
- [ ] El preview usa el **mismo** renderizador que la app móvil (mismo `theme_spec`).
- [ ] Editor manual: color picker por token, y tamaño de tipografía.
- [ ] Los overrides manuales se guardan y persisten.
- [ ] Historial de versiones con rollback.
- [ ] Si la IA falla, el panel muestra el tema por defecto con un aviso claro (no un error rojo).

#### `TSK-F1-20` — Listado, métricas y estado de premios

**Depende de:** `TSK-F1-18` · **Esfuerzo:** 2 d

**Criterios de aceptación:**
- [ ] Listado de sorteos con filtros por estado, búsqueda y orden.
- [ ] Métricas por sorteo: participantes, tickets, cobertura, premios pendientes, tiempo a entrega.
- [ ] Pantalla de premios con cambio de estado (pending → claimed → delivered) y notas de entrega.
- [ ] `PATCH /prizes/{id}` valida la transición; una transición inválida muestra el error exacto.

#### `TSK-F1-21` — Participantes, export y leads

**Depende de:** `TSK-F1-18`, `TSK-F1-12` · **Esfuerzo:** 2,5 d

**Criterios de aceptación:**
- [ ] Listado de participantes con paginación por cursor y virtualization.
- [ ] Los participantes sin consentimiento muestran "contacto no compartido" en vez de ocultarse.
- [ ] Export CSV: columnas de contacto vacías cuando no hay consentimiento. **Test automatizado.**
- [ ] El CSV incluye una línea de metadatos (sorteo, fecha, cantidad de filas).
- [ ] La exportación queda auditada con actor, hora y número de filas.
- [ ] Panel de leads: contactos consentidos, con último sorteo y total de participaciones.
- [ ] Un revoke de consentimiento saca el contacto del export siguiente. **Test automatizado.**

---

### Épica E1.10 — Privacidad, premios y cierre

#### `TSK-F1-22` — Consentimiento y auditoría

**Depende de:** `TSK-F1-12` · **Esfuerzo:** 1,5 d

**Criterios de aceptación:**
- [ ] La pantalla de reserva pide consentimiento granular (`share_marketing`, `share_contact`).
- [ ] Cada cambio de consentimiento escribe una fila en `consent_records` (append-only) con `source`, `ip_hash` y `user_agent`.
- [ ] `GET/PUT /v1/me/consents` funcionan.
- [ ] `audit::write` se invoca desde todas las acciones sensibles (publicar, cerrar, anular, generar tema, activar versión, exportar, cambiar estado de premio, suspender).
- [ ] La auditoría guarda el diff, no el objeto completo.
- [ ] `audit_log` es append-only (test que verifica que un UPDATE no cambia nada).

#### `TSK-F1-23` — Reclamo y entrega de premios (app móvil)

**Depende de:** `TSK-F1-16` · **Esfuerzo:** 1,5 d

**Criterios de aceptación:**
- [ ] "Mis premios" lista los premios con estado y muestra el `claim_code`.
- [ ] El ganador puede reclamar desde la app; el estado pasa a `claimed` y el comercio lo ve.
- [ ] El `claim_code` se puede compartir (copiar o QR) para canjear en el local.
- [ ] Los premios expiran a los 30 días (`expires_at`) y el job de expiración los marca `expired` y notifica a las dos partes.
- [ ] El commerce puede extender la fecha de expiración.

#### `TSK-F1-24` — Perfil, baja y portabilidad

**Depende de:** `TSK-F1-04` · **Esfuerzo:** 1 d

**Criterios de aceptación:**
- [ ] `GET/PATCH /v1/me` permite editar nombre, teléfono, locale y preferencias de notificación.
- [ ] `DELETE /v1/me` anonimiza PII, pone `tickets.user_id = NULL` y conserva los premios.
- [ ] `GET /v1/me/data` devuelve un JSON con todos los datos personales del usuario.

#### `TSK-F1-25` — Endurecimiento y cierre del MVP

**Depende de:** todas · **Esfuerzo:** 2 d

**Criterios de aceptación:**
- [ ] Test de aislamiento multi-tenant que recorre **todos** los endpoints de negocio con un token de otro business y verifica `404`.
- [ ] RLS activada y testeada: una sesión sin `app.business_id` no lee datos.
- [ ] Los 7 ítems de §4.3 del PRD están cumplidos y verificados.
- [ ] Carga de prueba: 10 000 sesiones concurrentes de lectura sostenidas durante 5 min sin errores.
- [ ] El flujo E2E completo (crear → publicar → reservar → sortear → notificar → reclamar → entregar) pasa en CI.
- [ ] Los paneles de observabilidad tienen las alertas de §10 de `architecture.md`.

---

## 4. Fase 2 — Crecimiento

**Objetivo:** que el producto crezca y que la suscripción se pague. Detalle por tarea al momento de entrar en la fase.

| # | Épica | Contenido | Depende de |
|---|---|---|---|
| `TSK-F2-01` | Página pública del sorteo | SSR, Open Graph, indexable, deep link desde WhatsApp | E1.9 |
| `TSK-F2-02` | Cobro de la suscripción | Integración de B2B (Mercado Pago / Stripe) + factura + estado de plan | E1.2 |
| `TSK-F2-03` | Mensajería por WhatsApp | Proveedor, plantillas, opt-in, entrega | E1.8 |
| `TSK-F2-04` | Tiempo real | WebSocket para cambios de sorteo, premios y feed | E1.5 |
| `TSK-F2-05` | Kit de difusión | Materiales para que el negocio difunda (imágenes, copy, QR) | E1.4 |
| `TSK-F2-06` | Imágenes por IA | Banner generado, moderación, revisión humana | E1.4 |
| `TSK-F2-07` | Validación en el local | Validación de compra por QR en mostrador (sin app de personal) | E1.6 |
| `TSK-F2-08` | Multi-negocio | Multi-negocio: un participante en muchos, un negocio en muchos | E1.6 |
| `TSK-F2-09` | KYC del negocio | Verificación de CUIT y documentos | E1.1 |
| `TSK-F2-10` | Observabilidad de negocio | Embudo de participación, atribución por canal | E1.9 |

**Criterio de salida de la fase:** un negocio activo pasa de `starter` a un plan pago y mantiene el uso en los meses siguientes.

---

## 5. Fase 3 — Expansión

Backlog abierto, priorizado por evidencia del uso real.

| # | Épica | Nota |
|---|---|---|
| `TSK-F3-01` | Adaptador de lotería externa | Interfaz ya preparada. Solo si hay un número oficial y un caso de negocio real (riesgo `R3`). |
| `TSK-F3-02` | Insights con IA | Sobre datos agregados del propio negocio, con el dato citado. |
| `TSK-F3-03` | CRM externo | Mailchimp / Braze, solo para negocios con consentimientos suficientes. |
| `TSK-F3-04` | Gamificación | Sellos, referidos, misiones. |
| `TSK-F3-05` | Multi-idioma | es-AR primero, luego pt-BR. |
| `TSK-F3-06` | App de mostrador | App para el personal del local, con validación de códigos. |
| `TSK-F3-07` | Sorteos en cadena | Un sorteo que da acceso a otro (multi-nivel). |
| `TSK-F3-08` | API pública para comercios | Webhooks y API documentada para integraciones. |

---

## 6. Resumen

| Fase | Épicas | Tareas detalladas | Esfuerzo | Riesgo principal |
|---|---|---|---|---|
| 0 — Cimientos | 7 | 7 | ~8,5 d | La reserva transaccional (`TSK-F1-12`) |
| 1 — MVP vertical | 10 | 25 | ~42 d | La reserva transaccional (`TSK-F1-12`) |
| 2 — Crecimiento | 10 | por detallar | ~30 d | Cobro de la suscripción |
| 3 — Expansión | 8 | por detallar | ~40 d+ | Dependencia de terceros (lotería, CRM) |

### 6.1 Secuencia crítica

Si solo hay tiempo para una cosa, el camino de menor resistencia al primer usuario real es:

```
TSK-F0-01 → TSK-F0-02 → TSK-F0-03 → TSK-F0-04
           → TSK-F1-01 → TSK-F1-03 → TSK-F1-04
           → TSK-F1-12 → TSK-F1-14 → TSK-F1-15
           → TSK-F1-16
```

Esa cadena produce el **bucle verificable completo**: negocio → sorteo → participant reserva → se sortea → se puede verificar → se notifica. Todo lo demás (IA, panel lindo, export) es valor alrededor de ese núcleo.

La IA de tema (`TSK-F1-06` → `TSK-F1-09`) es el segundo camino de menor resistencia al **diferenciador**, y es independiente del primero: se puede construir en paralelo sin bloquearlo.
