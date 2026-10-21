# Contrato del proveedor de SMS

> **Qué es esto:** los requisitos de código y de implementación del **servicio externo** que falta. El flujo de la toma de control de un dispositivo ya está construido y verificado contra un puerto; lo que falta es una implementación real de ese puerto.
>
> **Mientras no exista, el worker de identidad se niega a arrancar fuera de development.** Es deliberado: el único emisor que hay escribe los códigos de los usuarios en el log.
>
> El flujo que usa este servicio: [`toma-control-dispositivo-sms.md`](./toma-control-dispositivo-sms.md) · El contrato HTTP: [`api-contract.md`](./api-contract.md) §3.8

---

## 1. Qué tiene que resolver el proveedor

El servicio de SMS tiene **una sola misión** y una sola llamada:

> Entregar un mensaje de texto a un número de teléfono y decir con verdad si lo entregó o no.

No decide a quién se manda el mensaje, no genera el código, no guarda nada. Todo eso es del worker.

| | Lo hace el worker | Lo hace el proveedor |
|---|---|---|
| Genera el código de seis dígitos | ✅ | |
| Decide a quién va el mensaje | ✅ — el dueño actual del token | |
| Decide el cuerpo del mensaje | ✅ | |
| Guarda el hash del código | ✅ | |
| Cuenta los intentos y vence la verificación | ✅ | |
| **Entregar el texto al teléfono** | | **✅** |
| **Decir si la entrega ocurrió** | | **✅** |
| **No duplicar el código** | | **✅** |

---

## 2. El puerto

El proveedor implementa **un solo trait**. Vive en `services/identity/src/ports.rs` y no depende de nada del proveedor: es la costura que permite escribir uno sin tocar el flujo.

```rust
/// The SMS channel. The provider is not decided yet: the flow is built
/// against this port so that integrating one is a matter of writing an
/// implementation.
pub trait SmsSender: Send + Sync {
    fn send(&self, to: &str, message: &SmsMessage) -> BoxFuture<'_, Result<(), SmsError>>;
}
```

### 2.1 Los tipos que recibe

```rust
/// The text of the message.
pub struct SmsMessage {
    /// The body of the message.
    pub body: String,
}

impl SmsMessage {
    /// A message with its body.
    pub fn new(body: impl Into<String>) -> Self {
        Self { body: body.into() }
    }
}
```

```rust
/// The failure of the SMS channel.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SmsError {
    /// The provider refused the message.
    #[error("the SMS provider refused the message: {0}")]
    Refused(String),
    /// The channel failed for a reason the provider does not explain.
    #[error("the SMS channel failed: {0}")]
    Channel(String),
}
```

### 2.2 Qué significa cada parámetro

| Parámetro | Qué es | Qué **no** es |
|---|---|---|
| `to` | El teléfono en E.164 de la persona a la que va el mensaje: **la que hoy es dueño del dispositivo**. | Nunca es el del reclamante. Nunca lo elige el proveedor. |
| `message` | El cuerpo, con el código dentro. | El proveedor **no genera** el código ni lo modifica. |

**El proveedor nunca elige el destinatario.** Si necesita un prefijo de país o un formato distinto, lo aplica sobre el `to` que recibe, y nada más.

---

## 3. Las cinco garantías

Estas son las condiciones que **no** son negociables. Cada una tiene su razón, y la razón importa más que la regla cuando alguien tenga que decidir en un caso que no está escrito acá.

### G1 — Nunca `Ok` por un mensaje que no se entregó

**Qué pasa si se rompe:** el worker responde `201`/`202` como si todo estuviera bien, y el participante espera un código que nunca llega. La verificación vence sola, y el día que expire el participante ya se fue.

**Cómo se cumple:** el `Ok` se devuelve **después** de que el proveedor acepte el mensaje. Si el proveedor devuelve un identificador de envío, ese identificador es la prueba.

**Cómo se prueba:** un proveedor de mentira que devuelve `Ok` sin hacer nada **tiene que hacer fallar el test** que dice que un envío fallido se propaga.

