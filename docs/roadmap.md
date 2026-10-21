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

> **Estado:** completadas `TSK-F0-01` a `TSK-F0-06`. El spike `spikes/iii-health` validó el engine, los workers del registro (`http`, `queue`, `cron`), un worker Rust propio, la ruta HTTP completa, **la ruta de colas y cron completa** y **los dos transportes de cola**, **sin KVM**. También fijó las versiones de iii (engine 0.24.6 + SDK 0.24.6), decidió el transporte de producción (`builtin`, `ADR-013`) y entregó el golden test de la derivación commit–reveal. Ver [`spike-iii-health.md`](./spike-iii-health.md) para el porqué, la mecánica y los hallazgos.
>
> **Abiertas:** `TSK-F0-07` (rate limits), `TSK-F0-08` (envío real del magic link), `TSK-F0-09` (rol de aplicación y smoke test del backend), `TSK-F0-11` (proveedor de SMS, que bloquea el despliegue de producción).
>
> **Una función de la base falta todavía:** `claim_prize(p_prize_id, p_user_id, p_claim_code)`, sin la cual `FR-DR-08` y `POST /v1/me/prizes/{prize_id}/claim` no tienen camino. La especificación está en `docs/data-model.md` §3.27.

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

**Épica:** E0.3 · **Depende de:** `TSK-F0-01` · **Esfuerzo:** 1,5 d · **Estado: COMPLETADA**

**Contexto:** el esquema es el contrato de [`data-model.md`](./data-model.md) §3. La Fase 1 empieza por acá, así que el esquema tiene que existir entero y con sus invariantes **en la base**, no en checks de aplicación.

**Criterios de aceptación:**
- [x] Las 16 tablas de §3 existen con columnas, tipos, defaults y `CHECK`.
- [x] `UNIQUE (raffle_id, number)` en `ticket_numbers` rechaza una segunda reserva del mismo número.
- [x] `UNIQUE (raffle_id)` en `draws` rechaza un segundo sorteo del mismo sorteo.
- [x] El índice parcial único de `tickets (raffle_id, user_id)` rechaza un ticket de más.
- [x] El trigger `tickets_business_match_trg` rechaza un ticket de otro negocio.
- [x] `audit_log` rechaza `UPDATE` y `DELETE`.
- [x] Las políticas de RLS **fallan cerradas**: sin contexto de tenant no se lee nada.
- [x] Las 12 migraciones de §7.2, en orden, aplicadas desde cero.
- [x] `suertudo-db migrate` aplica las migraciones.
- [x] Tests de integración contra un Postgres real: 22 tests.

**Verificación:** `DATABASE_URL_TEST=… cargo test -p suertudo-db` → 22 tests en verde.

**Desviaciones que el trabajo le impuso al documento** (corregidas en `data-model.md` §3.19): dos foreign keys circulares partidas entre archivos, el índice de email único, el RLS sobre 13 tablas con dos excepciones públicas, y la condición de consentimiento de §6.4 corregida para que un consentimiento revocado no se exporte.

**Hallazgo que cambió la arquitectura:** el propietario de una tabla en Postgres **no le aplican** sus políticas de RLS. El rol de la API tiene que ser un rol sin propiedad sobre el esquema, en producción y en los tests. `data-model.md` §3.20.

---

### `TSK-F0-04` — Identidad: registro, magic link, sesión

**Épica:** E0.4 · **Depende de:** `TSK-F0-02`, `TSK-F0-03` · **Esfuerzo:** 2 d · **Estado: COMPLETADA**

**Contexto:** el primer vertical de la API. Es también el que define el patrón de todos los handlers.

**Criterios de aceptación:**
- [x] `POST /v1/auth/magic-link` responde `202` siempre, exista o no el email.
- [x] El enlace expira en 15 minutos y es de un solo uso (`400 AUTH_LINK_CONSUMED` al segundo uso).
- [x] `POST /v1/auth/magic-link/verify` devuelve access token (15 min) y refresh token (30 días).
- [x] `POST /v1/auth/refresh` rota el refresh token; usar uno ya rotado invalida la familia completa.
- [x] El access token es un JWT firmado HS256 con `sub`, `role`, `exp`, `iat` y `jti`.
- [x] El middleware `identity::authorize` resuelve la sesión e inyecta el contexto.
- [x] En `ENV=development` la respuesta incluye `dev_magic_link`; en otro entorno el **arranque falla**.
- [x] Test: un token expirado da `401 AUTH_EXPIRED`.

