# Contrato de API — Suertudo

> Contrato HTTP y de funciones iii. Este documento es **normativo**: la implementación debe coincidir, y cualquier desviación se reporta como bug, no como "variación".
>
> En la práctica, este documento y las **migraciones + tests de contrato** son la fuente de verdad. Los ejemplos de payload son normativos; los de responseComplete son ilustrativos cuando se marca como tal.

---

## 1. Convenciones

### 1.1 Transporte

| Aspecto | Regla |
|---|---|
| Base | `https://api.suertudo.app/v1` |
| Formato | JSON (`application/json; charset=utf-8`) salvo indicación contraria |
| Versionado | En la URL (`/v1`). Cambios que rompen compatibilidad → `/v2`. Agregar campos es compatible. |
| Autenticación | `Authorization: Bearer <access_token>` |
| Idempotencia | `Idempotency-Key: <uuid>` en toda escritura de negocio. Repetir con la misma key devuelve la **misma** respuesta (código y body) sin reejecutar. |
| Trazas | El cliente puede mandar `traceparent` (W3C). Se propaga a todos los workers. |
| Compresión | `gzip` y `br` en respuestas > 1 KB. |
| Límite de body | 1 MB (excepto la carga de logos: 5 MB) |

### 1.2 Convenciones de nombres

| Concepto | Formato | Ejemplo |
|---|---|---|
| IDs | `uuid` v4 en JSON | `9f2c1b7e-...` |
| Fechas | ISO 8601 con offset | `2026-11-14T21:00:00-03:00` |
| Horas | RFC 3339 | `21:00:00-03:00` |
| Números de sorteo | `integer` ≥ 0 | `742` |
| Códigos de ticket | `SU-XXXXXXX` (alfanumérico, sin caracteres ambiguos) | `SU-TKU4F9K` |
| Códigos de canje | `SU-CXXXXXX` | `SU-C93KD2P` |
| Códigos de validación | `SU-VXXXXXX` | `SU-V41MZQ8` |
| Enums | `snake_case` en minúsculas | `pending_review` |

### 1.3 Respuesta estándar de error

Todos los errores usan la misma forma:

```json
{
  "error": {
    "code": "RAFFLE_CLOSED",
    "message": "El sorteo se cerró el 14/11/2026 a las 21:00.",
    "details": { "raffle_id": "9f2c1b7e-...", "closed_at": "2026-11-14T21:00:00-03:00" },
    "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736"
  }
}
```

`message` es **humano y accionable**, en español rioplatense, y está pensado para mostrarse tal cual en la app (excepto en errores del panel web, donde se muestra en un toast). Nunca contiene stack traces ni nombres de tablas.

### 1.4 Catálogo de errores

| Code | HTTP | Cuándo ocurre |
|---|---|---|
| `VALIDATION_ERROR` | 400 | Faltan campos o tienen formato inválido. `details` lista los campos. |
| `AUTH_REQUIRED` | 401 | Sin token o token inválido. |
| `AUTH_EXPIRED` | 401 | Access token vencido. El cliente debe refrescar. |
| `AUTH_LINK_CONSUMED` | 400 | El magic link ya se usó. |
| `EMAIL_TAKEN` | 409 | El email ya está registrado. |
| `FORBIDDEN` | 403 | Autenticado pero sin permiso. |
| `RBAC_FORBIDDEN` | 403 | El rol no alcanza para la operación. |
| `BUSINESS_SUSPENDED` | 403 | El negocio está suspendido. |
| `PLAN_LIMIT_REACHED` | 402 | Se excedió un límite del plan. `details` dice cuál. |
| `NOT_FOUND` | 404 | El recurso no existe **o** no es visible para quien pregunta. Nunca se distingue entre ambos casos. |
| `METHOD_NOT_ALLOWED` | 405 | Método HTTP no soportado en esa ruta. |
| `CONFLICT` | 409 | Conflicto de estado genérico. |
| `INVALID_STATE_TRANSITION` | 409 | La transición de estado no es válida. `details.valid_transitions` lista las permitidas. |
| `IMMUTABLE_FIELD` | 409 | Se intentó modificar un campo inmutable (ej. el compromiso del sorteo). |
| `RAFFLE_CLOSED` | 409 | El sorteo ya no acepta reservas. |
| `RAFFLE_NOT_OPEN` | 409 | El sorteo todavía no abrió. |
| `NUMBER_TAKEN` | 409 | Alguno de los números ya está reservado. `details.taken_numbers` los lista. |
| `TICKET_LIMIT_REACHED` | 409 | El usuario alcanzó el máximo de tickets para ese sorteo. |
| `PRIZE_ALREADY_CLAIMED` | 409 | El premio ya fue reclamado por su ganador. |
| `DEVICE_CODE_INVALID` | 400 | El código de la verificación de un dispositivo es incorrecto. |
| `DEVICE_VERIFICATION_EXPIRED` | 409 | La verificación de la toma de control de un dispositivo venció. |
| `DEVICE_OWNER_UNVERIFIABLE` | 409 | El dueño actual del dispositivo no tiene teléfono en su ficha: la toma de control no se puede verificar. |
| `TICKET_SIZE_OUT_OF_RANGE` | 422 | La cantidad de números está fuera de `[min, max]`. |
| `INVALID_VALIDATION_CODE` | 422 | El código no existe o está revocado. |
| `VALIDATION_CODE_USED` | 409 | El código ya se usó. |
| `PROMPT_TOO_LONG` | 422 | El prompt excede 500 caracteres. |
| `AI_UNAVAILABLE` | 503 | La generación por IA no pudo completarse. El tema por defecto se aplica. |
| `RATE_LIMITED` | 429 | Rate limit excedido. Siempre incluye `Retry-After`. |
| `MESSAGE_RATE_LIMITED` | 429 | El negocio excedió el límite de mensajes. |
| `INTERNAL_ERROR` | 500 | Error no esperado. Se registra con `trace_id`. |

### 1.5 Paginación

**Cursor-based.** Offset pagination se rompe con escrituras concurrentes, y el feed cambia constantly.

```
GET /v1/public/raffles?limit=20&cursor=eyJpZCI6Ii4uLiJ9
```

