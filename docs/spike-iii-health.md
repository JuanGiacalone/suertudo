# Spike de iii — por qué existe, qué prueba y qué hay que recordar

> **Para quien retome este repo, humano o agente:** este documento explica el *porqué* de `spikes/iii-health/`. Sin él, el spike parece código muerto y el próximo que lo encuentre lo borra sin saber que detrás hay dos riesgos medidos y una versión fijada.
>
> Estado: **PASS**. Commiteado en `feat/fase0-cimientos`. Documentos relacionados: [`PRD.md`](./PRD.md) · [`architecture.md`](./architecture.md) · [`roadmap.md`](./roadmap.md) · [`data-model.md`](./data-model.md)

---

## 1. La decisión, en una línea

Antes de escribir el monorepo, se levantó el stack real de iii en una máquina **sin KVM** y se comprobó que la fórmula de sorteo commit–reveal es reproducible por un tercero. Eso cerró los riesgos `R1` y `R2` del PRD y fijó las versiones de iii en **0.24.6**.

| Veredicto | Detalle |
|---|---|
| Stack completo sin KVM | **PASS** — engine, workers del registro y worker Rust propio, `GET /v1/health` → `200` |
| Fórmula commit–reveal reproducible | **PASS** — 7 tests, golden values calculados con una implementación independiente |
| Ruta de colas y cron | **PASS** — publish, consume, named queue, 3 reintentos, DLQ, redrive, discard, cron |
| Ambos transportes de cola | **PASS** — `builtin` y `rabbitmq` verificados con el mismo script |
| Riesgo `R1` (SDK en RC) | **Cerrado** — engine y SDK fijados en `0.24.6` |
| Riesgo `R2` (KVM en CI) | **Cerrado** — el CI no necesita `/dev/kvm` |
| Transporte de producción de la cola | **Decidido: `builtin`** — [`ADR-013`](./architecture.md) |

---

## 2. Por qué existió este spike

### 2.1 El contexto que lo volvió necesario

El PRD se escribió completo yIa tomó dos decisiones de stack con un riesgo **no verificado**:

| Decisión | Lo que creíamos | Lo que no sabíamos |
|---|---|---|
| Backend en Rust con iii | iii es real, tiene SDK de Rust y encaja con el requisito de orquestación | ¿El SDK en *release candidate* funciona de verdad? ¿Con qué versión del engine? |
| iii como orquestador | El motor maneja workers, colas y HTTP | ¿Los workers necesitan `/dev/kvm`? En CI y en la mayoría de los runners de cloud **no** hay KVM |

Los dos datos que faltaban eran los únicos que podían **invalidar la Fase 0 entera**. Si el SDK RC no compilaba, o si todos los workers exigían microVM, el trabajo de escribir el monorepo, las migraciones y el panel web había sido tirar la basura.

### 2.2 Por qué un spike y no simplemente "empezar y ver qué pasa"

Tres razones, en orden de peso:

1. **El costo de descubrirlo tarde es asimétrico.** Descubrir la incompatibilidad del RC después de escribir 9 workers, 16 migraciones y el CI significa rehacer. Descubrirla con un spike de una tarde significa corregir una línea.
2. **El orden importa.** La Fase 1 empieza por el esqueleto y las migraciones. Si el stack no funciona, esas dos tareas son las primeras que hay que rehacer. El spike va **antes**.
3. **Una decisión de arquitectura sin evidencia es una opinion.** El ADR-002 ("iii es orquestador, no contrato") y el ADR-011 ("los workers corren como proceso, no en microVM") se escribieron **antes** de medir. Este spike los convirtió en hechos verificados, o los refutó.

### 2.3 Lo que el spike NO es

- **No es un prototipo que se tire.** El worker y el script de verificación se mantienen: `scripts/smoke.sh` es el smoke test del CI (`.github/workflows/smoke.yml`).
- **No es una prueba unitaria.** Verifica un stack entero, en un entorno real.
- **No es un documento de arquitectura.** La arquitectura vive en [`architecture.md`](./architecture.md). Este documento explica por qué se midió lo que se midió.

---

## 3. Qué es, en concreto