**Verificación:** `DATABASE_URL_TEST=… cargo test -p suertudo-identity` → 31 tests en verde (14 de dominio sin base, 17 de integración).

**Lo que el trabajo le agregó al contrato:**
- `auth_tokens` — la tabla que el modelo de datos **no tenía** y que la identidad necesita: un magic link es de un solo uso y un refresh token rota con su familia. `data-model.md` §3.17.
- `users.notification_preferences` — `marketing_opt_in` no alcanzaba para los canales operativos. Migración `0014`.
- El índice `auth_tokens_one_live_per_family_idx` — el original era **muerto**: filtraba por una columna que la rotación nunca escribe. El correcto impide que dos refresh concurrentes produzcan dos sucesores vivos.
- `revoke_token` ahora **informa** si revocó. Sin eso, la ventana entre leer un token y revocarlo dejaba pasar un segundo refresh concurrente que emitía un sucesor válido sin matar la familia (`FR-ID-04`).
- `sqlx::migrate!` **embebe los archivos al compilar**: agregar una migración no invalida el build y el binario aplica el conjunto viejo en silencio. `data-model.md` §3.21.

**Fuera de su alcance, con dueño asignado:** el login del comercio con contraseña (`FR-ID-02`) es `TSK-F1-01`.

---

### `TSK-F0-10` — El token de dispositivo y la clase del error

**Épica:** E0.10 · **Depende de:** `TSK-F0-04` · **Esfuerzo:** 1 d · **Estado: COMPLETADA**

**Contexto:** el token de un dispositivo push identifica **un teléfono**, no a una persona. La fila de `push_devices` guarda el `user_id` de quien la registró, y `register_push_device` rechazaba con `42501` el token que ya era de otro. El escenario real: la misma persona crea una segunda cuenta en el mismo teléfono. La cuenta nueva **perdía el push en silencio**, y el error que recibía era `403 RBAC_FORBIDDEN`, que dice "tu rol no alcanza" y no dice nada del teléfono.

**Decisión de producto tomada:** cuando el token se repite, se abre un **flujo de verificación por SMS**. El token **no cambia de dueño hasta que alguien verifica**.

**La decisión de seguridad que define el flujo:**

> El SMS va al teléfono **del dueño actual del token**, no al del que lo reclama. Si fuera al del reclamante, un atacante que copió el token recibiría su propio código y la transferencia funcionaría.

**Criterios de aceptación:**
- [x] Un token libre se registra sin verificación: `201`.
- [x] Un token de otro participante abre una verificación: `202` con el id, el estado y el vencimiento.
- [x] El SMS va al teléfono del dueño actual del token. Hay un test que lo prueba.
- [x] El código son seis dígitos, vive diez minutos y admite cinco intentos. Sólo se guarda su SHA-256.
- [x] El código correcto transfiere el dispositivo; el incorrecto responde `400 DEVICE_CODE_INVALID` y cuenta el intento.
- [x] Una verificación vencida responde `409 DEVICE_VERIFICATION_EXPIRED`.
- [x] Un dueño sin teléfono en ficha responde `409 DEVICE_OWNER_UNVERIFIABLE` y el token no se mueve.
- [x] El cierre de sesión libera los dispositivos: un dispositivo es de una sesión, no de una persona.
- [x] El worker **se niega a arrancar** fuera de development con el emisor de desarrollo, que escribe el código en el log.

**Lo que quedó para la Fase 1:** el proveedor real de SMS. El flujo está construido contra un puerto (`SmsSender`), así que integrarlo es escribir una implementación. Ver `TSK-F0-11`.

**Riesgo:** el token de un dispositivo es una credencial, y ahora se mueve entre participantes después de una verificación por SMS.

---

### `TSK-F0-11` — Proveedor de SMS

