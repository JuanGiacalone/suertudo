# Pipeline de CI — manual operativo

> **Para quien retome este repo, humano o agente:** este documento es el manual del pipeline. [`architecture.md`](./architecture.md) §9 dice **por qué** el pipeline es como es; este dice **cómo se usa**, **qué se rompe** y **qué hacer cuando se rompe**. Sin él, los checks parecen arbitrarios y el primero que los cambie sin entenderlos los deshace.
>
> Estado: **verde en los tres stacks**. Commiteado en `feat/fase0-cimientos`. Documentos relacionados: [`architecture.md`](./architecture.md) §9 · [`roadmap.md`](./roadmap.md) `TSK-F0-05` · [`spike-iii-health.md`](./spike-iii-health.md) · [`verificacion-fase0.md`](./verificacion-fase0.md)

---

## 1. La decisión, en una línea

El pipeline es **la red de seguridad de todo lo que viene**: un PR que rompe el build no llega a `main`. Por eso vive en el repositorio y no en un script que alguien recuerda correr.

| | |
|---|---|
| Dónde | `.github/workflows/ci.yml` y `.github/workflows/smoke.yml` |
| Cuándo corre | Cada push a `main`, cada pull request, y el smoke **por semana** |
| Qué protege | Los tres stacks: Rust, Flutter, Next.js — más el stack de iii entero |

---

## 2. Los dos archivos y los cuatro jobs

La partición es de **naturaleza**, no de tamaño:

| Archivo | Jobs | Por qué |
|---|---|---|
| `ci.yml` | `rust`, `flutter`, `nextjs` | El código, en minutos, en cada push |
| `smoke.yml` | `smoke` | El stack de iii entero, de punta a punta, sin KVM |

**El smoke está aparte por dos razones.** Necesita el motor, la red y minutos —y no es lo mismo que "el código está bien"—. Y es el único job que vale la pena correr **por calendario**: si una versión upstream del motor rompe el stack, tiene que aparecer en el pipeline del lunes y no en el próximo PR que toque el backend. También se puede correr a mano, con `workflow_dispatch`.

**El orden dentro de `ci.yml` no es arbitrario.** Lo más barato y lo más informativo va primero, para que un error se encuentre en segundos y no en minutos:

| Job | Comprueba | Con qué |
|---|---|---|
| `rust` | `fmt`, `clippy`, `test`, cache de sqlx, `cargo audit` | Un Postgres 16 de servicio |
| `flutter` | `analyze`, `format`, guard de tests enfocados, `test` | Flutter 3.47.7 |
| `nextjs` | `npm ci`, `lint`, `tsc --noEmit`, `build` | Node 26.12.0 |

---

## 3. Correrlo localmente

**El pipeline se puede correr entero en local, y hay que correrlo antes de pushear.** No es una cortesía: varios de los checks que fallaron en esta sesión no fallaban en local por falta de base de datos, y otros porque el shell del CI es `bash -e` y el de tu terminal no.

### 3.1 El job de Rust

```bash
# La base. El harness aplica las migraciones y crea el rol de la aplicación.
export DATABASE_URL_TEST="postgres://suertudo:suertudo@127.0.0.1:5432/suertudo_test"

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked

# El check del cache de sqlx, tal como lo hace el pipeline.
touch services/db/src/lib.rs
SQLX_OFFLINE=false SQLX_OFFLINE_DIR="$PWD/services/db/.sqlx" \
  DATABASE_URL="$DATABASE_URL_TEST" cargo build -p suertudo-db --locked
git status --porcelain services/db/.sqlx   # tiene que salir vacío

# Las vulnerabilidades.
cargo install cargo-audit --locked --version 0.22.2
cargo audit
cargo audit --file spikes/iii-health/Cargo.lock
```

> **La base tiene que existir y estar migrada.** El harness aplica las migraciones solo, pero necesita una base `suertudo_test` a la que llegar:
> `createdb suertudo_test` y el rol `suertudo` tiene que existir.