```
spikes/iii-health/
├── Cargo.toml           # Crate aparte: su propio lockfile y su propio pin del SDK.
├── src/main.rs          # Worker Rust mínimo: una función, un trigger http.
├── src/lib.rs           # Raíz de librería, para que los tests corran sueltos.
├── src/queue_probe.rs   # Las pruebas de cola y cron.
├── config.yaml          # Config del engine. `workers: []` apaga el HTTP legacy.
├── worker-compose.yaml  # Declara los workers `http`, `queue` y `cron` del registro.
└── README.md            # Manual operativo.

scripts/smoke.sh        # El check end-to-end. Idempotente, se autolimpia.
services/draws/          # La fórmula commit-reveal y sus golden tests.
```

Dos piezas del spike PROMUEBAN al producto y por eso se promovieron: el script de verificación pasó a ser `scripts/smoke.sh` —el smoke test del CI— y la derivación pasó a `services/draws/`, que es donde vive la fórmula que el producto promete.

La mínima cantidad de piezas que prueban las dos cosas que importan: **que el stack levanta** y **que la fórmula es auditable**.

---

## 4. Cómo funciona

### 4.1 Las tres primitivas de iii

| Primitiva | Qué es | En el spike |
|---|---|---|
| **Engine** | El router. Recibe invocaciones y las enruta a los workers. Escucha por WebSocket. | `iii --config config.yaml` en `:49134` |
| **Worker** | Cualquier proceso que use el SDK y se conecte al engine. Registra funciones y triggers. | `iii-health-spike` (nuestro) y el `http` del registro |
| **Function** | Una unidad de trabajo direccionable, con esquema de entrada y salida. | `health::check` |
| **Trigger** | La causa que invoca una función. | `http` → `GET /v1/health` |

El engine no es un bus de mensajes ni un servidor web: es un **router de invocaciones**. Un worker registra funciones; otro registra el tipo de trigger `http`; cuando llega una request, el engine enruta al worker queTiene esa función.

### 4.2 El flujo de una request

```
curl GET /v1/health
   │
   ├─▶ worker `http` (0.22.7, proceso del host)   ← tiene el listener en :3111
   │      matchea la ruta contra los triggers `http` registrados
   │
   ├─▶ engine (ws://127.0.0.1:49134)               ← enruta por function_id
   │
   └─▶ worker `iii-health-spike` (nuestro)         ← ejecuta health::check
          │
          └─▶ { status_code: 200, body: {...} }    ← el valor de retorno es la respuesta
```

**Dato clave:** el valor de retorno de la función **es** la respuesta HTTP. La función devuelve `status_code`, `headers` y `body`. No hay un contrato de respuesta aparte.

### 4.3 Cómo se probó la ausencia de KVM

La máquina de desarrollo **sí** tiene `/dev/kvm`. Probar ahí no probaría nada. La técnica: un namespace de mounts privado donde `/dev/kvm` se reemplaza por `/dev/null`.

```bash
unshare -rm bash -c '
  mount --bind /dev/null /dev/kvm
  stat -c "%t,%T" /dev/kvm      # -> 1,3  (hex). Con KVM real sería a,e8.
  ...  # levantar el stack completo
'
```

`stat` imprime en **hexadecimal**: `a,e8` es el KVM real (10, 232) y `1,3` es `/dev/null`. Ese es el assert que usa `scripts/smoke.sh`.

Dentro de ese namespace, `iii`, `iii compose` y el worker Rust levantan igual y el health responde `200`. Ningún proceso `iii-worker` (el lanzador de microVM) llega a existir.

### 4.4 Por qué los workers corren sin microVM

El `iii.worker.yaml` es el manifiesto que le dice a iii **cómo aprovisionar** el worker: qué imagen base, qué dependencias, con qué comando arrancar. Solo importa si iii lo provisions.

> *"Any process that uses an iii SDK, calls `registerWorker()`, and connects to a iii instance is a worker, and behaves identically whether iii started it or not."*

Los tres tipos de worker:

| Tipo | Cómo corre | ¿KVM? |
|---|---|---|
| `binary` | Proceso hijo del host. Es el caso del worker `http` del registro (`Type: binary`). | **No** |
| `path` | Un directorio local, corrido como proceso del host por el compose daemon. | **No** |
| `bundle` | Se aprovisiona en una microVM. Requiere `iii-worker` y `/dev/kvm`. | **Sí** |

Nuestro spike usa `binary` (el del registro) y `path` (el nuestro). **Nunca `bundle`.**

