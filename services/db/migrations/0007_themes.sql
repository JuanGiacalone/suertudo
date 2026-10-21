-- Temas visuales y trazabilidad de las generaciones de IA.

CREATE TABLE theme_versions (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id   uuid NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    raffle_id     uuid REFERENCES raffles(id) ON DELETE CASCADE,   -- NULL = tema de negocio
    version       integer NOT NULL,
    source        text NOT NULL CHECK (source IN ('ai','manual','default','preset')),
    prompt        text,
    prompt_hash   text,
    spec          jsonb NOT NULL,          -- theme_spec validado
    warnings      jsonb NOT NULL DEFAULT '[]',
    is_active     boolean NOT NULL DEFAULT false,
    model         text,                    -- modelo usado, si source = 'ai'
    created_by    uuid REFERENCES users(id),
    created_at    timestamptz NOT NULL DEFAULT now(),
    UNIQUE (raffle_id, version)
);

COMMENT ON TABLE theme_versions IS
    'Temas versionados e inmutables. Un sorteo apunta a una versión concreta, no a un "tema actual", para que sea reproducible.';
COMMENT ON COLUMN theme_versions.spec IS
    'JSONB validado contra el JSON Schema de ai-spec.md ANTES de escribir: un spec inválido nunca entra a la base.';

CREATE INDEX theme_versions_raffle_idx ON theme_versions (raffle_id, version DESC);
CREATE UNIQUE INDEX theme_versions_one_active_idx
    ON theme_versions (raffle_id) WHERE is_active AND raffle_id IS NOT NULL;

-- La FK de raffles hacia theme_versions cierra el ciclo: se agrega acá porque
-- theme_versions depende de raffles y las dos referencias no pueden existir en
-- el mismo CREATE TABLE.
ALTER TABLE raffles
    ADD CONSTRAINT raffles_theme_fk FOREIGN KEY (theme_id) REFERENCES theme_versions(id);

CREATE TABLE ai_generations (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id    uuid NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    raffle_id      uuid REFERENCES raffles(id) ON DELETE CASCADE,
    theme_version_id uuid REFERENCES theme_versions(id),
    prompt         text NOT NULL,
    prompt_hash    text NOT NULL,
    model          text NOT NULL,
    status         text NOT NULL DEFAULT 'pending'
                   CHECK (status IN ('pending','running','succeeded','failed','rejected')),
    attempt_count  integer NOT NULL DEFAULT 0,
    tokens_in      integer,
    tokens_out     integer,
    cost_usd       numeric(10,6),
    latency_ms     integer,
    validation_errors jsonb,
    error_code     text,
    created_at     timestamptz NOT NULL DEFAULT now(),
    completed_at   timestamptz
);

COMMENT ON TABLE ai_generations IS
    'Trazabilidad de la IA. Existe por dos motivos: cache de generaciones (FR-AI-10) y auditoría de calidad (NFR-AI-02, NFR-AI-03).';
COMMENT ON COLUMN ai_generations.validation_errors IS
    'Por qué se rechazó la generación. Sin esto no se puede responder "¿por qué la IA falló?" cuando el comercio reporta un tema raro.';

CREATE INDEX ai_generations_hash_idx ON ai_generations (prompt_hash, created_at DESC);
CREATE INDEX ai_generations_status_idx ON ai_generations (status) WHERE status IN ('pending','running');