### G2 — El cuerpo no se registra fuera de development

**Qué pasa si se rompe:** el cuerpo lleva el código de una toma de control. Un log con ese código es un código de autenticación en texto plano, en un sistema de logs al que suele tener acceso gente que no debería.

**Qué tiene que hacer la implementación:** registrar los **identificadores** —el destinatario enmascarado, el identificador del proveedor, el resultado— y **nunca** el cuerpo. La regla aplica también a los logs del propio proveedor: el cuerpo no se manda a un servicio de observabilidad externo.

**Cómo se prueba:** el test mira lo que la implementación registra.

### G3 — Es honesto con los fallos: rechazo permanente y falla temporal son cosas distintas

**Qué pasa si se rompe:** un rechazo permanente —un número inexistente, un destino bloqueado— se reporta como falla temporal. El worker responde `500`, el participante reintenta, y el número **nunca va a funcionar**. O al revés: una falla de red se reporta como rechazo y el participante recibe un mensaje que dice "ese número no existe", que es mentira.

| Variante | Cuándo | Qué significa |
|---|---|---|
| `SmsError::Refused(motivo)` | El proveedor rechazó el mensaje de forma permanente: el número no existe, está bloqueado, el remitente no está autorizado. | **No tiene sentido reintentar.** El número no va a funcionar nunca. |
| `SmsError::Channel(motivo)` | El canal falló de una forma que puede ser transitoria: un timeout, un `5xx`, una conexión caída. | **Puede tener sentido reintentar**, y el participante debería poder pedir un código nuevo. |

**Cómo se prueba:** un proveedor de mentira que devuelve cada variante, y dos tests que comprueban que llegan al cliente como errores distintos.

### G4 — No reintenta de una forma que duplique el código

**Qué pasa si se rompe:** la verificación lleva **un** código y **un** hash. Si el proveedor manda el mismo mensaje dos veces, no pasa nada grave. Si el worker genera un código nuevo para el reintento, **el hash guardado deja de corresponder** y el código que el participante tiene en el SMS deja de servir: la verificación se vuelve imposible de completar sin que nada diga por qué.

**Qué tiene que hacer la implementación:** el mensaje recibido manda. El código se genera **una vez**, antes de encolar el envío, y el reintento —si lo hay— reenvía **el mismo** mensaje.

### G5 — Es barata de probar

**Qué pasa si se rompe:** el trait tiene I/O de red, así que probarlo exige un servidor de mentira o llamadas de verdad, y los tests se vuelven lentos, intermitentes y carísimos.

**Qué tiene que hacer la implementación:** el trait **no tiene estado propio ni I/O propio**: recibe el destino y el mensaje, y hace la llamada. Toda la configuración —credenciales, endpoint, tiempo de espera— entra por el constructor. Eso permite que el harness sustituya un grabador.

**El grabador ya existe** en `services/identity/tests/common/mod.rs`: `RecordingSmsSender` registra a quién fue cada mensaje y con qué cuerpo. El test que importa es `the_sms_goes_to_the_owner_of_the_token_and_not_to_the_claimant`, y **no depende del proveedor**: la implementación real tiene que pasar el mismo harness.

---

## 4. La configuración

### 4.1 El selector

El proveedor se elige con una variable de entorno, igual que las otras decisiones del despliegue:

```rust
pub enum SmsProvider {
    /// The sender of development: it writes the message to the log.
    Logging,
}
```

| Variable | Valores | Defecto |
|---|---|---|
| `SMS_PROVIDER` | `logging` (hoy), el nombre del proveedor real (a definir) | `logging` |

**Un valor desconocido es un error de arranque**, no un defaults silencioso:

```
ConfigError::UnknownSmsProvider("twilio")
```

### 4.2 El chequeo de arranque

```rust
ConfigError::DevSmsOutsideDevelopment
```

> "the SMS sender of development writes the codes of the users in the log and that is only allowed in development"

El chequeo es **un fallo de arranque, no una advertencia**, y ocurre en la configuración, donde ya se leen el resto de los secretos.