### 3.2 El job de Flutter

```bash
cd apps/mobile
flutter pub get
flutter analyze
dart format --set-exit-if-changed .

# El guard de tests enfocados: primero su test, después el guard.
cd ../..
./scripts/check-focused-tests_test.sh
./scripts/check-focused-tests.sh

cd apps/mobile && flutter test
```

### 3.3 El job de Next.js

```bash
cd apps/business-web
npm ci
npm run lint
npx tsc --noEmit
npm run build
```

### 3.4 El job de smoke

```bash
# El motor, en la versión que el proyecto declara.
version="$(grep '^iii=' infra/iii/VERSION | cut -d= -f2)"
curl -fsSL https://install.iii.dev/iii/main/install.sh | sh -s -- --non-interactive "$version"

# El stack entero, sin KVM.
unshare -rm ./scripts/smoke.sh
```

Salida esperada: `### RESULTADO: PASS`. El script compila el worker de prueba, levanta el motor, los workers del registro y el worker propio, y comprueba la ruta HTTP, la cola, el cron y el valor dorado de la derivación.

### 3.5 Los linters que el pipeline NO corre

El pipeline no corre `actionlint` ni `shellcheck`, y en local son los que más Findings dan. Instalalos:

```bash
brew install actionlint shellcheck    # macOS
actionlint .github/workflows/*.yml
shellcheck scripts/*.sh
```

---

## 4. El contrato de cada check

Qué falla, por qué y qué hacer. Esta es la parte que hay que leer antes de tocar el pipeline.

| Check | Falla cuando | Qué hacer |
|---|---|---|
| `cargo fmt --all -- --check` | El código no está formateado | `cargo fmt --all` |
| `cargo clippy --locked -- -D warnings` | Un warning de clippy | Arreglarlo. **No** bajes el nivel del lint |
| `cargo test --workspace --locked` | Un test falla | Arreglar el test o el código. Un test que falla es información, no un obstáculo |
| El cache de sqlx | Una query cambió y el cache no se regeneró | Regenerar (§5.2). **No** borres el archivo a mano |
| `cargo audit` | Una dependencia tiene un aviso | Actualizar la dependencia. Un *yanked* también sale: hay que actualizar |
| `flutter analyze` | Un error de análisis | Arreglarlo |
| `dart format --set-exit-if-changed .` | El código no está formateado | `dart format .` |
| El guard de tests enfocados | Hay un `test.only`, `testWidgets.only` o `group.only` | Sacar el `.only`. El script **no** falla por un comentario |
| `flutter test` | Un test falla | Arreglarlo |
| `npm run lint` | Un error de eslint | Arreglarlo |
| `npx tsc --noEmit` | Un error de tipos | Arreglarlo |
| `npm run build` | El build falla | Arreglarlo |
| El smoke | El stack de iii no levanta, o una ruta no responde | Ver §8 |

---

## 5. Recetas

### 5.1 Cambiar la versión de una herramienta

**Una sola fuente de verdad por versión.** Un CI que compila con otro compilador que el tuyo es un CI que prueba otra cosa.

| Herramienta | Dónde vive la versión | Cómo la lee el CI |
|---|---|---|
| Rust | `rust-toolchain.toml` | `rustup`, en local y en el CI |
| Node | `apps/business-web/.nvmrc` | `setup-node` con `node-version-file` |
| Flutter | el número exacto, en `ci.yml` | `flutter-action` |
| iii (motor y SDK) | `infra/iii/VERSION` | el paso que instala el motor |
| `cargo-audit` | el número en `ci.yml` | `cargo install` |

Para cambiar una:

1. Cambiá el archivo (o el número en el workflow).
2. Corré el job correspondiente en local (§3).
3. Commit. El archivo y el pipeline van en el **mismo** commit: si divergen, el pipeline prueba una cosa y vos probás otra.