```json
{
  "data": [ /* ... */ ],
  "page": {
    "next_cursor": "eyJpZCI6Ii4uLiJ9",
    "has_more": true,
    "limit": 20
  }
}
```

El cursor es un **cursor opaco** (base64url de un JSON con el último id y el timestamp de orden). El cliente no debe parsearlo. Cuando `has_more` es `false`, `next_cursor` es `null`.

### 1.6 Rate limits

| Grupo | Límite | Ventana | Alcance |
|---|---|---|---|
| `auth` | 10 | 5 min | Por IP y por email |
| `auth:link` | 5 | 15 min | Por email |
| `raffles:read` | 120 | 1 min | Por token |
| `tickets:create` | 10 | 5 min | Por usuario y por IP |
| `ai:generate` | 20 | 1 h | Por business |
| `export` | 5 | 1 h | Por business |
| `public` | 300 | 1 min | Por IP |

Respuesta: `429` con `Retry-After: <segundos>` y `X-RateLimit-Remaining`.
Un `Retry-After` de `0` nunca sale: "probá ahora mismo" es justamente la
rafaga que el límite está para frenar.

**Cómo está implementado** (`TSK-F0-07`):

- **Los contadores viven en Redis**, no en la memoria del proceso: hay más
  de un worker y un límite que cada uno cuenta por su lado es un límite
  que cada uno puede rodear por su lado. Cada worker arranca **con
  conexión al store**: un worker que no puede alcanzar Redis no arranca.
- **Ventana fija**: `INCR` + `EXPIRE` en una sola operación atómica (script
  Lua), así que la ventana empieza con el primer golpe y no se estira.
- **El store no conoce la política**: devuelve el golpe contados; el
  cálculo del `remaining` y la decisión vive en el dominio. Las claves son
  `SHA-256` del grupo, el alcance y el sujeto: **ningún email ni IP va
  en claro** al store.
- **Store caído no es permiso**: la respuesta es `500 INTERNAL_ERROR` con
  `Retry-After` — no un `429`, porque el límite no se agotó: no se pudo
  consultar. Falla **cerrada**.
- **Dónde aplica hoy**: la fase de credenciales ya lo ejecuta —
  `POST /v1/auth/magic-link` (grupos `auth` y `auth:link`),
  `POST /v1/auth/verify` y `POST /v1/auth/refresh` (grupo `auth`). El
  resto de los grupos entran con sus endpoints cada vez que se
  implementen. `logout` y los endpoints autenticados de `/v1/me` no
  llevan grupo: no exponen una credencial utilisable, el límite ahí no
  tiene algo que proteger.
- **El enfriamiento de `auth:link` es durable**: además del contador, la
  separación mínima (ventana sobre límite: 15 min / 5 = 3 min) vive en
  `users.last_magic_link_at`, marcado por la función `touch_last_magic_link`
  que devuelve la emisión anterior en la misma operación. **Cada intento
  renueva la espera**, incluido el rechazado: es lo que mantiene a un
  abusador afuera. Una dirección que todavía no tiene usuario no tiene
  registro que marcar: la cubren el contador de `auth:link` y el de
  `auth` por IP.

---

## 2. Endpoints públicos

Sin autenticación. Cacheables con `ETag` y `Cache-Control`.

### 2.1 Salud

```http
GET /v1/health
```

```json
{ "status": "ok", "version": "1.0.0", "time": "2026-11-14T18:00:00Z" }
```

No revela dependencias ni su estado. El detalle va al endpoint de admin.

### 2.2 Listar sorteos activos

```http
GET /v1/public/raffles?limit=20&cursor=...&category=restaurant&city=CABA
```

Filtros: `category`, `city`, `province`, `business_id`, `closing_within` (horas), `prize_type`.

```json
{
  "data": [
    {
      "public_id": "3f8a1c22-0b6e-4a91-8d3f-2c7e5a9b1d04",
      "slug": "sorteo-mothers-day",
      "title": "Sorteo Mother's Day",
      "subtitle": "Un brunch para dos personas",
      "business": {
        "public_id": "b1c2d3e4-...",
        "name": "Café Whisk",
        "slug": "cafe-whisk",
        "logo_url": "https://cdn.suertudo.app/logos/b1c2d3e4.png",
        "category": "restaurant",
        "city": "CABA"
      },
      "prize_name": "Brunch para dos",
      "prize_image_url": null,
      "cover_image_url": "https://cdn.suertudo.app/covers/3f8a1c22.jpg",
      "status": "published",
      "closes_at": "2026-11-21T21:00:00-03:00",
      "draws_at": "2026-11-21T21:05:00-03:00",
      "number_pool_size": 1000,
      "numbers_available": 847,
      "numbers_per_ticket_min": 1,
      "numbers_per_ticket_max": 5,
      "tickets_count": 153,
      "theme": { "id": "tv-uuid", "layout": "festive", "palette": { "primary": "#1E5F74" } },
      "commitment": "9f2b…",
      "commitment_algorithm": "hmac-sha256/v1"
    }
  ],
  "page": { "next_cursor": null, "has_more": false, "limit": 20 }
}
```

### 2.3 Detalle de un sorteo

```http
GET /v1/public/raffles/{public_id}
```

Devuelve el objeto del listado **más**: `description`, `rules_md`, `prize_description`, `prize_delivery_instructions`, `prize_quantity`, `participation_mode`, `draw_unit`, `no_winner_policy`, `full_theme`, `my_participation` (si hay sesión; si no, `null`).

**Nunca** incluye datos de participantes, ni el `seed` sin revelar, ni la lista de números reservados.

### 2.4 Verificación del sorteo

```http
GET /v1/public/raffles/{public_id}/verify
```

