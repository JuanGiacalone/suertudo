-- El resultado del sorteo y los premios que genera.

CREATE TABLE draws (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    raffle_id         uuid NOT NULL UNIQUE REFERENCES raffles(id) ON DELETE CASCADE,
    business_id       uuid NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    -- Compromiso (público desde la publicación)
    seed_commitment   text NOT NULL,           -- hex(SHA-256(seed))
    algorithm         text NOT NULL DEFAULT 'hmac-sha256/v1',
    derivation_spec   jsonb NOT NULL,          -- { message_template, hash, mod }
    -- Semilla (cifrada hasta el revelado)
    seed_ciphertext   bytea NOT NULL,          -- AES-256-GCM
    seed_revealed     boolean NOT NULL DEFAULT false,
    revealed_at       timestamptz,
    -- Resultado
    draw_index        integer NOT NULL DEFAULT 0,
    attempts          integer NOT NULL DEFAULT 0,
    pool_size         integer NOT NULL,
    winning_numbers   integer[] NOT NULL DEFAULT '{}',
    status            text NOT NULL DEFAULT 'pending'
                      CHECK (status IN ('pending','executed','failed','voided')),
    executed_at       timestamptz,
    executed_by       uuid REFERENCES users(id),   -- NULL si fue automático
    execution_mode    text NOT NULL DEFAULT 'auto'
                      CHECK (execution_mode IN ('auto','manual','redraw')),
    error             text,
    created_at        timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE draws IS
    'El sorteo ejecutado. UNIQUE (raffle_id) garantiza FR-DR-01: un sorteo se sortea una sola vez, sin importar cuántas ejecuciones concurrentes lleguen.';
COMMENT ON COLUMN draws.seed_commitment IS
    'hex(SHA-256(seed)). Se escribe al publicar el sorteo y es público desde ese instante.';
COMMENT ON COLUMN draws.seed_ciphertext IS
    'La misma semilla, cifrada con AES-256-GCM. La clave vive en el secret manager, nunca en la base.';
COMMENT ON COLUMN draws.derivation_spec IS
    'La fórmula aplicada (message_template, hash, mod). Cada sorteo conserva la suya, así la verificación pública sigue siendo exacta aunque la fórmula cambie.';

CREATE INDEX draws_raffle_idx ON draws (raffle_id);
CREATE INDEX draws_status_idx ON draws (status) WHERE status = 'pending';

CREATE TABLE prizes (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    raffle_id         uuid NOT NULL REFERENCES raffles(id) ON DELETE CASCADE,
    draw_id           uuid NOT NULL REFERENCES draws(id) ON DELETE CASCADE,
    ticket_id         uuid REFERENCES tickets(id),
    user_id           uuid REFERENCES users(id) ON DELETE SET NULL,
    business_id       uuid NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    prize_name        text NOT NULL,
    prize_description text,
    winning_number    integer,
    status            text NOT NULL DEFAULT 'pending'
                      CHECK (status IN ('pending','claimed','delivered','expired','unassigned','voided')),
    unassigned_reason text,                     -- 'no_winner','no_prize_remaining'
    claim_code        text UNIQUE,              -- código que el ganador usa para canjear en el local
    claimed_at        timestamptz,
    delivered_at      timestamptz,
    delivery_notes    text,
    delivery_evidence_url text,
    expires_at        timestamptz,
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE prizes IS
    'Premios y su entrega. `claim_code` es el nexo entre el mundo físico y el digital: permite al comercio demostrar que entregó.';
COMMENT ON COLUMN prizes.unassigned_reason IS
    'Un sorteo puede tener más premios que ganadores; el sobrante queda explícito en vez de desaparecer (FR-RA-08).';

CREATE UNIQUE INDEX prizes_raffle_position_idx ON prizes (raffle_id, prize_name)
    WHERE status <> 'voided';
CREATE INDEX prizes_user_idx ON prizes (user_id, status) WHERE user_id IS NOT NULL;
CREATE INDEX prizes_business_status_idx ON prizes (business_id, status);
CREATE INDEX prizes_expiry_idx ON prizes (expires_at) WHERE status = 'pending';