### 4.5 La fórmula commit–reveal

```
Publicación:  seed ← CSPRNG(32 bytes)
               commitment ← hex(SHA-256(seed))        # público e inmutable
               almacenamiento ← AES-256-GCM(seed)      # cifrado hasta el reveal

Ejecución:    para i en 0..100:
                  digest ← HMAC-SHA256(seed, "suertudo/v1|raffle:{id}|draw:{i}")
                  n ← u64_be(digest[0..8])
                  índice ← n mod pool_size
                  si el índice no salió antes: ese es el ganador

Revelado:     seed en claro, público y permanente
```

El golden test la implementa tal cual y la congela. El valor de ejemplo, con `seed = 0..31` y el raffle `3f8a1c22-0b6e-4a91-8d3f-2c7e5a9b1d04`, es **144**; la secuencia de redraw es **144, 820, 74, 885**.

---

## 5. Hallazgos

### 5.1 El stack no necesita KVM — `R2` cerrado

Todos los workers usados son procesos del host. El worker `http` del registro es `Type: binary`; el nuestro es un `path`. `scripts/smoke.sh` asserta que **no** exista ningún proceso `iii-worker` en ningún momento de la corrida.

**Consecuencia:** el pipeline de CI no necesita `/dev/kvm`, ni acceso privilegiado, ni una máquina specially equipped. Eso elimina una restricción de infraestructura que era la vía más rápida a un despliegue de backend.

### 5.2 El SDK 0.25 RC es incompatible con el engine 0.24 — y falla en silencio

Este fue el hallazgo más importante, y el que **no** se esperaba.

| SDK crate | Compila | Registra la función | Registra el trigger `http` |
|---|---|---|---|
| `0.24.6` | sí | funciona | **funciona** — la ruta responde `200` |
| `0.25.0-rc.4` | sí | **funciona** | **RECHAZADO** — la ruta nunca existe |

Con el RC, la función se registra sin error. Pero el trigger queda en el log como:

```
[WARN] iii::trigger Trigger registration rejected by provider;
       parked as pending until the trigger type re-registers
```

y la ruta **no aparece jamás** en el worker HTTP.

**Por qué es peligroso:** `engine::workers::list` muestra el worker como registrado y sano. Todo lo que un health check ingenuo mire dice que está bien. La única señal está en el **log del engine**. Un error de compilación te frena; esto te deja un sistema donde las funciones existen y las rutas no, y el síntoma aparece como "el frontend no recibe nada" mucho tiempo después.

La causa: el RC deprecó `register_trigger` en favor de `iii.observe(Source::function(..).args(..), target)`, que el engine 0.24.6 no implementa. El propio compilador lo avisa:

```
warning: use of deprecated method `iii_sdk::IIIClient::register_trigger`:
  use iii.observe(...) instead; see https://iii.dev/docs/upgrading/from-0-24-x
```

**Decisión:** engine y SDK fijados en `0.24.6`, declarados juntos en `infra/iii/VERSION`, verificados por el CI. La regla de upgrade de [`architecture.md`](./architecture.md) §8.7 (motor y SDK en el mismo PR, una versión a la vez) existe exactamente para evitar esto.

### 5.3 La fórmula es reproducible por un tercero

Los golden values se calcularon con una **implementación independiente** (Python `hmac`/`hashlib`), no con el código Rust, y después se congelaron:

| Dato | Valor |
|---|---|
| `commitment(0xAB * 32)` | `9a2db2e23f1504cd056606553ac049c5e718e8f9ce9233876df1a7a1821af885` |
| `seed = 0..31`, draw 0, pool 1000 | `144` |
| Secuencia de redraw (draws 0..3) | `144, 820, 74, 885` |

7 tests cubren: corrección del commitment, determinismo, el valor golden, la secuencia de redraw, separación por raffle id y límites del pool.

**Detalle de proceso que vale la pena:** la primera corrida de estos tests **falló**, porque los valores golden estaban escritos a mano en vez de calculados. Eso es exactamente el failure mode que el golden test existe para atrapar, y terminó siendo la mejor validación del spike. Si un golden value está inventado, el test falla; si está calculado por una segunda implementación, el test confirma que **dos** implementaciones coinciden.

### 5.4 Trampa operativa: un worker sobreviviente rompe la corrida siguiente