```json
{
  "raffle_id": "3f8a1c22-0b6e-4a91-8d3f-2c7e5a9b1d04",
  "algorithm": "hmac-sha256/v1",
  "derivation": {
    "hash": "HMAC-SHA256",
    "message_template": "suertudo/v1|raffle:{raffle_id}|draw:{draw_index}",
    "mod": "pool_size",
    "encoding": "u64_be(first 8 bytes)"
  },
  "pool_size": 1000,
  "commitment": "9f2b8c…",
  "commitment_algorithm": "sha256",
  "seed_revealed": true,
  "seed": "5f8a…(hex, 64 caracteres)",
  "revealed_at": "2026-11-21T21:05:03-03:00",
  "draws": [
    { "draw_index": 0, "winning_number": 742, "executed_at": "2026-11-21T21:05:03-03:00" }
  ],
  "steps": [
    "Tomá el seed en hex y convertilo a 32 bytes.",
    "Calculá HMAC-SHA256(key = seed, msg = \"suertudo/v1|raffle:3f8a1c22-…|draw:0\").",
    "Tomá los primeros 8 bytes del resultado como entero big-endian sin signo.",
    "Aplicá módulo 1000 (pool_size) para obtener el número ganador.",
    "El resultado debe ser 742."
  ]
}
```

**Este endpoint es un contrato de confianza.** Su forma no cambia sin versionar: es la base de la promesa de que cualquier tercero puede auditar el sorteo.

### 2.5 Número reservado (anónimo)

```http
GET /v1/public/raffles/{public_id}/numbers/{number}
```

```json
{ "number": 742, "reserved": true, "reserved_count": 3 }
```

Permite mostrar "el 742 ya está taken" sin revelar quién lo tiene. `reserved_count` es la cantidad de números que tiene ese ticket (siempre 1 en la práctica, pero el dato es genérico).

---

## 3. Endpoints de identidad

### 3.1 Solicitar enlace mágico

```http
POST /v1/auth/magic-link
```

```json
{ "email": "ana@example.com", "name": "Ana" }
```

→ `202 Accepted` siempre, exista o no el email (evita enumeración de cuentas).

```json
{
  "data": {
    "sent": true,
    "expires_in": 900,
    "delivery": "email"
  }
}
```

> En desarrollo (`ENV=development`), la respuesta incluye `dev_magic_link` con el token. **Nunca en staging ni producción** — el servidor rechaza el arranque si `ENV != development` y la variable `DEV_MAGIC_LINK` está activa.

### 3.2 Verificar enlace mágico

```http
POST /v1/auth/magic-link/verify
```

```json
{ "token": "eyJ1c2VyX2lkIjoi…" }
```

→ `200` con tokens, o `400 AUTH_LINK_CONSUMED` / `400 VALIDATION_ERROR` (token vencido).

```json
{
  "data": {
    "access_token": "eyJhbGciOi…",
    "refresh_token": "rt_9f2c…",
    "token_type": "Bearer",
    "expires_in": 900,
    "user": {
      "id": "u-uuid",
      "email": "ana@example.com",
      "display_name": "Ana",
      "role": "participant",
      "phone_e164": null,
      "marketing_opt_in": false
    }
  }
}
```

### 3.3 Login de comercio

```http
POST /v1/auth/login
```

```json
{ "email": "duenio@cafe-whisk.com", "password": "…" }
```

Devuelve la misma forma de token. Si el usuario es `business_owner` o tiene membresía en un negocio, el token incluye el contexto de negocio.

### 3.4 Refresh

```http
POST /v1/auth/refresh
```

```json
{ "refresh_token": "rt_9f2c…" }
```

**Rotación:** cada refresh emite un refresh token nuevo e invalida el anterior. Si se usa un token ya rotado, **toda la familia** queda invalidada (detección de robo de token) y el usuario debe volver a autenticarse.

### 3.5 Logout

```http
POST /v1/auth/logout
```

Revoca la familia de refresh tokens actual.

### 3.6 Perfil

```http
GET    /v1/me
PATCH  /v1/me
```

```json
{
  "display_name": "Ana",
  "phone_e164": "+5491122334455",
  "locale": "es-AR",
  "timezone": "America/Argentina/Buenos_Aires",
  "notification_preferences": { "push": true, "email": true, "marketing": false }
}
```

`PATCH /v1/me` con `{ "phone_e164": null }` **no** borra el teléfono: lo deja en `null`. La auditoría registra **el hecho** del cambio, no el valor: guarda qué campos cambiaron, nunca sus valores. Para borrado real, `DELETE /v1/me/contact`.

### 3.7 Baja de cuenta

```http
DELETE /v1/me
```

```json
{ "reason": "no_uso_mas", "confirm": "BORRAR" }
```

Anonimiza PII, pone `tickets.user_id = NULL`, conserva los premios. Devuelve `202` (el borrado real es un job diferido de 30 días).

### 3.8 Dispositivos push

```http
POST   /v1/me/devices
DELETE /v1/me/devices/{id}
POST   /v1/me/devices/verifications/{id}/confirm
```

**El token de un dispositivo identifica un teléfono, no a una persona.** Es único de forma global, así que la misma cuenta puede pedir un token que ya es de otra: eso no es un ataque, es la misma persona con una segunda cuenta.

Cuando el token ya pertenece a otro participante, el registro **no falla**: abre una **verificación por SMS** y responde `202`.

| Respuesta | Cuándo | Cuerpo |
|---|---|---|
| `201` | El token estaba libre, o ya era del participante | El dispositivo registrado |
| `202` | El token es de otro participante | `{ "verification_id": "…", "status": "pending", "expires_at": "…" }` |
| `409 DEVICE_OWNER_UNVERIFIABLE` | El dueño actual no tiene teléfono en ficha | No se puede verificar, el token no se mueve |

**El SMS va al teléfono del dueño actual del token, no al del que lo reclama.** Si fuera al del reclamante, un atacante que copió el token recibiría su propio código y la transferencia funcionaría. Mandándolo al dueño, solo quien tiene el teléfono en la mano puede autorizar, y el dueño también puede negarse: no haciendo nada.

El código son **seis dígitos**, vive **diez minutos** y admite **cinco intentos**. Solo se guarda su SHA-256.

```http
POST /v1/me/devices/verifications/{id}/confirm
{ "code": "123456" }
```

| Respuesta | Cuándo |
|---|---|
| `200` | El código era correcto y el dispositivo cambió de dueño |
| `400 DEVICE_CODE_INVALID` | El código no era |
| `409 DEVICE_VERIFICATION_EXPIRED` | La verificación venció |
| `403 RBAC_FORBIDDEN` | La verificación no es de quien la confirma |