**El caso que hay que decidir antes del despliegue:** ¿el despliegue de *staging* tiene que poder arrancar con el emisor de log?

| Opción | Consecuencia |
|---|---|
| **Staging exige el proveedor real** (lo que hay hoy) | Staging no se despliega hasta que el proveedor esté. Es lo seguro. |
| **Staging acepta el emisor de log** | Staging despliega antes, y los códigos de los usuarios de prueba quedan en el log de staging. Aceptable si los números de prueba no son reales. |

La decisión es del despliegue, no del código. Si cambia, el cambio es una línea en `services/identity/src/config.rs` y un test.

### 4.3 Las credenciales

Vienen del **secret manager**, nunca del repositorio, y se leen de la entorno como `JWT_SECRET`:

| Variable | Qué es |
|---|---|
| `SMS_API_KEY` | La credencial del proveedor. |
| `SMS_SENDER_ID` | El remitente que el proveedor exige, si lo exige. |
| `SMS_PROVIDER` | Qué proveedor es. |

El `.env.example` documenta las tres, con el valor de desarrollo.

---

## 5. Qué tiene que implementar el proveedor, en código

### 5.1 La estructura

```
services/identity/src/adapters/
├── mod.rs
├── sms.rs              # el emisor de desarrollo, hoy
└── sms_<proveedor>.rs  # el proveedor real
```

Y el **`SmsProvider` gana una variante**:

```rust
pub enum SmsProvider {
    /// The sender of development: it writes the message to the log.
    Logging,
    /// The real provider.
    <Proveedor>,
}
```

### 5.2 El esqueleto de la implementación

```rust
/// The SMS sender of <proveador>.
#[derive(Debug, Clone)]
pub struct <Proveedor>SmsSender {
    client: <Cliente>,
    sender_id: String,
}

impl SmsSender for <Proveedor>SmsSender {
    fn send(&self, to: &str, message: &SmsMessage) -> BoxFuture<'_, Result<(), SmsError>> {
        let client = self.client.clone();
        let sender_id = self.sender_id.clone();
        let to = to.to_string();
        let body = message.body.clone();
        Box::pin(async move {
            // 1. el envío, y su traducción al fallo
            //    - el proveedor lo rechaza de forma permanente → SmsError::Refused
            //    - el canal falla de forma transitoria      → SmsError::Channel
            // 2. NUNCA el cuerpo en un log
            // 3. NUNCA Ok si no se entregó
        })
    }
}
```

### 5.3 El mapeo de los fallos del proveedor

Cada proveedor tiene su propio vocabulario de errores. El mapeo al `SmsError` del puerto es **explícito, caso por caso**, y tiene su test:

| Lo que dice el proveedor | `SmsError` |
|---|---|
| Número inexistente, bloqueado, destinatario inválido | `Refused` |
| Credenciales inválidas, cuenta suspendida | `Refused` — no tiene sentido reintentar |
| `5xx`, timeout, error de red, `429` (cuota) | `Channel` |
| Respuesta con cuerpo inesperado | `Channel` |
| `Ok` sin identificador de envío | **No se puede aceptar**: es G1 |

Un `429` de cuota es `Channel` y no `Refused`: el mensaje **puede** salir más tarde.

---

## 6. Los tests que tiene que traer la implementación

El harness ya existe y **no hay que escribirlo**:

| Pieza | Dónde está | Qué sirve |
|---|---|---|
| `RecordingSmsSender` | `services/identity/tests/common/mod.rs` | Registra a quién fue cada mensaje y con qué cuerpo |
| `app_pool` | el mismo | El pool con el rol de la aplicación, que es donde corren los tests del flujo |
| `the_sms_goes_to_the_owner_of_the_token_and_not_to_the_claimant` | `services/identity/tests/identity_http.rs` | La prueba de que el mensaje va al dueño, no al reclamante |

**Los tests que la implementación tiene que agregar**, y que corren con el harness, no con el proveedor:

| Test | Qué prueba |
|---|---|
| `the_provider_receives_the_body_and_the_recipient` | Que el `to` y el `body` llegan tal cual |
| `a_refused_message_is_not_a_channel_failure` | Que un rechazo permanente se traduce a `Refused` |
| `a_channel_failure_is_not_a_refusal` | Que una falla transitoria se traduce a `Channel` |
| `the_implementation_does_not_log_the_body` | G2, sobre lo que la implementación registra |
| `the_implementation_does_not_return_ok_without_delivering` | G1, con un proveedor de mentira |

Los dos últimos usan un proveedor de mentira: una implementación del trait que no hace nada y devuelve `Ok`, y otra que registra mal. **El test tiene que fallar contra ellas.**

---

## 7. Criterios de aceptación

- [ ] Hay una implementación real de `SmsSender`, en su propio módulo de `adapters/`.
- [ ] `SmsProvider` gana la variante, y `SMS_PROVIDER` la selecciona.
- [ ] Un valor desconocido de `SMS_PROVIDER` es un error de arranque.
- [ ] Las credenciales vienen del secret manager, y ninguna está en el repositorio.
- [ ] El chequeo de arranque sigue rechazando el emisor de log fuera de development, o la decisión de staging está tomada y documentada.
- [ ] Un rechazo permanente y una falla temporal se traducen a `Refused` y `Channel`, con un test cada uno.
- [ ] La implementación **no registra el cuerpo** fuera de development, y hay un test que lo comprueba.
- [ ] La implementación **no devuelve `Ok` sin entregar**, y hay un test que lo comprueba.
- [ ] El mensaje que se reenvía es **el mismo**, con el mismo código.
- [ ] Los tests del flujo (`the_sms_goes_to_the_owner_of_the_token_and_not_to_the_claimant` y los demás) siguen verdes con el proveedor real detrás del puerto.
- [ ] `docs/architecture.md` §8.3 dice qué hace falta para desplegar la identidad fuera de development, y la lista ya no incluye el proveedor de SMS.

---

## 8. Las preguntas que hay que responder antes de elegir proveedor

No son técnicas, y cambian la implementación:

| Pregunta | Por qué importa |
|---|---|
| **¿Qué volumen y a qué costo?** | El precio por SMS varía por país y por volumen. El modelo es de sorteos locales en Argentina: el volumen es bajo, pero el costo por mensaje es alto. |
| **¿Qué países?** | El remitente al destinatario tiene que funcionar en el país donde está el comercio. |
| **¿Se puede enviar a cualquier número, o hay una lista de remitentes validados?** | Algunos proveedores exigen verificar el remitente antes de poder enviar. Eso es una tarea con plazo. |
| **¿Se puede entregar con prefijado de país?** | El número en ficha es E.164. Si el proveedor necesita otra cosa, la traducción va en la implementación, no en el flujo. |
| **¿Quién lee los fallos y cómo se responde?** | Un rechazo permanente y una caída del canal quieren respuestas distintas. |
| **¿Hay un límite de reintentos por mensaje?** | G4 dice que el reintento reenvía el mismo mensaje. El límite lo pone el proveedor. |

---

## 9. Lo que **no** tiene que hacer el proveedor

Para que quede claro, y porque es tentador meterlo ahí:

| No | Por qué |
|---|---|
| No genera el código | El código se genera con el CSPRNG del worker y se guarda hasheado. Si lo generara el proveedor, el hash quedaría en un sistema de terceros. |
| No decide el destinatario | La decisión es del worker y es **la** decisión de seguridad del flujo. |
| No guarda el código ni el cuerpo | El proveedor ve el mensaje en tránsito y en su propia base, y no debería duplicar el código en una base de códigos. |
| No decide cuándo vence la verificación | El tiempo de vida es una regla del dominio: `DEVICE_VERIFICATION_TTL`. |
| No lleva la cuenta de los intentos | Es una regla del dominio, y tiene que sobrevivir al reinicio del worker. |
| No implementa la lógica de la toma de control | El flujo es del dominio, con sus tests. El proveedor es un canal. |