El worker `http` del registro **se reconecta solo al engine** después de una caída. Si sobrevive a una corrida, la siguiente falla con:

```
CONTAINER_NAME_TAKEN: a worker named 'http' is already registered in namespace 'spike'
```

Dos detalles que costaron tiempo:

- Su **nombre de proceso es `http`** (el binario del paquete), no `iii-*`. Un teardown que busque `iii` no lo mata.
- `compose::add` **no reinicia** un worker que ya está declarado en el YAML: solo edita el archivo. Para arrancarlo hace falta `compose::up`.

### 5.5 Trampa de tooling: `pgrep -f` se auto-matchea

El chequeo de precondición "no hay procesos iii vivos" usaba `pgrep -f 'iii --config|iii:c:'`. El patrón matchea **la propia línea de comandos del script**, así que el chequeo siempre encontraba un proceso y fallaba, y el `pkill` mataba al propio shell.

La corrección: `pgrep -x` (match exacto por nombre de proceso) y `pkill -9 -x`. Un nombre exacto no puede auto-matchearse.

### 5.6 El engine no trae ningún trigger de cola ni de cron

**Este es el hallazgo que más afecta al diseño del producto.** El engine 0.24.6 no incluye los tipos de trigger `durable:subscriber` ni `cron`: cada uno necesita su **worker del registro**.

| Tipo de trigger | Worker del registro | Versión |
|---|---|---|
| `http` | `http` | 0.22.7 |
| `durable:subscriber` | `queue` | 0.21.22 |
| `cron` | `cron` | 0.21.26 |

Sin ellos, el trigger queda en `PENDING` **para siempre** y el engine lo dice:

```
[WARN] iii::trigger Trigger ... was NOT activated: trigger type cron is not registered.
It will be registered automatically when the trigger type becomes available.
If this persists, the cron worker is missing — run:
iii trigger -n <compose-daemon-namespace> compose::add worker=cron
```

**Por qué importa:** `draws::close_due_raffles` y `draws::execute_draw` —los dos jobs que cierran y ejecutan sorteos— dependen del tipo `cron`. Un backend sin el worker `cron` **no sortea nada**, y el síntoma es un sorteo que nunca se cierra, no un error.

**Consecuencia para el CI:** el smoke test tiene que assertar que el log del engine no contiene `is not registered` para ningún tipo. Un health check que solo mira un `200` no lo detecta: el sistema queda con las funciones registradas y sin rutas. Es el mismo modo de fallo silencioso que el RC de §5.2, y por eso la aserción tiene que ser explícita.

### 5.7 `TriggerAction::Enqueue` funciona, pero la cola tiene que existir

La doc del worker `queue` advertía: *"The `TriggerAction::Enqueue` path requires an engine version that routes named enqueue actions through the registered `engine::queue::enqueue` provider."* **El engine 0.24.6 tiene ese corte.** Verificado: encolar devuelve `messageReceiptId` y el consumidor recibe el mensaje.

Pero la named queue tiene que definirse antes, y el schema es **estricto**. Tres trampas, cada una con su error:

| Trampa | Error que devuelve |
|---|---|
| `type: concurrent` | `queue 'X' type must be either 'standard' or 'fifo'` |
| `type: fifo` sin `message_group_field` | `queue 'X' with type 'fifo' requires message_group_field` |
| `function_id` dentro de la config | `unknown field 'function_id', expected one of 'max_retries', 'concurrency', ...` |

**La función destino viaja con el mensaje, no con la cola.** Una named queue define el transporte; cada enqueue lleva su `function_id`. Es un detalle de diseño que cambia cómo se escribe el código de notificaciones.

### 5.8 El worker `cron` colisiona con el cron del sistema

El worker `cron` del registro corre como un proceso cuyo nombre es **`cron`**, exactamente igual que el scheduler del sistema (`/usr/sbin/cron`).

```
635   cron   /usr/sbin/cron -f -P                                    ← el del sistema
23070 cron   ~/.iii/compose/packages/cron-0.21.26-.../cron           ← el de iii
```

Un `pkill -x cron` en un script de teardown **mata el cron del sistema**. Es un error que no se manifiesta en el script (que sigue bien) sino en la máquina (que deja de programar tareas).