**El cierre de sesión libera los dispositivos.** Un dispositivo es de una sesión, no de una persona: si no se liberan, la misma persona con una segunda cuenta pierde el push en silencio.

```json
{ "platform": "android", "token": "fcm-token", "app_version": "1.2.0" }
```

### 3.9 Consentimientos

```http
GET /v1/me/consents
PUT  /v1/me/consents/{business_id}
```

```json
{ "share_marketing": true, "share_contact": true }
```

Registra un nuevo `consent_record` (append-only) con `source = 'settings'`.

---

## 4. Endpoints de participante

### 4.1 Reservar números

```http
POST /v1/raffles/{public_id}/tickets
Idempotency-Key: 4a3f-…
```

```json
{
  "numbers": [742, 118, 903],
  "auto_pick": false,
  "validation_code": "SU-V41MZQ8",
  "consents": { "share_marketing": true, "share_contact": true }
}
```

O, para reserva automática:

```json
{ "auto_pick": true, "count": 3, "consents": { "share_marketing": false } }
```

**Reglas de validación:**

- `numbers` XOR `auto_pick` (exactamente uno).
- Si `numbers`: `numbers_per_ticket_min ≤ len(numbers) ≤ numbers_per_ticket_max`, todos distintos, todos dentro de `[0, number_pool_size)`.
- Si `auto_pick`: `count` en el mismo rango.
- `validation_code` es obligatorio si `participation_mode = 'validated'`.
- `consents` es obligatorio (aunque sea `false` en todos).

Respuesta `201`:

```json
{
  "data": {
    "id": "t-uuid",
    "code": "SU-TKU4F9K",
    "raffle": {
      "public_id": "3f8a1c22-…",
      "title": "Sorteo Mother's Day",
      "business_name": "Café Whisk",
      "closes_at": "2026-11-21T21:00:00-03:00"
    },
    "numbers": [742, 118, 903],
    "auto_picked": false,
    "status": "reserved",
    "created_at": "2026-11-14T18:03:11-03:00"
  }
}
```

Errores frecuentes: `409 NUMBER_TAKEN` (con `details.taken_numbers`), `409 TICKET_LIMIT_REACHED`, `409 RAFFLE_CLOSED`, `422 TICKET_SIZE_OUT_OF_RANGE`, `422 INVALID_VALIDATION_CODE`.

### 4.2 Cancelar un ticket

```http
DELETE /v1/tickets/{ticket_id}
```

Solo mientras el sorteo esté `published`. Devuelve `204`.

### 4.3 Mis participaciones

```http
GET /v1/me/tickets?status=reserved&limit=20&cursor=...
```

```json
{
  "data": [
    {
      "id": "t-uuid",
      "code": "SU-TKU4F9K",
      "numbers": [742, 118, 903],
      "status": "winner",
      "raffle": {
        "public_id": "3f8a1c22-…",
        "title": "Sorteo Mother's Day",
        "business_name": "Café Whisk",
        "cover_image_url": "https://cdn.suertudo.app/covers/3f8a1c22.jpg",
        "status": "drawn",
        "closes_at": "2026-11-21T21:00:00-03:00",
        "draws_at": "2026-11-21T21:05:00-03:00",
        "winning_number": 742
      },
      "prize": {
        "id": "p-uuid",
        "prize_name": "Brunch para dos",
        "status": "claimed",
        "claim_code": "SU-C93KD2P",
        "expires_at": "2026-12-21T21:00:00-03:00"
      },
      "created_at": "2026-11-14T18:03:11-03:00"
    }
  ],
  "page": { "next_cursor": null, "has_more": false, "limit": 20 }
}
```

### 4.4 Mis premios

```http
GET  /v1/me/prizes?status=pending
POST /v1/me/prizes/{prize_id}/claim
```

El `claim` devuelve el `claim_code` que el ganador muestra en el local.

### 4.5 Notificaciones

```http
GET  /v1/me/notifications?unread=true
POST /v1/me/notifications/{id}/read
POST /v1/me/notifications/read-all
```

---

## 5. Endpoints de comercio

Todos requieren `Authorization` con un token que pertenezca a un miembro del negocio. El `business_id` **se deriva del token**; mandar `business_id` en el body o en el path se ignora y no da error (para no filtrar información de otros negocios).

### 5.1 Negocio

```http
GET   /v1/businesses/me
PATCH /v1/businesses/me
```

```json
{
  "id": "b-uuid",
  "slug": "cafe-whisk",
  "name": "Café Whisk",
  "category": "restaurant",
  "description": " specialty coffee desde 2014",
  "logo_url": "https://cdn.suertudo.app/logos/b1c2d3e4.png",
  "brand": {
    "primary": "#1E5F74",
    "secondary": "#F2A65A",
    "font_display": "serif",
    "font_body": "sans"
  },
  "address": { "line": "Av. Corrientes 1234", "city": "CABA", "province": "Buenos Aires" },
  "plan": "pro",
  "plan_limits": {
    "active_raffles": 5,
    "active_raffles_used": 2,
    "monthly_participants": 5000,
    "monthly_participants_used": 341,
    "ai_generations": 100,
    "ai_generations_used": 12
  },
  "status": "active"
}
```

### 5.2 Miembros

```http
GET  /v1/businesses/me/members
POST /v1/businesses/me/members
PATCH /v1/businesses/me/members/{member_id}
DELETE /v1/businesses/me/members/{member_id}
```

```json
{ "email": "empleado@cafe-whisk.com", "role": "staff" }
```

El `owner` no puede ser removido ni degradado. Solo hay un `owner` por negocio.

### 5.3 Crear sorteo

```http
POST /v1/businesses/me/raffles
Idempotency-Key: …
```