> **La versión del motor y la del SDK se mueven juntas, en el mismo pull request.** Contra un motor 0.25, un SDK 0.24 registra la función pero rechaza el trigger, y la ruta nunca existe. El síntoma es silencioso: el worker parece sano en `engine::workers::list`.

### 5.2 Regenerar el cache de sqlx

El cache de `services/db/.sqlx` **está commiteado a propósito**: sin él, `cargo build` en un checkout limpio falla con errores de compilación de los macros `query!`.

**La expansión de los macros nunca poda.** Si borrás una query, su archivo de cache queda atrás y el pipeline no lo detecta: el check compara el cache contra el esquema vivo y sólo ve las queries **nuevas o cambiadas**. Regenerar de cero lo resuelve.

```bash
# 1. El cache se aparta. El destino tiene que ser NUEVO: con un destino
#    fijo, la segunda corrida anida el cache dentro del directorio que ya
#    existe y la comparación del paso 4 lista las cosas equivocadas.
old="$(mktemp -d)"
mv services/db/.sqlx "$old/cache" && mkdir -p services/db/.sqlx

# 2. La recompilación se fuerza. SIN ESTO, cargo ve el crate ya compilado,
#    no vuelve a expandir los macros, y el paso 3 no regenera NADA: el
#    procedimiento aparenta funcionar y no cambia una línea.
touch services/db/src/lib.rs

# 3. Los macros se expanden contra el esquema vivo y escriben el cache nuevo.
SQLX_OFFLINE_DIR=$PWD/services/db/.sqlx DATABASE_URL=<la de la base> \
  cargo build -p suertudo-db

# 4. `comm -23` muestra SOLO lo que sobró. Se borran a mano.
#    `comm` exige las entradas ORDENADAS, y `ls` ordena por locale.
comm -23 <(ls "$old/cache" | LC_ALL=C sort) <(ls services/db/.sqlx | LC_ALL=C sort)
```

> **El paso 2 no es opcional, y su ausencia es silenciosa.** Es la misma trampa de `AGENTS.md` §5.3: `sqlx::migrate!` embebe las migraciones en tiempo de compilación. Sin el `touch`, el build termina en menos de un segundo, dice `Finished`, y el cache queda vacío.

### 5.3 Agregar un check

**Un check tiene que poder fallar.** La regla de `AGENTS.md` §9: un check que sólo busca la ausencia de un error reporta PASS cuando nada corrió.

Tres reglas que nacieron de los bugs que encontró la revisión:

1. **El estado de salida del escaneo se mira por separado del veredicto.** Un `grep A | grep -v B` pierde el estado de `A`: un escaneo roto y un repositorio limpio se ven iguales.
2. **El veredicto no es el estado de salida del filtro.** Un escaneo vacío produce una línea en blanco que el filtro deja pasar. Lo que decide es el **contenido**: la sustitución de comandos se come el salto de línea final.
3. **`bash -e` convierte un resultado vacío en un paso rojo.** El shell del CI es `bash -e {0}`. Un `set -e` mata el paso antes del veredicto.

Y el patrón que las tres evitan: **la lógica va en un script con su test**, no en el YAML. El guard de tests enfocados es el ejemplo: `scripts/check-focused-tests.sh` con `scripts/check-focused-tests_test.sh`, y el pipeline corre el test **antes** del guard.

### 5.4 Agregar un stack nuevo

1. Un job nuevo en `ci.yml`, con su toolchain y sus comandos.
2. La versión de su toolchain en **un** archivo, no en dos.
3. Un test de humo que se pueda correr en local, como `scripts/smoke.sh`.
4. Si el stack necesita una base, un **service container** y la variable `DATABASE_URL_TEST` al nivel del workflow, para que la reciba todo job.

### 5.5 Agregar un job al smoke

El smoke test es un script, y un script es más fácil de extender que un workflow: agregá una sección `### N.` en `scripts/smoke.sh`, con su `pass` o su `fail`, y actualizá el encabezado del script, que es la lista de lo que verifica.