**Épica:** E0.11 · **Depende de:** `TSK-F0-10` · **Esfuerzo:** 0,5 d

**Contexto:** el flujo de toma de control de un dispositivo está construido contra un puerto, y el único emisor que existe escribe el código en el log.

**El despliegue de staging ya no está bloqueado:** el emisor de desarrollo se acepta en staging con el opt-in explícito `DEV_SMS_SENDER=true`, así que el flujo se puede probar de punta a punta antes de que exista el proveedor. **El de producción sigue bloqueado**, y sin excepción: nadie despliega un servicio que escribe los códigos de sus usuarios en el log.

**Lo que tiene que cumplir la implementación:**

| # | Garantía |
|---|---|
| 1 | **Nunca devuelve `Ok` por un mensaje que no entregó al proveedor.** Un éxito mudo es una verificación que nadie puede confirmar. |
| 2 | **No registra el cuerpo fuera de development.** El cuerpo lleva el código de una toma de control. |
| 3 | **Es honesto con los fallos**: un rechazo permanente del proveedor y una falla temporal del canal son errores distintos. Confundirlos hace que el participante espere un código que nunca llega. |
| 4 | **No reintenta por su cuenta de una forma que duplique el código**: la verificación lleva un código y un hash, y un segundo código invalidaría el primero. |
| 5 | **Es barata de probar**: el puerto no tiene estado ni I/O propios, así que el harness sustituye un grabador. |

**Criterios de aceptación:**
- [ ] Hay una implementación real del puerto `SmsSender`.
- [ ] `SMS_PROVIDER` la selecciona, y sus credenciales vienen del secret manager.
- [ ] El chequeo de arranque deja de rechazar cuando el proveedor es el real. **Sigue rechazando el emisor de desarrollo en producción**, y en staging lo acepta sólo con `DEV_SMS_SENDER=true`.
- [ ] La distinción entre rechazo permanente y falla temporal está en el código y tiene test.
- [ ] `docs/architecture.md` §8.3 dice qué hace falta para desplegar la identidad en producción.

---

---

### `TSK-F0-05` — Pipeline de CI

**Épica:** E0.5 · **Depende de:** `TSK-F0-01` · **Esfuerzo:** 1 d · **Estado: COMPLETADA**

**Contexto:** el pipeline tiene que ser la red de seguridad de todo lo que viene. Un PR que rompe el build no llega a main.

**Archivos:** `.github/workflows/ci.yml`, `.github/workflows/smoke.yml`, `scripts/smoke.sh`, `apps/mobile/test/app_smoke_test.dart`, `apps/business-web/.nvmrc`

**Criterios de aceptación:**
- [x] Job de Rust: `fmt --all -- --check`, `clippy -D warnings`, `test --workspace`, `cargo audit`, y el cache offline de sqlx contra el esquema.
- [x] Job de Flutter: `analyze`, `test`, `format --set-exit-if-changed`.
- [x] Job de Next.js: `lint`, `typecheck`, `build`.
- [x] Smoke test: levanta el motor iii, registra un worker, invoca una función por HTTP y verifica la respuesta.

**Criterios que no se pueden cumplir todavía, y por qué:**
- [ ] Job de Next.js: `test`. El panel no tiene runner de tests, ni tests. Instalar una cadena de dependencias nueva para testear un scaffolding que la Fase 1 va a reemplazar no compra nada. El criterio se cierra con la primera pantalla del panel.
- [ ] Job de contrato: compara el OpenAPI generado contra el commiteado. **No hay OpenAPI**: el contrato vive escrito a mano en [`api-contract.md`](./api-contract.md) y la API sólo expone `/v1/health`. Diferenciar un archivo contra sí mismo no es un check. Llega con los primeros endpoints de la Fase 1.
- [ ] Job de evals de IA: corre sólo si el PR toca `services/ai-branding/`. Ese crate es de la Fase 2.

