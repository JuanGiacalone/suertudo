# La toma de control de un dispositivo, verificada por SMS

> **Qué es esto:** el diseño completo del flujo que permite que un participante registre un token de dispositivo push que **ya pertenece a otra cuenta**, sin que eso sea ni un error ni un secuestro de notificaciones.
>
> El proveedor de SMS todavía no está decidido. **El contrato que tiene que cumplir está en [`contrato-proveedor-sms.md`](./contrato-proveedor-sms.md).**
>
> Documentos relacionados: [`api-contract.md`](./api-contract.md) §3.8 · [`data-model.md`](./data-model.md) §3.28 · [`roadmap.md`](./roadmap.md) `TSK-F0-10` y `TSK-F0-11` · [`architecture.md`](./architecture.md)

---

## 1. El problema

`push_devices.token` es **único de forma global**. El token de un dispositivo push identifica **un teléfono**, no a una persona: la misma persona, con dos cuentas, tiene el mismo token en las dos.

Antes de este trabajo, el registro de un token ajeno se rechazaba:

```
403 RBAC_FORBIDDEN — "the device token belongs to another participant"
```

Eso era correcto para la seguridad del dueño anterior, y tenía dos costos que no se pueden tolerar:

| Costo | Qué pasaba |
|---|---|
| **El mensaje no decía nada del teléfono** | "Tu rol no alcanza" es la respuesta correcta para un permiso, y la equivocada para un teléfono. El participante no tenía forma de saber qué hacer. |
| **El push se perdía en silencio** | La misma persona abría una segunda cuenta en el mismo teléfono y la cuenta nueva no recibía **nada**. Sin error, sin aviso: solo silencio. |

**La decisión de producto:** cuando el token se repite, se abre un **flujo de verificación**. El token **no cambia de dueño hasta que alguien verifica**.

---

## 2. La decisión de seguridad

De todo el diseño, esto es lo único que no se negocia:

> **El SMS va al teléfono en ficha del participante que hoy es dueño del token. No al del participante que lo reclama.**

Si el SMS fuera al del reclamante, un atacante que copió el token —un string corto que viaja en cada push— recibiría **su propio código** y la transferencia funcionaría. El atacante se verificaría a sí mismo.

Mandándolo al dueño actual:

| Escenario | Qué pasa |
|---|---|
| **El mismo teléfono, dos cuentas** (legítimo) | El SMS llega al teléfono que los dos comparten. El código es correcto y el token pasa a la cuenta nueva. |
| **Un atacante copió el token** | El SMS llega al dueño real, que no es el atacante. El atacante nunca recibe el código, la verificación vence y **el token no se mueve**. |
| **El dueño no quiere** | No hace nada. La verificación vence y el token se queda. **El dueño puede negarse.** |

El dueño actual es, además, **la única persona a la que el teléfono puedeándole**: nadie más puede recibir un SMS en ese aparato.

**El teléfono sale de la fila de la verificación, no de la petición.** `device_takeover_phone` devuelve el `phone_e164` del `current_owner_id` registrado, de modo que **ningún parámetro de la petición puede redirigir el mensaje** a otro participante.

---

## 3. El flujo

```
Participante B                  Base de datos                  Participante A (dueño)
      │                              │                                │
      │ POST /v1/me/devices          │                                │
      │ { token: "abc123" }          │                                │
      ├─────────────────────────────>│                                │
      │                    request_device_takeover                   │
      │                    ¿el token es de B? NO                    │
      │                    → crea la verificación pendiente        │
      │                    → devuelve el id                        │
      │<─────────────────────────────┤                                │
      │                              │                                │
      │ 202 { verification_id,       │                                │
      │       status, expires_at }   │                                │
      │                              │                                │
      │                       device_takeover_phone                  │
      │                       → el teléfono de A                    │
      │                              │                                │
      │                    SMS ──────────────────────────> A        │
      │                              │        "ingresá el código 123456"│
      │                              │                                │
      │ B recibe el código de la app│   (lo lee del SMS)            │
      │ POST /v1/me/devices/verifications/{id}/confirm              │
      │ { code: "123456" }           │                                │
      ├─────────────────────────────>│                                │
      │                    confirm_device_takeover                   │
      │                    digest(code) == code_hash [x]              │
      │                    → el token pasa a B                      │
      │<─────────────────────────────┤                                │
      │ 200 { verified: true, device }│                                │
```