**La regla: los workers del registro se identifican por su ruta de ejecutable**, no por su nombre. `~/.iii/compose/packages/` es único y no puede matchear un proceso ajeno.

### 5.9 Linux trunca los nombres de proceso a 15 caracteres

`iii-health-spike` (16 caracteres) se registra como `iii-health-spik`. Un `pkill -9 -x iii-health-spike` **no matchea nada y no da error**: el worker viejo seguía vivo y seguía respondiendo con el código viejo, lo que hizo que un fix pareciera no aplicada.

Esto se combina con §5.4: el nombre exacto que se debe usar es el truncado, y para los workers del registro, la ruta.

### 5.10 El SDK de Rust exige un `.await` explícito

`RegisterFunction::new_async` con un closure que llama a `worker.trigger(...)` **no compila** si el future se retorna sin `await`:

```rust
// NO compila: la inferencia de R (el tipo de retorno) no resuelve
RegisterFunction::new_async(move |payload: Value| async move {
    publisher.trigger(TriggerRequest { .. })   // future sin await
})

// Compila
RegisterFunction::new_async(move |payload: Value| async move {
    publisher.trigger(TriggerRequest { .. }).await
})
```

El error que devuelve el compilador es inútil para diagnosticar: `the trait bound {closure@...}: IntoAsyncHandler<_> is not satisfied`. La causa real es que `trigger` es genérico sobre `impl Into<TriggerRequestWithMetadata>`, y al retornar el future sin `await` el compilador no puede inferir el tipo de retorno.

**Regla: siempre `.await` explícito en los handlers async.** Vale la pena conocerlo porque el mensaje del error no lleva a la causa.

### 5.11 En RabbitMQ la DLQ es inalcanzable desde iii

El worker `queue` ofrece tres transportes. `builtin` (in-process, file-backed) es el default y no necesita broker. `rabbitmq` es el completo con broker externo. Los dos se verificaron contra un broker real (RabbitMQ 4.4.0).

| Operación | `builtin` | `rabbitmq` |
|---|---|---|
| Publicar y consumir | ✅ | ✅ |
| Named queue con `TriggerAction::Enqueue` | ✅ | ✅ |
| 3 reintentos con backoff → DLQ | ✅ | ✅ |
| `dlq_messages` (listar la DLQ) | ✅ ve el mensaje | ❌ devuelve `[]` |
| `dlq_topics` | ✅ | ❌ devuelve `[]` |
| `topic_stats` → `dlq_depth` | ✅ correcto | ❌ reporta `0`, **miente** |
| `redrive` (reprocesar) | ✅ | ❌ `NOT_FOUND: no queue 'iii.spike-fail.dlq'` |
| `discard_message` | ✅ | ❌ (misma causa) |

**La causa.** En RabbitMQ, los reintentos escriben en la DLQ **por función**:

```
iii.spike-fail.queue::fail_always@spike.dlq     msgs=1   ← el mensaje está acá
iii.spike-fail.queue::fail_always@spike.queue   msgs=0
```

Pero la API de DLQ de iii direcciona el **topic desnudo** (`iii.spike-fail.dlq`), que no existe. El mensaje queda en el broker y **nadie puede verlo, reprocesarlo ni descartarlo desde iii**: hay que ir a la API de administración de RabbitMQ a mano.

La propia documentación de iii lo reconoce y lo remite a un ticket upstream (`MOT-3904`).

**Por qué esto decide la arquitectura.** Un producto cuyo ciclo central es "sortear → notificar al ganador" necesita poder **operar** los mensajes que fallaron. Si un push al ganador queda dead-lettered y no se puede re-encolar, esa persona no se entera nunca. Con `builtin`, la DLQ es operable con la API.

**Decisión: `builtin` en producción** (`ADR-013`). Además evita un broker externo: un servicio menos que operar y que puede caerse.

**Regla para futuros agentes:** cuando se mida un comportamiento, se miden **los dos** caminos, no solo el que funciona. El `builtin` daba verde en las 13 secciones y el `rabbitmq` también daba verde en las 9 primeras: la diferencia solo apareció en las de DLQ, que son las que importan.

---

## 6. Correcciones que este spike imponió a los documentos previos