**El check del cache de sqlx, sin `sqlx prepare --check`.** El cache de `services/db/.sqlx` está commiteado a propósito: sin él, `cargo build` en un checkout limpio falla. El CI lo comprueba con otro mecanismo, que no necesita instalar `sqlx-cli`: compila el crate con la base de datos viva y con `SQLX_OFFLINE_DIR` apuntando al cache commiteado. Los macros se expanden contra el esquema real y **escriben** el cache; si una query cambió sin regenerarlo, aparece un archivo nuevo y `git status` lo ve. `SQLX_OFFLINE` no se pone en ese paso a propósito: con él los macros leerían el cache en vez del esquema, y el check pasaría sin comparar nada.

**Una sola fuente de verdad para cada versión.** Un CI que compila con otro compilador que el del desarrollador es un CI que prueba otra cosa: la versión de Rust vive en `rust-toolchain.toml`, la de Node en `apps/business-web/.nvmrc`, la de Flutter y la de iii en el pipeline y en `infra/iii/VERSION`.

**Verificado en local, no sólo escrito:**
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` y `cargo test --workspace`: limpios.
- El check del cache de sqlx: limpio con el esquema actual, y **con dientes** — al cambiar una query a mano, el check lo detectó.
- Flutter: `analyze` sin hallazgos, `format` sin cambios, 1 test verde.
- Next.js: `lint` limpio, `tsc --noEmit` sin errores, `build` OK.
- `unshare -rm ./scripts/smoke.sh`: **PASS**, las quince secciones en verde.

**Revisión independiente:** el pipeline pasó por la revisión nativa de Gentle AI, con riesgo alto y tres lentes —riesgo, legibilidad y fiabilidad—. Salió **aprobada**. Un hallazgo crítico fue **refutado**: decía que el job de Rust no exporta la base y que los macros de sqlx no expandían sin ella, y es falso —el build cae al cache commiteado sin `DATABASE_URL`, y está verificado—. Los veinticuatro hallazgos restantes son advertencias informativas.

**Lo que se corrigió después de la revisión:**
- El motor se instala con la versión declarada, y ahora el paso **verifica que la versión no esté vacía**. `bash -e` sin `pipefail` enmascara un `grep` fallido detrás del `cut` exitoso: una clave renombrada instalaba el **último** motor en silencio, que es exactamente la clase de fallo que este pipeline existe para atrapar.
- Todos los comandos de cargo van con `--locked`. Sin eso, cargo puede re-resolver las dependencias, y el pipeline auditaría y probaría un árbol que no es el commiteado.
- El probe del spike se compila con `--locked`. El spike está excluido del workspace y tiene su propio lockfile, así que `cargo build --workspace` nunca lo construye: sin `--locked`, cargo puede re-resolver sus dependencias en silencio y el pipeline probaría un árbol que no es el commiteado. **Ojo con lo que el spike NO es**: no tiene tests unitarios —su `lib.rs` es una línea—; sus aserciones son las del script de smoke, que es donde debe estar.
- El job de Flutter falla si queda un `test.only`. Dart no tiene `forbidOnly`: un test enfocado hace que la suite corra **un** test y salga con 0, y el pipeline queda verde mientras el resto nunca corrió.
- El check del cache de sqlx pone `SQLX_OFFLINE=false` explícito, para que no se vuelva un no-op si algún día se activa el modo offline en el pipeline.
- Se podaron **8 archivos stale** del cache de sqlx: quedaban de queries que ya no existen, y la expansión nunca poda. Ahora el cache tiene exactamente las 7 queries vivas.

**Queda abierto como trabajo posterior:** anclar las actions de terceros a su SHA en vez de a su etiqueta mayor, y verificar el instalador del motor con un checksum. El instalador se baja de una URL mutable y se ejecuta en cada push: hoy es el único punto del pipeline que ejecuta código de un tercero sin fijar.

---

### `TSK-F0-06` — Cola y cron de iii (path sin verificar)

**Épica:** E0.6 · **Depende de:** `TSK-F0-02` · **Esfuerzo:** 1 d · **Estado: COMPLETADA**

**Contexto:** el spike `spikes/iii-health` verificó el engine, los triggers `http` y el registro de funciones, pero **no** la ruta de colas. El engine registra un warning explícito: `trigger type durable:subscriber is not registered ... If this persists, the queue worker is missing`. `draws::execute_draw` y `notifications::dispatch` dependen de esa ruta, así que hay que probarla antes de construir encima.

**Archivos:** `scripts/smoke.sh` (extender), `spikes/iii-health/src/queue_probe.rs` (nuevo), `docs/api-contract.md` §7.3

**Criterios de aceptación:**
- [x] El worker `queue` del registro alcanza estado `ready` y provee `durable:subscriber` y `engine::queue::*`.
- [x] Un mensaje publicado con `iii::durable::publish` llega a su suscriptor.
- [x] Un mensaje encolado con `TriggerAction::Enqueue { queue }` es consumido por la función destino.
- [x] Un consumidor que falla reintenta 3 veces con backoff y después va a la DLQ.
- [x] `engine::queue::dlq_topics` y `engine::queue::dlq_messages` muestran el mensaje muerto.
- [x] `iii::queue::redrive` reprocesa el mensaje; `iii::queue::discard_message` lo descarta.
- [x] Un trigger `cron` se dispara efectivamente (contador de ticks).
- [x] Todo el stack sigue levantando **sin KVM**.
- [x] `scripts/smoke.sh` queda como el smoke test del CI, con teardown por nombre de proceso exacto.
- [x] El adaptador `rabbitmq` se verificó contra un broker real y dio lugar a `ADR-013`.

**Verificación:** `unshare -rm ./scripts/smoke.sh` → `RESULTADO: PASS` (13 secciones)

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

### `TSK-F0-07` — Rate limits de la API

**Épica:** E0.7 · **Depende de:** `TSK-F0-04` · **Esfuerzo:** 0,5 d

**Contexto:** `api-contract.md` §1.6 define seis grupos de rate limit. No hay ninguno implementado: sin ellos, el registro por magic link es un vector de spam de correo y de enumeración de emails.

**Criterios de aceptación:**
- [ ] Los seis grupos de §1.6 funcionan, con el límite por IP y por email donde corresponde.
- [ ] La respuesta `429` incluye `Retry-After` y `X-RateLimit-Remaining`.
- [ ] `users.last_magic_link_at` se usa para el grupo `auth:link`.
- [ ] El rate limit vive en Redis, no en la memoria del proceso.

---

### `TSK-F0-08` — Envío real del magic link

**Épica:** E0.8 · **Depende de:** `TSK-F0-04` · **Esfuerzo:** 0,5 d

**Contexto:** el magic link se genera y se guarda, pero **nadie lo envía**: en development viaja en la respuesta y en staging no hay proveedor de correo. Sin esto, el flujo de registro no se puede probar fuera de la máquina.

**Criterios de aceptación:**
- [ ] Un worker de correo envía el magic link con una plantilla versionada.
- [ ] En `staging` el correo llega a la allow-list y a nadie más.
- [ ] El token en claro existe una sola vez: en el correo, nunca en la base.
- [ ] `consent_records.ip_hash` se completa con un salt definido y rotable (hoy queda `NULL`).

---

### `TSK-F0-09` — Rol de aplicación y smoke test del backend

**Épica:** E0.9 · **Depende de:** `TSK-F0-03` · **Esfuerzo:** 0,5 d

**Contexto:** el aislamiento por RLS depende de que la aplicación **no** sea la propietaria del esquema, y el smoke test del spike todavía no levanta los workers del proyecto.

**Criterios de aceptación:**
- [ ] Existe un rol de aplicación con permisos sobre las tablas de tenant y **sin** propiedad del esquema.
- [ ] El despliegue crea ese rol; ninguna migración lo hace.
- [ ] `scripts/dev.sh up` levanta el engine, los workers del registro y los workers del proyecto, y `GET /v1/health` responde `200`.
- [ ] El smoke test verifica que `SELECT max(version) FROM _sqlx_migrations` coincide con la cantidad de archivos `.sql`.
- [ ] El smoke test verifica que el log del engine no contiene `is not registered`.

---

## 3. Fase 1 — MVP vertical

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
| 0 — Cimientos | 11 | 11 | ~12 d | El proveedor de SMS, que bloquea el despliegue fuera de development (`TSK-F0-11`) |
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