### 3.1 Las respuestas de `POST /v1/me/devices`

| Código | Cuándo | Cuerpo |
|---|---|---|
| `201` | El token estaba libre, o ya era del participante | El dispositivo registrado |
| `202` | El token es de otro participante | `{ "verification_id": "…", "status": "pending", "expires_at": "…" }` |
| `409 DEVICE_OWNER_UNVERIFIABLE` | El dueño actual no tiene teléfono en ficha | No hay a quién dirigir el SMS. **El token no se mueve.** |

### 3.2 Las respuestas de la confirmación

| Código | Cuándo |
|---|---|
| `200` | El código era correcto y el dispositivo cambió de dueño |
| `400 DEVICE_CODE_INVALID` | El código no era. **El intento se cuenta.** |
| `409 DEVICE_VERIFICATION_EXPIRED` | La verificación venció |
| `403 RBAC_FORBIDDEN` | La verificación no es de quien la confirma |

### 3.3 El código

| Propiedad | Valor | Por qué |
|---|---|---|
| Dígitos | 6 | Se dicta por teléfono. |
| Vida | 10 minutos | Suficiente para leer un SMS, poco para usarlo. |
| Intentos | 5 | El sexto intento no llega a la base. |
| Almacenamiento | **sólo su SHA-256** | El texto claro existe en el SMS y en la petición de quien lo recibió, y en ningún otro lado. |
| Origen | CSPRNG, con muestreo por rechazo | Un `rand` sesgado reduce el espacio y con él la seguridad del código. |

### 3.4 El mensaje

> Suertudo: alguien está intentando registrar en su cuenta el dispositivo que ya tenés registrado en otra. Para autorizar el traslado, ingresá el código **123456** en la app. Vence en 10 minutos. Si no reconocés el dispositivo, no hagas nada: el registro no se mueve sin tu autorización.

Dice las cuatro cosas que tiene que decir —que alguien está registrando un dispositivo que ya es tuyo, que el código autoriza el traslado, el código, y que vence— y **le sirve a alguien que no sabe que tiene una cuenta abierta en otro teléfono**: la última frase es la que evita el pánico y le da la opción de negarse.

### 3.5 El cierre de sesión libera los dispositivos

**Un dispositivo es de una sesión, no de una persona.** Si al cerrar sesión no se liberan, el caso legítimo más común —la misma persona, dos cuentas— sigue chocando con la verificación.

`release_user_devices` **borra** las filas y no las desactiva: una fila desactivada sigue siendo dueña del token, y el `UNIQUE` global no lo liberaría.

---

## 4. El modelo de datos

La tabla `device_verifications` (migración `0042_device_takeover.sql`):

```sql
CREATE TABLE device_verifications (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    device_token      text NOT NULL,
    current_owner_id  uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    claimant_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash         bytea NOT NULL,
    status            text NOT NULL DEFAULT 'pending'
                      CHECK (status IN ('pending','confirmed','expired','refused','cancelled')),
    attempts          integer NOT NULL DEFAULT 0,
    expires_at        timestamptz NOT NULL,
    confirmed_at      timestamptz,
    created_at        timestamptz NOT NULL DEFAULT now()
);
```

| Decisión | Razón |
|---|---|
| **`current_owner_id` aparte de `claimant_id`** | El SMS va al dueño **actual**, no al reclamante. Es la columna que hace segura la transferencia. |
| **La transferencia se ata al dueño registrado** | Una verificación vieja no puede robarle el dispositivo a la cuenta que lo tiene hoy. |
| **Índice único parcial `(device_token, claimant_id) WHERE status='pending'`** | El flujo no apila verificaciones del mismo token. Una carrera entre dos reclamantes da un `23505` mapeado. |
| **`code_hash` es `bytea`** | El hash es binario, como el resto de los hashes de la base. |

### 4.1 Las cinco funciones

