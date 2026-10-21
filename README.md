# Suertudo

**Sorteos promocionales para comercios locales.** El participante reserva
números gratis en el sorteo de un negocio, el negocio financia el premio y el
resultado se sortea con un mecanismo que cualquier tercero puede verificar.

## Qué hay que leer primero

| Documento | Para qué |
|---|---|
| [`docs/PRD.md`](docs/PRD.md) | El producto: problema, personas, alcance, 103 requisitos |
| [`docs/architecture.md`](docs/architecture.md) | Cómo está construido y por qué |
| [`docs/roadmap.md`](docs/roadmap.md) | Qué se construye ahora |
| [`docs/spike-iii-health.md`](docs/spike-iii-health.md) | Por qué el stack fue medido en vez de asumido |
| [`AGENTS.md`](AGENTS.md) | **El contrato de trabajo.** Leelo antes de escribir código |

## Stack

| Capa | Tecnología |
|---|---|
| App de participantes | Flutter 3.47 (Android + iOS) |
| Panel de comercios | Next.js 16 · React 19 · TypeScript |
| Backend | Rust 1.99 · sqlx · Postgres 16 |
| Orquestación | iii 0.24.6 (engine y SDK pineados juntos) |
| IA | LLM externo con validación determinista en servidor |

## Puesta en marcha

```bash
# Servicios de datos
docker compose -f infra/docker-compose.yml up -d

# Base de datos
createdb suertudo && createdb suertudo_test

# El smoke test del backend: levanta el stack completo sin KVM
unshare -rm ./scripts/smoke.sh
```

## Estructura

```
apps/mobile/         Flutter — participantes
apps/business-web/   Next.js — comercios
services/            Rust — un crate por worker de iii
infra/               iii, docker-compose
docs/                PRD, contratos, arquitectura, roadmap
spikes/              experimentos que respondieron una pregunta
```

## Estado

Fase 0 (Cimientos) en curso. Ver [`docs/roadmap.md`](docs/roadmap.md).

## Licencia

MIT