| Documento | Decía | La realidad medida | Estado |
|---|---|---|---|
| `architecture.md` ADR-002 | "SDK en release candidate `0.25.0-rc.4`" | El RC deprecó `register_trigger` y su trigger queda rechazado contra el engine 0.24.6 | **Corregido**: versiones fijadas en 0.24.6 (§8.6) |
| `architecture.md` ADR-011 | "Los workers en microVM exigen `/dev/kvm`" | Solo los `bundle` lo exigen. `binary` y `path` corren como proceso del host | **Corregido y verificado** |
| `PRD.md` R1 | Probabilidad media, impacto alto | El RC rompe de forma silenciosa, no ruidosa | **Cerrado** con evidencia |
| `PRD.md` R2 | Probabilidad alta, impacto medio | No aplica: el stack levanta sin KVM | **Cerrado** con evidencia |
| `roadmap.md` §6 | "Riesgo principal: que iii no funcione sin KVM" | Verificado. El riesgo real de Fase 0 pasó a ser la ruta de colas | **Corregido** |

> **Moraleja para futuros agentes:** la documentación de iii dice *"the engine and SDK packages can have different patch versions within the same minor line"*. Es cierto **dentro** de una línea minor. Entre una estable y un RC, no. La nota de release del RC es la que deprecó la API de triggers. **Midalo, no lo supongas.**

---

## 7. Detalles a tener en cuenta para futuros modelos

Reglas operativas, derivadas de los hallazgos. Todas son verificables.

### 7.1 Sobre el stack

| Regla | Por qué |
|---|---|
| **Engine y SDK siempre en la misma versión, y en el mismo PR.** | El RC deprecó una API que el engine estable no implementa. El fallo es silencioso. |
| **Nunca uses `latest` ni un tag flotante** fuera de `local`. | La resolución de versión tiene que ser reproducible. `infra/iii/VERSION` declara ambas. |
| **El smoke test del CI asserta una RUTA respondiendo `200`.** | Un worker registrado en `engine::workers::list` puede no tener ninguna ruta. Ese fue exactamente el fallo del RC. |
| **El smoke test también asserta que el log del engine no dice `is not registered`.** | Es el síntoma del worker faltante. Sin esa aserción, un backend sin `queue` ni `cron` parece sano y no sortea nada. |
| **Todo tipo de trigger que uses necesita su worker del registro declarado.** | `http`, `queue` y `cron` son tres workers separados. `worker-compose.yaml` tiene que listar los tres. |
| **Medí los dos transportes de cola, no solo el que funciona.** | `builtin` daba verde en las 13 secciones y `rabbitmq` en las 9 primeras. La diferencia —la DLQ inalcanzable— solo apareció en las secciones que importan. |
| **El transporte de cola de producción es `builtin`** (`ADR-013`). | En RabbitMQ la API de DLQ no puede direccionar la DLQ por función: los mensajes muertos quedan inaccesibles desde iii. |
| **Un worker Rust que solo registra y sale del `main` muere al instante.** | Los hilos del SDK no mantienen vivo el proceso. Llamá `std::thread::park()` al final, o `shutdown_async()`. |
| **`compose::add` edita el YAML; `compose::up` arranca lo declarado.** | Son dos operaciones distintas y la confusión entre ellas produce un worker en estado `stopped`. |
| **`.await` explícito en todo handler async del SDK.** | `RegisterFunction::new_async` no compila si se retorna el future de `trigger` sin `await`, y el error no lleva a la causa. |
| **Definí la named queue antes de encolar, y recordá que la función destino va en el mensaje.** | El schema de `queue::define` es estricto: `type` es `standard` o `fifo`, `fifo` exige `message_group_field`, y `function_id` no pertenece a la config. |

### 7.2 Sobre los scripts

| Regla | Por qué |
|---|---|
| **Identificá los workers del registro por su ruta de ejecutable, nunca por nombre de proceso.** | El worker `cron` comparte nombre con `/usr/sbin/cron` del sistema. Un `pkill -x cron` mata el scheduler del SO. |
| **Recordá que Linux trunca los nombres a 15 caracteres.** | `iii-health-spike` es `iii-health-spik`. Un `pkill -x` con el nombre largo no matchea nada y deja el worker viejo sirviendo. |
| **Usá `pgrep -x` y `pkill -x`, nunca `-f`, para nombres de proceso.** | `-f` matchea la propia línea de comandos del script: el chequeo miente y el teardown se suicida. |
| **El teardown tiene que matar a los workers del registro.** | Se reconectan solos al engine. Si sobreviven, la corrida siguiente falla con `CONTAINER_NAME_TAKEN`. |
| **Cada corrida del smoke test tiene que terminar con cero procesos iii vivos.** | Es la precondición de la corrida siguiente. `scripts/smoke.sh` lo asserta al principio y lo limpia al salir. |
| **Para probar la ausencia de KVM: `unshare -rm` + `mount --bind /dev/null /dev/kvm`.** | `stat -c '%t,%T'` da `1,3` en hex cuando el KVM está enmascarado, `a,e8` cuando es real. |