```json
{
  "title": "Sorteo Mother's Day",
  "subtitle": "Un brunch para dos personas",
  "description": "Participá y ganá un brunch para dos.",
  "rules_md": "El sorteo cierra el 21/11 a las 21:00. El premio se entrega en el local con el código de canje.",
  "prize": {
    "name": "Brunch para dos",
    "description": "Incluye café de especialidad y dos medialunas.",
    "quantity": 1,
    "delivery_instructions": "Mostrá el código en el mostrador. Válido por 30 días."
  },
  "category": "restaurant",
  "number_pool_size": 1000,
  "numbers_per_ticket_min": 1,
  "numbers_per_ticket_max": 5,
  "max_tickets_per_user": 1,
  "participation_mode": "open",
  "draw_unit": "number",
  "no_winner_policy": "redraw",
  "closes_at": "2026-11-21T21:00:00-03:00",
  "draws_at": "2026-11-21T21:05:00-03:00"
}
```

Respuesta `201` con el sorteo completo en `draft`.

**Validaciones:** `closes_at` en el futuro; `draws_at > closes_at`; `number_pool_size` entre 10 y 1 000 000; `numbers_per_ticket_min ≤ max`; `closes_at` dentro de los límites del plan (máx. 90 días en `starter`).

### 5.4 Listar y editar sorteos

```http
GET   /v1/businesses/me/raffles?status=published
GET   /v1/businesses/me/raffles/{raffle_id}
PATCH /v1/businesses/me/raffles/{raffle_id}
```

El `PATCH` solo acepta campos editables según el estado:

| Estado | Editable |
|---|---|
| `draft` | Todo |
| `published` | `description`, `rules_md`, `prize.delivery_instructions`, `cover_image_url`, `theme_id`, `closes_at` (extender) |
| `closed` | `prize.delivery_instructions` |
| `drawn` | Nada (salvo `prize` por admin) |

Intentar editar algo no editable → `409 INVALID_STATE_TRANSITION` con `details.editable_fields`.

### 5.5 Publicar, cerrar, sortear, anular

```http
POST /v1/businesses/me/raffles/{raffle_id}/publish
POST /v1/businesses/me/raffles/{raffle_id}/close
POST /v1/businesses/me/raffles/{raffle_id}/draw
POST /v1/businesses/me/raffles/{raffle_id}/void
```

`publish` → `200` con el `commitment` generado. `draw` → ejecuta el sorteo de forma manual (queda auditado con `execution_mode = 'manual'`). `void` requiere `{ "reason": "…" }` y dispara la notificación de anulación.

### 5.6 Tema con IA

```http
POST /v1/businesses/me/raffles/{raffle_id}/theme/generate
```

```json
{
  "prompt": "Sorteo de fin de semana para una peluquería, algo elegante y cálido, color arena y dorado"
}
```

→ `202 Accepted` (asíncrono):

```json
{
  "data": {
    "generation_id": "gen-uuid",
    "status": "generating",
    "poll_url": "/v1/businesses/me/theme-generations/gen-uuid"
  }
}
```

```http
GET /v1/businesses/me/theme-generations/{generation_id}
```

```json
{
  "data": {
    "generation_id": "gen-uuid",
    "status": "succeeded",
    "theme_version": {
      "id": "tv-uuid",
      "version": 3,
      "source": "ai",
      "is_active": true,
      "spec": {
        "palette": {
          "primary": "#1E5F74",
          "primary_soft": "#E8F1F3",
          "secondary": "#F2A65A",
          "background": "#FBF8F3",
          "surface": "#FFFFFF",
          "text": "#2A2622",
          "text_muted": "#6B635A",
          "success": "#2E7D53",
          "danger": "#B23A2E",
          "gradient_from": "#1E5F74",
          "gradient_to": "#F2A65A"
        },
        "typography": {
          "display_family": "serif",
          "body_family": "sans",
          "display_size": 34,
          "title_size": 22,
          "body_size": 16,
          "caption_size": 13,
          "display_weight": 700
        },
        "shape": { "radius": 16, "border_width": 1 },
        "layout": { "variant": "elegant", "hero_style": "gradient", "card_style": "elevated", "cta_style": "solid" },
        "motion": { "entrance": "fade-up", "confetti_intensity": "medium" },
        "copy": {
          "title": "Sorteo Mother's Day",
          "subtitle": "Un brunch para dos personas",
          "cta_label": "Reservar mis números",
          "rules_short": "Cierra el 21/11 a las 21:00",
          "prize_label": "Brunch para dos",
          "urgency_line": "Quedan 153 números disponibles",
          "disclaimer": "El resultado se verifica públicamente en suertudo.app/verificar"
        },
        "imagery": { "style": "gradient_mesh" }
      },
      "warnings": [
        { "code": "ADJUSTED_CONTRAST", "field": "palette.text", "message": "Se ajustó el color de texto para cumplir contraste AA." }
      ]
    },
    "usage": { "model": "claude-sonnet-4-5", "tokens_in": 1420, "tokens_out": 780, "cost_usd": 0.0041, "latency_ms": 3120 }
  }
}
```

Estados posibles de la generación: `generating` → `succeeded` | `failed` | `rejected` (schema inválido dos veces) | `cached` (devuelve la versión cacheada, `theme_version` con `source: 'ai'` y `cached: true`).

**Si la generación falla**, el sorteo conserva el tema por defecto y la respuesta incluye `"fallback_applied": true` con un `theme_version` de `source: 'default'`. La IA nunca deja un sorteo sin tema.

### 5.7 Temas: listar, activar, editar

```http
GET   /v1/businesses/me/raffles/{raffle_id}/theme/versions
POST  /v1/businesses/me/raffles/{raffle_id}/theme/versions/{version}/activate
PATCH /v1/businesses/me/raffles/{raffle_id}/theme
```

El `PATCH` aplica overrides parciales sobre el tema activo:

```json
{ "palette": { "primary": "#B23A2E" }, "copy": { "cta_label": "Participar ahora" } }
```

El resultado se guarda como **nueva versión** con `source = 'manual'` y `overrides_from = <version base>`. Un override manual nunca se pierde ante una regeneración.

### 5.8 Participantes y export

```http
GET /v1/businesses/me/raffles/{raffle_id}/participants?limit=50&cursor=...
GET /v1/businesses/me/raffles/{raffle_id}/participants/export   (CSV)
GET /v1/businesses/me/raffles/{raffle_id}/stats
```

Participantes:

```json
{
  "data": [
    {
      "ticket": { "id": "t-uuid", "code": "SU-TKU4F9K", "numbers": [742, 118, 903], "status": "reserved", "created_at": "2026-11-14T18:03:11-03:00" },
      "participant": {
        "display_name": "Ana",
        "contact": { "email": "ana@example.com", "phone_e164": "+5491122334455" },
        "contact_visible": true
      },
      "prize": null
    }
  ],
  "page": { "next_cursor": null, "has_more": false, "limit": 50 }
}
```

**`contact_visible: false`** significa que el participante no consentió compartir con este negocio. El panel lo muestra explícitamente ("contacto no compartido") en vez de ocultar la fila, para que el comercio entienda la diferencia entre "no participó" y "no nos dejó contactarlo".

El export CSV devuelve un `text/csv` con `Content-Disposition: attachment`. Columnas:

```
ticket_code,numbers,status,participant_name,email,phone,consent_marketing,consent_at,participated_at,prize_status,prize_name
```

Las columnas `email` y `phone` salen **vacías** para los participantes sin consentimiento. La cabecera del CSV incluye una línea de metadatos con el sorteo, la fecha de exportación y el número de filas.

### 5.9FIDELIZACIÓN — leads

```http
GET /v1/businesses/me/leads?limit=50&cursor=...
```

```json
{
  "data": [
    {
      "participant": { "display_name": "Ana", "email": "ana@example.com", "phone_e164": "+5491122334455" },
      "consent_at": "2026-11-14T18:03:12-03:00",
      "first_participation_at": "2026-11-02T19:14:00-03:00",
      "last_participation_at": "2026-11-14T18:03:11-03:00",
      "total_participations": 4,
      "raffles": [
        { "public_id": "3f8a1c22-…", "title": "Sorteo Mother's Day", "prize_status": "winner" }
      ]
    }
  ],
  "page": { "next_cursor": null, "has_more": false, "limit": 50 }
}
```

### 5.10 Premios

```http
GET  /v1/businesses/me/prizes?status=claimed
PATCH /v1/businesses/me/prizes/{prize_id}
```

```json
{ "status": "delivered", "delivery_notes": "Entregado el 22/11, mesa 4." }
```

Transiciones válidas: `pending → claimed` (automático al reclamo del ganador), `claimed → delivered`, `pending|claimed → expired` (automático), `* → voided` (solo admin).

### 5.11 Códigos de validación

```http
POST /v1/businesses/me/validation-codes
```

```json
{ "raffle_id": "r-uuid", "quantity": 100, "prefix": "SU-V" }
```

Devuelve los códigos generados en texto plano, una sola vez. Se pueden descargar como CSV o copiar.

### 5.12 Mensaje a participantes

```http
POST /v1/businesses/me/messages
```

```json
{ "raffle_id": "r-uuid", "subject": "¡Gracias por participar!", "body": "…", "channels": ["push", "email"] }
```

Solo a participantes **de ese sorteo** con `share_marketing = true`. Máximo 1 mensaje por sorteo por semana → `429 MESSAGE_RATE_LIMITED`.

### 5.13 Métricas del negocio

```http
GET /v1/businesses/me/analytics/summary?from=2026-10-01&to=2026-11-01
```

```json
{
  "data": {
    "raffles_published": 6,
    "raffles_drawn": 4,
    "participants_unique": 289,
    "tickets_total": 412,
    "tickets_per_raffle_median": 68,
    "coverage_pct_median": 12.4,
    "prizes_delivered": 3,
    "prizes_delivery_rate": 0.75,
    "avg_time_to_claim_hours": 6.2
  }
}
```

---

## 6. Endpoints de administración

Requieren rol `platform_admin`.

```http
GET  /v1/admin/businesses?status=suspended
POST /v1/admin/businesses/{business_id}/suspend   { "reason": "…" }
POST /v1/admin/raffles/{raffle_id}/void            { "reason": "…" }
GET  /v1/admin/audit?entity_type=raffle&entity_id=…
GET  /v1/admin/system/health
```

`system/health` expone el detalle de dependencias (Postgres, cola, motor iii, proveedor de email, proveedor de push). **Solo accesible para admins autenticados.**

---

## 7. Funciones iii

La API HTTP es **una** de las entradas al sistema. La lógica de negocio vive en funciones iii, invocables tanto por HTTP como por otros workers.

### 7.1 Convenciones de nombres

```
<namespace>::<verb>_<recurso>
```

Namespaces: `identity`, `businesses`, `raffles`, `participation`, `draws`, `notifications`, `ai`, `crm`, `audit`, `admin`.

El `namespace` de un worker define dónde se registran sus funciones y dónde se resuelven sus llamadas, salvo que se indique otro explícitamente.

### 7.2 Inventario