| Función | Qué hace |
|---|---|
| `request_device_takeover(token, platform, app_version, user_id, code_hash, expires_at)` | Registra el token, o abre la verificación. Devuelve `NULL` si quedó a nombre del participante, o el id de la verificación pendiente. |
| `confirm_device_takeover(verification_id, user_id, code)` | Confirma con el código. `true` si el token cambió de dueño. |
| `release_user_devices(user_id)` | Libera los dispositivos. Devuelve cuántos. |
| `device_takeover_phone(verification_id, user_id)` | El teléfono del dueño actual, para dirigirle el SMS. `NULL` si no tiene. |
| `expire_device_verifications()` | Marca como vencidas las pendientes. El único productor del estado `expired`. |

Las cinco son `SECURITY DEFINER` con `search_path` fijo, y **atan al que llama**.

---

## 5. Dos detalles que la base forzó

### 5.1 Un rechazo no puede ser una excepción

Un código incorrecto **no** puede responderse con un `RAISE EXCEPTION`: la excepción aborta la transacción, y el contador de intentos se va con el rollback. La función devuelve **`false`**, que es un resultado, y **la transacción se confirma**.

Por eso la llamada que devuelve `false` tiene que confirmarse en la transacción que la envuelve, y no revertirse. Es un detalle pequeño que, si se olvida, deja el flujo sin límite de intentos.

### 5.2 El teléfono exige una función

`users.phone_e164` está protegido por el RLS de `users`: un participante sólo lee su propia fila. Para que el reclamante pueda conocer el teléfono del dueño —y sólo para dirigirle el SMS— hace falta una función `SECURITY DEFINER` que lo devuelva, y que devuelva `NULL` cuando el dueño no tiene teléfono.

---

## 6. Qué está verificado

| Garantía | Test |
|---|---|
| Un token libre se registra sin verificación | `a_free_device_is_registered_without_a_verification` |
| Un token repetido abre una verificación | `a_repeated_token_opens_a_verification` |
| **El SMS va al dueño del token, no al reclamante** | `the_sms_goes_to_the_owner_of_the_token_and_not_to_the_claimant` |
| El código correcto transfiere el dispositivo | `a_correct_code_transfers_the_device` |
| El código incorrecto tiene su propio código y cuenta el intento | `a_wrong_code_is_refused_with_its_own_code` |
| Una verificación vencida tiene su propio código | `an_expired_verification_is_refused_with_its_own_code` |
| Un dueño sin teléfono no se puede verificar | `a_device_without_a_phone_cannot_be_verified` |
| El cierre de sesión libera los dispositivos | `the_logout_releases_the_devices` |
| El worker se niega a arrancar fuera de development con el emisor de log | `the_worker_refuses_to_start_in_production_with_a_development_sender` |

Los tests de la base (`services/db/tests/device_takeover.rs`, 14 tests) cubren el otro lado: que la función ata al que llama, que un tercero no confirma, que un código incorrecto no mueve el dueño, que una verificación vencida no se confirma, que una solicitud repetida **reutiliza** la pendiente, que una verificación vieja no puede robar un dispositivo que cambió de manos, y que un participante no ve las verificaciones de otro.

Todos los tests de aislamiento corren **con el rol de la aplicación**, que no es dueño del esquema: uno que conecta como dueño esquiva todas las políticas y no demuestra nada.

---

## 7. Lo que queda

| Tarea | Qué es |
|---|---|
| [`TSK-F0-11`](./roadmap.md) | El proveedor de SMS real. **Bloquea el despliegue fuera de development.** El contrato está en [`contrato-proveedor-sms.md`](./contrato-proveedor-sms.md). |
| `TSK-F0-09` | El rol de aplicación en el despliegue necesita `GRANT SELECT ON device_verifications` y `EXECUTE` en las cinco funciones. |
| Fase 1 | `release_user_devices` devuelve **cuántos** dispositivos liberó, no **cuáles**: el worker no puede desregistrarlos de FCM o APNs. Cuando exista la integración con el proveedor de push, hará falta una segunda función que devuelva los tokens. |

### 7.1 Una decisión que quedó tomada y conviene revisar

**Un dueño sin teléfono en ficha no se puede verificar**, así que el token se queda donde está y el reclamante recibe `409 DEVICE_OWNER_UNVERIFIABLE`. No hay canal alternativo.

La alternativa sería un segundo canal —correo, o una notificación push al dueño—, pero ambos tienen el mismo problema: **si el dueño no puede ser alcanzado, el token no se puede mover con seguridad**. La decisión es coherente: la seguridad manda, y el mensaje al cliente dice exactamente qué pasó.