### 7.3 Sobre el producto

| Regla | Por qué |
|---|---|
| **Los golden values se calculan con una segunda implementación, no a mano.** | Un valor inventado hace fallar el test, que es el objetivo. Un valor calculado por dos implementaciones **confirma** que la fórmula es la documentada. |
| **El golden value del sorteo es `144` para `seed = 0..31`.** | Si el test falla con otro número, la fórmula cambió: actualizá el PRD, `data-model.md` **y** este documento. |
| **El `commitment` se publica al publicar el sorteo, no al cerrarlo.** | Es lo que hace auditable el resultado: el compromiso precede a las reservas. |
| **El smoke test de Fase 0 no tiene que probar el dominio.** | El dominio se prueba con tests de dominio contra Postgres. El smoke test prueba el **plumbing**. |

### 7.4 Sobre el proceso de trabajo

| Regla | Por qué |
|---|---|
| **Un spike sin documento de rationale se lee como código muerto.** | El próximo lo borra. Este documento es lo que lo mantiene vivo. |
| **Si un spike tapa un riesgo y deja otro abierto, hay que decirlo en voz alta y dejar la tarea con ID.** | La ruta de colas quedó abierta y tiene ID: `TSK-F0-06`. |
| **No reescribas una decisión de arquitectura sin evidencia.** | `ADR-002` y `ADR-011` se escribieron antes de medir. Este spike los verificó en vez de sustituirlos a ciegas. |

---

## 8. Lo que el spike NO cubre

Honestidad sobre los límites. La lista es corta a propósito.

| No cubierto | Por qué importa | Dónde se cierra |
|---|---|---|
| **Postgres, Redis** | El spike no levantó ningún servicio de datos. | `TSK-F0-03` |
| **Auth, RBAC, middleware, namespaces múltiples** | El spike usó un solo namespace, sin autenticación. | `TSK-F0-04` |
| **Workers `bundle` y `iii-worker`** | Los que sí necesitan VM. No se usan en Suertudo. | Fuera de alcance |
| **Observabilidad y trazas OTel** | El engine la inicializa, pero no se verificó el pipeline completo. | `TSK-F0-05` |
| **Comportamiento bajo caída del broker** | Qué pasa con los mensajes en vuelo si el worker de cola se cae a mitad de un reintento. | Fase 1, con la carga real |
| **El fix upstream `MOT-3904`** | Si iii corrige la direccionación de la DLQ en RabbitMQ, hay que repetir la comparación de §5.11. | Cuando salga el fix |

**La regla:** el spike cierra el riesgo que bloquearía la Fase 0. No pretende ser la Fase 0.

---

## 9. Cómo reproducir y extender

```bash
# Stack completo en esta máquina
./scripts/smoke.sh

# Forzar la ruta sin KVM (lo que hace el CI)
unshare -rm ./scripts/smoke.sh
```

En el CI el mismo script corre en `.github/workflows/smoke.yml`, que además lo dispara **por calendario**: si una versión upstream del motor rompe el stack, tiene que aparecer en el pipeline del lunes y no en el próximo PR que toque el backend.

El script es autocontenido: compila el worker de prueba antes de arrancarlo. El spike está excluido del workspace de Rust — tiene su propio lockfile y su propio pin del SDK — así que `cargo build --workspace` no lo construye, y un script que necesita un binario compilado a mano es un script que sólo funciona en la máquina de quien lo escribió.

Salida esperada:

```
### 0. Precondición: sin procesos iii previos      OK: limpio
### 1. Estado de KVM                                OK
### 2. Engine                                       OK: engine escuchando en 49134
### 3. Workers del registro (http, queue, cron)     OK
### 4. Worker Rust propio                           OK: worker registrado
### 5. GET /v1/health                               OK: HTTP 200
### 6. Invocación directa por el engine             OK
### 7. Sin microVM                                  OK: ningún proceso iii-worker
### 8. Cola: publicar y consumir                    OK
### 9. Named queue vía TriggerAction::Enqueue       OK
### 10. Reintentos y DLQ                           OK
### 11. Redrive y discard de la DLQ                OK
### 12. Cron                                       OK
### 13. Golden test de la derivación               OK: tests verdes (7 passed)
### RESULTADO: PASS
```

**Para extenderlo** (agregar la ruta de colas, por ejemplo): se agrega un chequeo nuevo al script, con su propia sección, su `pass`/`fail` y su teardown. El patrón es: precondition → arrancar → esperar estado → assertar → limpiar.

---

## 10. Glosario de iii

| Término | Significado en este repo |
|---|---|
| **Engine** | El router de invocaciones. Escucha por WebSocket en `:49134`. |
| **Worker** | Proceso que usa el SDK y registra funciones y triggers. |
| **Function** | Unidad direccionable con esquema de entrada/salida. |
| **Trigger** | La causa que invoca una función. `http`, `cron`, `queue`, `durable:subscriber`, `state`. |
| **Namespace** | Ámbito donde se resuelven las funciones. Aísla workers con el mismo nombre. |
| **Compose** | El daemon que administra un proyecto de workers. Lee `worker-compose.yaml`. |
| **`worker-compose.yaml`** | Declaración de los workers del proyecto. `worker-compose.lock` fija las versiones resueltas. |
| **`iii-worker`** | El lanzador de microVM. **No** se usa en Suertudo. |
| **`binary` / `path` / `bundle`** | Tipos de worker. Los dos primeros corren como proceso del host; el tercero necesita VM. |
| **DLQ** | *Dead letter queue*. Cola de mensajes que agotaron sus reintentos. |
| **Outbox** | Patrón de la base: escribir el evento en la misma transacción que confirma el cambio, para que un crash no deje el estado a medias. |

---

## 11. Nota de entorno: Engram y el hook de `mem_save`

No es parte del producto, pero le va a pasar a cualquiera que trabaje en esta máquina y conviene que esté escrito.

**Síntoma:** `mem_save` falla con

```
gentle-engram could not resolve an authoritative OpenCode runtime session for mem_save
```

**Causa:** el plugin `~/.config/opencode/plugins/engram.ts` valida, antes de cada escritura, que la sesión de OpenCode resuelva contra el proyecto del servidor. Si el servidor de OpenCode se reinició, la identidad de sesión se pierde y el plugin bloquea **todas** las escrituras, aunque las lecturas sigan funcionando.

**Vía de escape:** el CLI `engram` habla con la misma instancia y escribe en el mismo store.

```bash
engram save "<título>" "<contenido>" --type discovery --project suertudo --topic <topic_key>
engram conflicts scan --project suertudo --apply --yes   # regenera las relaciones
```

**Para juzgar relaciones** que el scan dejó pendientes, el `judgment_id` es el `sync_id` de la relación (formato `rel-<hex>`):

```bash
engram conflicts list --project suertudo --status pending
```

y después `mem_judge` con ese `judgment_id`. El `mem_judge` del MCP funciona aunque `mem_save` esté bloqueado, porque no pasa por el hook de escritura.

**Estado de este repo:** 11 observaciones, 53 relaciones juzgadas, 0 pendientes.

---

## 12. Referencias

| Documento | Qué aporta |
|---|---|
| [`PRD.md`](./PRD.md) §7.2, §13 | La fórmula commit–reveal y los riesgos `R1`/`R2` ya corregidos con la evidencia de este spike. |
| [`architecture.md`](./architecture.md) ADR-002, ADR-011, §8.6, §8.7 | El motor como orquestador, los workers como proceso, las versiones fijadas y el procedimiento de upgrade. |
| [`roadmap.md`](./roadmap.md) `TSK-F0-06` | La ruta de colas, que es la última incógnita de Fase 0. |
| [`data-model.md`](./data-model.md) §3.8 | `draws`: commitment, semilla cifrada y revealed. |
| `spikes/iii-health/README.md` | Manual operativo del spike. |