| Función | Namespace | Descripción | Consumida por |
|---|---|---|---|
| `identity::register_user` | identity | Crea un participante | HTTP |
| `identity::login` | identity | Autentica | HTTP |
| `identity::refresh_session` | identity | Rota el refresh token | HTTP |
| `identity::get_user` | identity | Devuelve un usuario por id | Varios |
| `identity::authorize` | identity | Middleware RBAC: valida sesión y rol | Middleware de iii |
| `businesses::create_business` | businesses | Onboarding de comercio | HTTP |
| `businesses::get_business` | businesses | Lee un negocio | HTTP |
| `businesses::update_business` | businesses | Actualiza datos y tokens de marca | HTTP |
| `businesses::check_plan_limit` | businesses | Verifica límites del plan | raffles, participation |
| `businesses::list_members` | businesses | Lista miembros | HTTP |
| `businesses::invite_member` | businesses | Invita un miembro | HTTP |
| `raffles::create_raffle` | raffles | Crea un sorteo en `draft` | HTTP |
| `raffles::update_raffle` | raffles | Actualiza según el estado | HTTP |
| `raffles::publish_raffle` | raffles | Genera semilla + commitment, pasa a `published` | HTTP |
| `raffles::close_raffle` | raffles | Cierra la participación | HTTP, cron |
| `raffles::void_raffle` | raffles | Anula | HTTP, admin |
| `raffles::get_raffle` | raffles | Lee un sorteo | HTTP |
| `raffles::list_public_raffles` | raffles | Feed público | HTTP |
| `raffles::list_business_raffles` | raffles | Sorteos de un negocio | HTTP |
| `raffles::get_raffle_stats` | raffles | Métricas | HTTP |
| `participation::reserve_ticket` | participation | Reserva números (transaccional) | HTTP |
| `participation::cancel_ticket` | participation | Cancela y libera números | HTTP |
| `participation::list_user_tickets` | participation | Participaciones del usuario | HTTP |
| `participation::list_raffle_participants` | participation | Participantes con consentimiento | HTTP |
| `participation::generate_validation_codes` | participation | Genera códigos de un solo uso | HTTP |
| `draws::commit_seed` | draws | Publica el commitment al publicar | raffles |
| `draws::execute_draw` | draws | Ejecuta el sorteo (idempotente) | cron, HTTP |
| `draws::close_due_raffles` | draws | Cierra sorteos vencidos | cron |
| `draws::verify_draw` | draws | Verificación pública | HTTP |
| `draws::settle_prizes` | draws | Marca premios entregados | cron |
| `notifications::enqueue` | notifications | Encola una notificación | Varios |
| `notifications::dispatch` | notifications | Envía (push, email, in-app) | Cola |
| `notifications::expire_prizes` | notifications | Expira premios no reclamados | cron |
| `ai::generate_theme` | ai | Genera el `theme_spec` | HTTP |
| `ai::validate_theme` | ai | Valida schema, contraste y anclas | ai, http |
| `ai::apply_theme_overrides` | ai | Aplica overrides manuales | HTTP |
| `crm::export_participants` | crm | Genera el CSV | HTTP |
| `crm::list_leads` | crm | Panel de fidelización | HTTP |
| `audit::write` | audit | Escribe en la auditoría | Todos |

### 7.3 Triggers

| Tipo | Función | Config | Disparador |
|---|---|---|---|
| `http` | `api::*` | `{ "api_path": "/v1/…", "http_method": "GET" }` | Petición HTTP |
| `cron` | `draws::close_due_raffles` | `{ "expression": "0 * * * * *" }` | Cada minuto |
| `cron` | `draws::execute_draw` | `{ "expression": "0 * * * * *" }` | Cada minuto |
| `cron` | `notifications::expire_prizes` | `{ "expression": "0 3 * * * *" }` | 03:00 diario |
| `queue` (con nombre) | `notifications::dispatch` | `queue_configs: { "notifications": { "type": "standard", "concurrency": 10, "max_retries": 3, "backoff_ms": 1000 } }` | Mensaje encolado |
| `queue` (con nombre) | `crm::export_participants` | `queue_configs: { "exports": { "type": "standard", "max_retries": 3 } }` | Export encolado |
| `durable:subscriber` | `raffles::broadcast_event` | `{ "topic": "raffle-events" }` | Cambio de estado del sorteo |
| `state` | `raffles::invalidate_cache` | `{ "scope": "raffle" }` | Cambio de estado del sorteo |

**El cron de `execute_draw` corre cada minuto.** Un sorteo con `draws_at` vencido se sortea dentro de esa ventana (p95 ≤ 60 s, `NFR-PL-05`).

### 7.3.1 Los tipos de trigger no vienen con el engine

**Ningún tipo de trigger de cola ni de cron está incluido en el engine.** Cada uno necesita su worker del registro, y sin él el trigger queda en `PENDING` para siempre:

| Tipo de trigger | Worker del registro que lo provee | Qué pasa si falta |
|---|---|---|
| `http` | `http` (v0.22.7) | `Trigger type http is not registered` |
| `durable:subscriber` | `queue` (v0.21.22) | `Trigger type durable:subscriber is not registered` |
| `cron` | `cron` (v0.21.26) | `Trigger type cron is not registered` |

El engine lo dice explícitamente en el log, con el comando exacto:

```
[WARN] iii::trigger Trigger ... was NOT activated: trigger type cron is not registered.
It will be registered automatically when the trigger type becomes available.
If this persists, the cron worker is missing — run:
iii trigger -n <compose-daemon-namespace> compose::add worker=cron
```

**Regla para el CI:** el smoke test tiene que assertar que el log del engine **no** contiene `is not registered` para ningún tipo de trigger. Un health check que solo mira un `200` no lo detecta: el sistema queda con las funciones registradas y sin ninguna ruta, que es exactamente el modo de fallo silencioso que el spike de iii documentó.

### 7.3.2 Las dos clases de cola de iii

| | Cola con nombre (`TriggerAction::Enqueue`) | Pub/sub (`durable:subscriber`) |
|---|---|---|
| Distribución | **Reparte** el trabajo entre los consumidores | **Cada** suscriptor del topic en el namespace recibe **todos** los mensajes |
| Tipo en la consola | `function_queue` | `builtin` |
| Uso en Suertudo | Notificaciones y exports: son trabajos que se reparten y necesitan reintentos | Eventos de cambio de estado del sorteo: son broadcasts |
| Reintentos | 3 intentos, backoff exponencial (1 s, 2 s), luego DLQ | Los del suscriptor |

Las notificaciones usan la **cola con nombre**, porque el trabajo se reparte entre workers y cada mensaje necesita su propio ciclo de reintentos y DLQ. El outbox publica así:

```rust
// Publicar: encolar un trabajo
let receipt = worker.trigger(TriggerRequest {
    function_id: "notifications::dispatch".into(),
    payload: json!({ "notification_id": n_id }),
    action: Some(TriggerAction::Enqueue { queue: "notifications".into() }),
    timeout_ms: None,
}).await?;
```

### 7.3.3 Definir una named queue

Una named queue **tiene que existir antes de encolar**. Si no, el enqueue falla:

```
enqueue_error: queue 'notifications' is not defined; call queue::define before enqueueing
```

La definición se hace con `queue::define` o declarando `queue_configs` en la configuración del worker `queue`.

```json
{
  "queue": "notifications",
  "config": {
    "type": "standard",
    "concurrency": 10,
    "max_retries": 3,
    "backoff_ms": 1000
  }
}
```

**El schema de `config` es estricto y rechaza campos desconocidos.** Los campos aceptados son exactamente: `max_retries`, `concurrency`, `timeout_ms`, `type`, `message_group_field`, `backoff_ms`, `poll_interval_ms`, `redeliver_on_engine_restart`, `max_priority`, `priority_field`.

