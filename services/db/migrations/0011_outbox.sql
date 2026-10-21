-- Transactional outbox: la cola de mensajes de la plataforma.
--
-- No confundir con `notifications`: acá vive el evento que un proceso publica
-- en la cola de iii, no lo que el usuario recibe.

CREATE TABLE outbox (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    topic         text NOT NULL,
    payload       jsonb NOT NULL,
    published_at  timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE outbox IS
    'Transactional outbox. El evento se escribe en la MISMA transacción que confirma el sorteo, así "el sorteo se ejecutó" y "se encoló la notificación" son atómicos.';

CREATE INDEX outbox_pending_idx ON outbox (created_at) WHERE published_at IS NULL;
