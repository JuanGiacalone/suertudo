-- Consentimiento granular y códigos de validación de un solo uso.

CREATE TABLE consent_records (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    business_id  uuid NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    scope        text NOT NULL CHECK (scope IN ('share_marketing','share_contact','participation')),
    granted      boolean NOT NULL,
    source       text NOT NULL CHECK (source IN ('signup','participation','settings','business_message')),
    ip_hash      text,
    user_agent   text,
    created_at   timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE consent_records IS
    'Append-only: cada cambio de preferencia agrega una fila y el estado actual es la última por (user, business, scope). Guardar solo el booleano perdería la trazabilidad de FR-PR-02.';

CREATE INDEX consent_records_lookup_idx ON consent_records (user_id, business_id, scope, created_at DESC);
CREATE INDEX consent_records_business_idx ON consent_records (business_id) WHERE granted AND scope = 'share_marketing';

CREATE TABLE validation_codes (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id  uuid NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    raffle_id    uuid NOT NULL REFERENCES raffles(id) ON DELETE CASCADE,
    code         text NOT NULL UNIQUE,
    status       text NOT NULL DEFAULT 'unused'
                 CHECK (status IN ('unused','used','revoked')),
    used_at      timestamptz,
    used_by_ticket_id uuid REFERENCES tickets(id),
    created_at   timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE validation_codes IS
    'Códigos de un solo uso para sorteos con participation_mode = ''validated''.';

CREATE INDEX validation_codes_raffle_idx ON validation_codes (raffle_id, status);

-- La FK de tickets hacia validation_codes se cierra acá: la dependencia va en
-- sentido contrario al orden de los archivos.
ALTER TABLE tickets
    ADD CONSTRAINT tickets_validation_code_fk FOREIGN KEY (validation_code_id)
    REFERENCES validation_codes(id);