Tres trampas que costaron tiempo:

| Trampa | Consecuencia |
|---|---|
| `type` solo admite `standard` o `fifo`. `concurrent` no existe en este schema. | `handler error: queue 'X' type must be either 'standard' or 'fifo'` |
| `type: fifo` **exige** `message_group_field`. | `handler error: queue 'X' with type 'fifo' requires message_group_field` |
| `function_id` **no** es parte de la config. | La función destino viaja con cada mensaje, en `TriggerAction::Enqueue` o en `engine::queue::enqueue`. |

**La función destino viaja con el mensaje, no con la cola.** Una named queue define el transporte (tipo, concurrencia, reintentos); el `function_id` va en cada enqueue:

```rust
// Encolar: la función destino va en la petición
worker.trigger(TriggerRequest {
    function_id: "notifications::dispatch".into(),
    payload: json!({ "notification_id": n_id }),
    action: Some(TriggerAction::Enqueue { queue: "notifications".into() }),
    timeout_ms: None,
}).await?;
```

`TriggerAction::Enqueue` **funciona en el engine 0.24.6**: el corte `QueueEnqueuer` está presente y el engine enruta la acción a través del provider `engine::queue::enqueue` del worker `queue`. Verificado en el spike.

### 7.4 Ejemplo de registro

```rust
use iii_sdk::{register_worker, InitOptions, RegisterFunction};
use iii_sdk::protocol::RegisterTriggerInput;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, JsonSchema)]
struct ReserveInput {
    raffle_id: String,
    user_id: String,
    numbers: Option<Vec<i32>>,
    count: Option<i32>,
}

#[derive(Serialize, JsonSchema)]
struct ReserveOutput {
    ticket_id: String,
    code: String,
    numbers: Vec<i32>,
}

async fn reserve(input: ReserveInput) -> Result<ReserveOutput, iii_sdk::Error> {
    // La transacción, la validación de unicidad y el outbox viven en el dominio.
    todo!()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let worker = register_worker(
        &std::env::var("III_URL").unwrap_or_else(|_| "ws://127.0.0.1:49134".into()),
        InitOptions::default(),
    );

    worker.register_function(
        "participation::reserve_ticket",
        RegisterFunction::new_async(reserve)
            .description("Reserva N números de un sorteo para un participante"),
    );

    worker.register_trigger(RegisterTriggerInput::new(
        "http",
        "participation::reserve_ticket",
        json!({ "api_path": "/v1/raffles/:public_id/tickets", "http_method": "POST" }),
    ))?;

    Ok(())
}
```

El `request_format` / `response_format` se derivan automáticamente del `JsonSchema` de los tipos, y por eso el SDK los expone en `engine::functions::list`: un agente de IA puede descubrir las funciones disponibles y sus esquemas **en caliente**, sin leer el código. Esa es la propiedad que hace que el sistema sea operable por agentes.

### 7.5 Middleware de autorización

El middleware `identity::authorize` se registra en el puerto RBAC de iii y se ejecuta **antes** de cada función:

```rust
use iii_sdk::structs::MiddlewareFunctionInput;

worker.register_function(
    "identity::authorize",
    RegisterFunction::new(|input: MiddlewareFunctionInput| {
        // input.context trae el auth context de la sesión
        // Se puede mutar input.payload para inyectar business_id derivado del token
        Ok(input.payload)
    }),
);
```

**Regla dura:** el middleware inyecta el `business_id` derivado del token en el payload. Las funciones de negocio **nunca** leen el `business_id` del body o de los path params. Si no viene inyectado, la función falla con `403`.

---

## 8. Webhooks y tiempo real

### 8.1 WebSocket (canales de iii)

El motor soporta canales con streaming de datos binarios y texto. Se usan para:

| Canal | Contenido | Consumidor |
|---|---|---|
| `raffle:{public_id}` | Cambios de estado, conteo de números, resultado | App móvil, panel web |
| `business:{business_id}` | Cambios en los sorteos del negocio | Panel web |
| `prize:{prize_id}` | Cambios de estado del premio | App móvil |

```rust
let writer = iii_sdk::channel::ChannelWriter::new(&engine_ws_base, &channel_ref).await?;
writer.send_message(json!({ "type": "raffle.state", "status": "drawn" }).to_string()).await?;
```

En el MVP, la actualización de la UI se puede resolver con **polling** (el feed cada 60 s, el detalle del sorteo cada 15 s). El WebSocket es una optimización de la Fase 2, no un requisito del MVP.

### 8.2 Webhooks salientes (Fase 2)

Opcionalmente, un negocio puede pedir que se le notifique por webhook cuando:

- Su sorteo se publica (`raffle.published`)
- El sorteo se sortea (`raffle.drawn`)
- Hay un ganador (`raffle.winner_claimed`)

```http
POST https://cliente.com/webhooks/suertudo
X-Suertudo-Signature: sha256=<hmac_del_body_con_el_secret>
```

La firma se verifica contra el secreto del negocio. Los webhooks se reintentan 3 veces con backoff exponencial.

---

## 9. Checklist de implementación

- [ ] Todos los endpoints documentados implementados y cubiertos por tests de contrato.
- [ ] Catálogo de errores de §1.4 completo y consistente con las respuestas reales.
- [ ] `Idempotency-Key` funcionando en todas las escrituras de negocio, con test de replay.
- [ ] Paginación por cursor sin duplicados bajo escrituras concurrentes.
- [ ] `Retry-After` presente en todos los `429`.
- [ ] Endpoint `/public/raffles/{id}/verify` produce exactamente la forma de §2.4.
- [ ] `identity::authorize` inyecta `business_id` y ninguna función lo toma del body.
- [ ] Test de aislamiento: token del business A contra rutas de B → `404`.
- [ ] Una generación de IA fallida deja el sorteo con tema por defecto, sin error visible para el usuario final.
- [ ] CSV de export con columnas de contacto vacías cuando no hay consentimiento (test automatizado).
- [ ] Cola `notifications` con reintentos y DLQ configurada y verificada.
