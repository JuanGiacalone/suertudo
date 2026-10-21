-- Auditoría. Append-only: no hay UPDATE ni DELETE sobre esta tabla.

CREATE TABLE audit_log (
    id            bigserial PRIMARY KEY,
    business_id   uuid REFERENCES businesses(id) ON DELETE SET NULL,
    actor_user_id uuid REFERENCES users(id) ON DELETE SET NULL,
    actor_type    text NOT NULL CHECK (actor_type IN ('user','system','api_key')),
    action        text NOT NULL,        -- 'raffle.published','participants.exported','theme.activated',...
    entity_type   text NOT NULL,
    entity_id     text NOT NULL,
    before        jsonb,
    after         jsonb,
    metadata      jsonb NOT NULL DEFAULT '{}',
    ip_hash       text,
    created_at    timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE audit_log IS
    'Bitácora de acciones. Append-only por reglas de Postgres: el registro de una acción no se puede reescribir.';

CREATE INDEX audit_log_business_idx ON audit_log (business_id, created_at DESC);
CREATE INDEX audit_log_entity_idx ON audit_log (entity_type, entity_id);
CREATE INDEX audit_log_actor_idx ON audit_log (actor_user_id, created_at DESC);

-- Las reglas convierten UPDATE y DELETE en un no-op silencioso: la fila que
-- estaba no se toca y el llamador ve cero filas afectadas.
CREATE RULE audit_log_no_update AS ON UPDATE TO audit_log DO INSTEAD NOTHING;
CREATE RULE audit_log_no_delete AS ON DELETE TO audit_log DO INSTEAD NOTHING;