---

## 6. El guard de tests enfocados

Dart no tiene `forbidOnly`: un `test.only` que queda en el repo hace que la suite corra **un** test y salga con 0. El pipeline queda verde mientras el resto de la suite nunca corrió.

```bash
./scripts/check-focused-tests.sh          # falla si hay un test enfocado
./scripts/check-focused-tests_test.sh     # el test del guard
```

| Código de salida | Significado |
|---|---|
| `0` | No hay tests enfocados |
| `1` | Hay un test enfocado |
| `2` | **El escaneo falló.** No es lo mismo que "no hay": es un fallo del escaneo |

Las tres ramas están fijadas por el test, y el RED se observó: quitar el `|| true`, cambiar el `-gt 1`, dejar de eximir los comentarios o dejar de distinguir el escaneo roto, hacen fallar el test.

**El guard cubre todo el repositorio**, no sólo `apps/mobile`: un test enfocado en un paquete que no es la app móvil sigue siendo un test enfocado. El escaneo salta los directorios de las dependencias y de las builds.

---

## 7. Las trampas

Cada una costó un bug. Están también en `AGENTS.md` §9; lo que cambia es que **estas las costó el pipeline**.

| Trampa | Qué pasa | Qué hacer |
|---|---|---|
| `bash -e` y una asignación con un `grep` que no encuentra nada | El paso muere antes del veredicto | El `\|\| true`, o el filtro dentro de la condición de un `if` |
| Un `grep A \| grep -v B` | Se pierde el estado de `A` | Separar el escaneo del veredicto y mirar los dos estados |
| `cargo build` incremental después de cambiar el esquema | No re-expande los macros, y el cache queda vacío | `touch services/db/src/lib.rs` |
| La expansión de sqlx no poda | Los archivos de las queries borradas quedan para siempre | Regenerar el cache de cero (§5.2) |
| `flutter test` con `test/` vacío | Sale con código 1: el job nace rojo | Un test de humo, como el que hay |
| Un script que arranca un binario que no compila | En el CI el binario no existe | Que el script lo compile |
| Un artefacto excluido del workspace | `cargo build --workspace` no lo construye | El script que lo usa lo compila, con `--locked` |
| `cargo` sin `--locked` | Re-resuelve las dependencias en silencio | `--locked` en todos los comandos |
| `grep` con el contenido del repo en el stdout | GitHub Actions lo lee como comando de workflow | El contenido al stderr, la anotación estática al stdout |

---

## 8. Diagnóstico: el pipeline está rojo

### El smoke falla

```bash
unshare -rm ./scripts/smoke.sh
```

El script deja los logs en `spikes/iii-health/`: `engine.log`, `compose.log`, `spike.log`. **Mirá el `engine.log` primero**: la línea `trigger type X is not registered` es el síntoma del worker que falta, y es el motivo por el que el smoke comprueba que el log no la tenga.

| Síntoma | Causa probable |
|---|---|
| El engine no levanta | La versión de `infra/iii/VERSION` no coincide con el SDK del crate |
| Un trigger queda `PENDING` | Falta el worker del registro que da ese tipo de trigger |
| La ruta responde pero vacía | El trigger no se registró |
| El probe no arranca | El spike no compiló: corré `cargo build --locked --bin iii-health-spike` en `spikes/iii-health/` |

### El job de Rust falla en la base

El harness necesita una base `suertudo_test` alcanzable y el rol `suertudo` con permiso de crear el rol de la aplicación. En local: `createdb suertudo_test`.

### El check del cache de sqlx falla

Casi siempre es §5.2: la query cambió y el cache no. **Regenerá el cache**, no borres el archivo.

---

## 9. La revisión del pipeline

El pipeline se revisó con la **revisión nativa de Gentle AI**, seis rondas, todas aprobadas. No es un adorno: encontró defectos que ya estaban en el código, entre ellos dos CRÍTICOS.

