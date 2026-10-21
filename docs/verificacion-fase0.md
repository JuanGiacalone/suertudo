# Verificación de Fase 0 — el registro

> **Qué es esto:** el registro de las cuatro rondas de verificación independiente que /*<<< el esquema y la identidad. No es un informe de resultados: es **el catálogo de los defectos que se encontraron y de las reglas que nacieron de ellos**.
>
> Por qué existe: cada defecto de esta lista estuvo antes en producción de algún sistema real. Un agente que retome el repo y no lo lea va a repetirlos.
>
> Documentos relacionados: [`PRD.md`](./PRD.md) · [`architecture.md`](./architecture.md) · [`data-model.md`](./data-model.md) · [`api-contract.md`](./api-contract.md) · [`spike-iii-health.md`](./spike-iii-health.md)

---

## 1. Resumen

| | |
|---|---|
| **Unidad revisada** | `services/db/` (el esquema) y `services/identity/` (autenticación) |
| **Rondas de verificación** | 4, cada una independiente, cada una con su propio verificador |
| **Blockers encontrados** | 5 + 3 + 0 + 0 |
| **Hallazgos encontrados** | 11 + 6 + 6 + 2 |
| **Tests al terminar** | 106, todos verdes |
| **Veredicto final** | **PASS WITH FINDINGS** — sin blockers abiertos |
| **Rama** | `feat/fase0-cimientos` |

La cuarta ronda no encontró blockers nuevos. Esa es la señal de que la ronda anterior cerró la clase y no sólo los casos sueltos.

---

## 2. El patrón raíz

Todo lo que se encontró en la primera y la segunda ronda era **la misma clase**, repetida:

> Una política de escritura que dice `OR user_id = current_user_id()` sobre una tabla que también tiene `business_id` **no está atada al negocio**. Se ve inocente en cada tabla del esquema y abre escritura cruzada en todas.

Medido, con el rol de aplicación (`suertudo_test_app`, que no es dueño del esquema) y el contexto de un participante:

```sql
-- el participante se hace dueño de un negocio ajeno
INSERT INTO business_members (business_id, user_id, role, status)
VALUES (<negocio de B>, <yo>, 'owner', 'active');   -- ACEPTADO

-- un ticket fantasma en el sorteo de otro negocio
INSERT INTO tickets (raffle_id, business_id, user_id, code, numbers_count)
VALUES (<sorteo de B>, <B>, <yo>, 'PHANTOM1', 5);    -- ACEPTADO, con CERO ticket_numbers

-- un premio fantasma con el código de canje elegido por el atacante
INSERT INTO prizes (raffle_id, draw_id, business_id, user_id, prize_name, claim_code)
VALUES (<sorteo de B>, <sorteo de B>, <B>, <yo>, 'UN MILLON', 'CLAIM-123');  -- ACEPTADO
```

**La corrección:** toda política de escritura de una tabla con `business_id` queda atada al dueño de la fila. Los caminos que legítimamente cruzan el límite son funciones `SECURITY DEFINER` de cuerpo fijo que **atan al que llama**: una función que muta una fila de tenant sin comprobar quién llama es una escritura cruzada sin límite.

---

## 3. Lo que encontró cada ronda

### Ronda 1 — 5 blockers

| # | Defecto | Por qué importa | Corrección |
|---|---|---|---|
| B1 | Las políticas de RLS no tenían `WITH CHECK` | Sin `WITH CHECK` una política **cae a `USING`** para `INSERT` y `UPDATE`: las ramas públicas de lectura (`raffles.status='published'`, `draws.seed_revealed=true`) se convertían en permiso de escritura. Un tenant insertaba un sorteo publicado en el negocio de otro, y un **resultado forjado** en el sorteo de otro. | `WITH CHECK` explícito en toda política de escritura. |
| B2 | `tickets_business_match_trg` era vacío | El trigger hacía `SELECT business_id FROM raffles`, y esa lectura está filtrada por las políticas de `raffles`. Con el contexto de A, el subquery devolvía `NULL`, y `NULL <> 'A'` es `NULL`: **el trigger no levantaba**. La invariante era vacía exactamente en la configuración que el despliegue exige. | `SECURITY DEFINER` con `search_path` fijo, **más** una FK compuesta `(raffle_id, business_id)`: la garantía que sobrevive a cualquier cambio del trigger. |
| B3 | `DELETE /v1/me` fallaba con un participante que tenía tickets | `UPDATE tickets SET user_id = NULL` violaba la política. `FR-ID-05` dice exactamente eso: un usuario con participaciones. | `anonymise_user_tickets(uuid)`, una función `SECURITY DEFINER`. |
| B4 | `PATCH /v1/me` descartaba las preferencias de notificación | El `PATCH` respondía `200` y guardaba los valores por defecto. `api-contract.md` §3.6 lista las tres preferencias. | Se escribe la columna y se lee de vuelta. |
| B5 | Un reuso detectado en concurrencia no mataba la familia | El token presentado se revocaba y **después** se insertaba el sucesor: una rotación concurrente podía insertar su sucesor después de la revocación, y quedaba vivo. `FR-ID-04` pide que muera la familia entera. | La familia pasa a ser **una fila con estado** (`token_families.revoked_at`). Un trigger rechaza el sucesor de una familia muerta. |

### Ronda 2 — 3 blockers nuevos

Salieron de la corrección de la ronda 1: al atar las políticas, aparecieron caminos que la aplicación tenía que usar.

| # | Defecto | Corrección |
|---|---|---|
| N1 | Un participante podía hacerse `owner` de cualquier negocio | La rama `user_id` sale de la política; el alta es `register_business(...)`. |
| N2 | La misma rama en `tickets`, `prizes` y `consent_records` | `reserve_ticket(...)`, `create_prize(...)`, `record_consent(...)`. |
| N3 | `apply_reservation_counters` era una escritura cruzada sin límite: no era idempotente ni comprobaba quién llamaba. Medido: 3 llamadas movieron los contadores de otro sorteo de 0 a 9 con cero números reales. | **Recalcula** desde la verdad en vez de incrementar, y ata al dueño del ticket. Un contador recalculado no puede desviarse; uno incrementado puede ser movido por cualquiera. |

### Ronda 3 — 0 blockers, 6 hallazgos

| # | Hallazgo | Corrección |
|---|---|---|
| F-1 | Dos rechazos sin `ERRCODE` → SQLSTATE `P0001` → `500` en vez de `403` | Todos los rechazos llevan un SQLSTATE mapeado, y un test recorre `pg_proc` para que la clase no vuelva. |
| F-2 | Un participante podía escribir `status`, `deleted_at`, `email` y `password_hash` de su propia fila: **se levantaba la moderación y resucitaba una cuenta borrada** | Columnas de plataforma congeladas, con la excepción de la baja. |
| F-3 | `register_push_device` re-apuntaba el token de dispositivo de otro participante. **Alcanzable**: el endpoint toma el token del cuerpo. | El conflicto sólo actualiza si la fila ya es del mismo participante. |
| F-4 | `bind_magic_link_user` ataba un enlace consumido a un usuario arbitrario | Ata al que llama y comprueba que el correo del enlace es el de la cuenta. |
| F-5 | El test de catálogo tenía puntos ciegos: se saltaba dos políticas enteras y no cubría `ticket_numbers` | Cubre las dos excepciones con su expresión exacta. |
| F-6 | Un tenant podía escribir los contadores de su propio sorteo | Un trigger los rechaza. |

### Ronda 4 — 0 blockers, 2 hallazgos

| # | Hallazgo | Estado |
|---|---|---|
| N-1 | El trigger de los contadores prometía el contexto de sistema en su comentario y no lo implementaba | **Corregido** (migración `0041`), con un test verificado que falla sin el fix. |
| N-2 | El conflicto del token de dispositivo responde `403 RBAC_FORBIDDEN` ("tu rol no alcanza") cuando la verdad es "ese teléfono es de otro". El escenario real: el mismo teléfono con una segunda cuenta, y **el push se pierde en silencio**. | **Decisión de producto pendiente** → `TSK-F0-10`, con los dos caminos posibles escritos. |

---

## 4. Las siete reglas que nacieron de esto

Están en `AGENTS.md` §9. Acá está el porqué de cada una, que es lo que no se puede deducir de la regla.

| Regla | Por qué existe |
|---|---|
| Toda política de escritura lleva `WITH CHECK` | Sin él cae al `USING` y las ramas públicas de lectura se vuelven permiso de escritura. |
| Una tabla con `business_id` solo la escribe su dueño | La rama `user_id = current_user_id()` no está atada al negocio. |
| Una función `SECURITY DEFINER` ata al que llama | Si no, es una escritura cruzada sin límite. |
| Un trigger que lee otra tabla es `SECURITY DEFINER` | Su lectura está filtrada por las políticas de esa tabla, y la invariante se vuelve silenciosa. |
| El contexto de sistema tiene un marcador propio (`'system'`) | Un `SET LOCAL` deja un placeholder vacío a nivel de sesión: una conexión reusada reportaba el contexto de sistema para siempre. Medido: **1258 participantes visibles sin contexto**. |
| El rol de la aplicación no es dueño del esquema | Postgres no le aplica las políticas de RLS al dueño de la tabla. Y los tests de aislamiento tienen que correr con ese rol: uno que conecta como dueño no demuestra nada. |
| Todo rechazo lleva un SQLSTATE mapeado | `P0001` no lo traduce nadie y el cliente recibe `500` en vez de `403`. |

---

## 5. Lo que la verificación NO pudo comprobar

Honestidad sobre los límites del método:

| No comprobado | Por qué |
|---|---|
| El flujo de magic link por HTTP contra un cliente real | No hay servidor en el repo todavía: los endpoints se probaron a nivel de handler. |
| Dos refresh HTTP **simultáneos** | La garantía de orden se probó a nivel de SQL (`55000` al insertar en una familia revocada) y el reuso a nivel de handler, no con dos peticiones simultáneas. |
| La paridad de producción de los roles | El rol de aplicación se crea en el despliegue; en los tests es `suertudo_test_app`. |
| El correo | No hay proveedor de correo todavía (`TSK-F0-08`). |

---

## 6. El método, para la próxima vez

La parte más transferible de este trabajo no son los defectos: es **cómo se encontraron**.

1. **Reproducir el ataque, no leer el código.** Cada defecto se comprobó con el comando que lo explota, con el rol de la aplicación y el contexto del atacante.
2. **Quitar la garantía y ver el test fallar.** Un test que pasa sin la garantía que dice probar no es un test. Cada fix se verificó así, y el mensaje de falla exacto quedó registrado.
3. **Un test de catálogo sobre el esquema real.** Leer `pg_policy` y `pg_proc` desde el test es lo que detecta una política editada a mano. Un test que asserta sobre el código no ve la base.
4. **Corregir la clase, no el caso.** La primera ronda arreglaba ataques sueltos; la segunda volvió a encontrarlos porque la clase seguía viva. La tercera la cerró, y la cuarta no encontró nada nuevo.
5. **El verificador no toca el repositorio.** Trabaja en una copia aparte y con una base aislada. Un revisor que arregla lo que encuentra no está revisando.

### 6.1 Una trampa del propio smoke test

El smoke test tenía esta forma:

```bash
if echo "$TEST_OUT" | grep -q 'test result: FAILED'; then
  fail "los tests fallaron"
fi
pass "los tests están verdes"
```

**Una comprobación que pasa con salida vacía no es una comprobación.** Cuando `cargo` se negaba a correr (por un error de workspace, no por un test roto), la salida no contenía la palabra `FAILED`, el `if` no se activaba y el script reportaba **PASS** sobre un paso que nunca se ejecutó.

Ahora cada sección exige **la marca del éxito**: un número, un código, un cuerpo. Y hay un paso entero que verifica que el `cargo test` corrió de verdad, comparando el conteo de tests con el esperado.

La regla: **un check que sólo busca la ausencia de un error no puede distinguir "todo bien" de "nada corrió"**.

### 6.2 El error que la promoción del spike destapó

Al mover el golden test de la derivación a su crate propio (`services/draws`), el spike quedó fuera del workspace de Rust y `cargo` se negó a compilarlo desde su propio directorio. Llevaba así desde que se creó el `Cargo.toml` raíz, y el smoke test no lo detectó por el motivo de §6.1.

Lo que se hizo: el spike quedó **excluido** del workspace (tiene su propio lockfile y su propio pin de `iii-sdk`, y no debe heredar las versiones del proyecto), y el smoke test pasó a exigir que el paso corra.

---

## 7. Qué queda abierto de esta fase

| Tarea | Qué es |
|---|---|
| `TSK-F0-05` | El pipeline de CI. `scripts/smoke.sh` es su núcleo. |
| `TSK-F0-07` | Los rate limits de `api-contract.md` §1.6. |
| `TSK-F0-08` | El envío real del magic link. Hoy el token viaja en la respuesta, en development. |
| `TSK-F0-09` | El rol de aplicación en el despliegue, y el smoke test del backend. |
| `TSK-F0-10` | El token de dispositivo y la clase del error. **Decisión de producto.** |
| `claim_prize` | Falta la función de la base. Sin ella `FR-DR-08` y `POST /v1/me/prizes/{id}/claim` no tienen camino. La especificación está en [`data-model.md`](./data-model.md) §3.27. |

---

## 8. Referencias

| Documento | Qué aporta |
|---|---|
| [`data-model.md`](./data-model.md) §3.19–§3.27 | Las desviaciones, los seis hechos de Postgres, los SQLSTATE, las funciones de tenant y lo que falta |
| [`architecture.md`](./architecture.md) | Los ADR, las versiones de iii y el procedimiento de upgrade |
| [`spike-iii-health.md`](./spike-iii-health.md) | El spike en sí: por qué existió y qué midió |
| `AGENTS.md` §9 | Las trampas medidas, con su síntoma y su corrección |