Cómo se corre:

```bash
# 1. ¿Toca revisarlo?
gentle-ai review assess --cwd . --agent opencode --base-ref <último commit revisado> \
  --committed-only --json          # leer review_due y review_due_reason

# 2. Si review_due es true, el preflight devuelve el comando exacto del START.
#    Se ejecuta VERBATIM: los tokens los genera el provider y no se reconstruyen.

# 3. El START devuelve un envelope de consentimiento. Se le relay al humano
#    con sus tres opciones, y se ejecuta la invocación EXACTA de la elegida.

# 4. Los tres revisores (risk, readability, reliability) se lanzan con el
#    agent y el prompt que devuelve el collect, sin modificarlos.

# 5. Si hay hallazgos severos, el refuter y el validador toman la decisión.
```

**Lo que encontró, y por qué está documentado:** el guard que moría en el caso que él mismo eximía, la anotación de error que se perdió al mover la lógica a un script, el alcance que se achicó a un solo directorio, y el procedimiento documentado del cache que no funcionaba. Todo eso está en los commits, con el porqué en el mensaje.

**Si el assess dice que hay archivos sin versionar, no es un error: es una pregunta.** Un candidato con archivos sin versionar es ambiguo —¿entran en la revisión o no?— y el `assess` no adivina. Hay que declarar el alcance:

```bash
# El inventario canónico, que lo imprime el STATUS:
gentle-ai review status --cwd . --contract gentle-ai.review-integration/v2 \
  --agent opencode --next-transition

# Y el assess, con el alcance declarado:
gentle-ai review assess --cwd . --agent opencode --base-ref <commit> --committed-only --json \
  --untracked-scope=exclude --expected-untracked-inventory=sha256:<el que imprime el STATUS>
```

`--untracked-scope=exclude` dice que los archivos sin versionar **no** entran. `--untracked-scope=select --intended-untracked=<ruta>` dice que entra uno solo, y el hash del inventario tiene que coincidir con el que imprime el STATUS. La forma más simple: **versioná todo antes de revisar**, y el problema desaparece.

---

## 10. Lo que NO está, y lo que quedó abierto

**Criterios de `TSK-F0-05` que no se pudieron cumplir, y por qué:**

| Criterio | Por qué no |
|---|---|
| Job de Next.js: `test` | El panel no tiene runner de tests ni tests. Instalar una cadena de dependencias nueva para testear un scaffolding que la Fase 1 va a reemplazar no compra nada. Se cierra con la primera pantalla del panel |
| Job de contrato (OpenAPI) | **No hay OpenAPI.** El contrato vive escrito a mano en [`api-contract.md`](./api-contract.md) y la API sólo expone `/v1/health`. Diferenciar un archivo contra sí mismo no es un check |
| Job de evals de IA | `services/ai-branding` es de la Fase 2 |

**Abierto como trabajo posterior:**

- Anclar las actions de terceros a su SHA en vez de a su etiqueta mayor, y verificar el instalador del motor con un checksum. El instalador se baja de una URL mutable y se ejecuta en cada push: **es el único punto del pipeline que ejecuta código de un tercero sin fijar**.
- Un caso del test del guard que invoque el script con un **archivo suelto** en vez de un directorio. El `-H` del escaneo existe para que el formato de la salida sea el mismo en los dos casos, y ese contrato todavía no lo fija ningún test.
- Que el caso omitido bajo root deje una marca que el resumen del test consuma, y no sólo una línea que se lee a ojo.

---

## 11. Lo que este documento NO es

No es la especificación del pipeline: eso es [`architecture.md`](./architecture.md) §9. No es el registro de las rondas de verificación del backend: eso es [`verificacion-fase0.md`](./verificacion-fase0.md). No es el producto: eso es [`PRD.md`](./PRD.md).

Es lo único que responde a la pregunta que se hace el próximo que toca el pipeline: **"¿qué hago cuando esto se rompe?"**
