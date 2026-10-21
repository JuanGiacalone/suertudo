-- Participants and their devices.
--
-- `users` is deliberately NOT protected by Row Level Security: a participant
-- is a person, not a tenant resource (docs/data-model.md §7.3). Isolation for
-- business data lives on the tables that carry a `business_id`.

CREATE TABLE users (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    email             citext NOT NULL UNIQUE,
    phone_e164        text UNIQUE,
    display_name      text NOT NULL,
    password_hash     text,                       -- NULL para magic link / SSO
    role              text NOT NULL DEFAULT 'participant'
                      CHECK (role IN ('participant','business_owner','platform_admin')),
    status            text NOT NULL DEFAULT 'active'
                      CHECK (status IN ('active','pending_review','suspended','deleted')),
    locale            text NOT NULL DEFAULT 'es-AR',
    timezone          text NOT NULL DEFAULT 'America/Argentina/Buenos_Aires',
    marketing_opt_in  boolean NOT NULL DEFAULT false,
    -- Consentimiento granular por negocio: ver consent_records
    last_login_at     timestamptz,
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now(),
    deleted_at        timestamptz
);

COMMENT ON TABLE users IS
    'Participantes. `marketing_opt_in` es el default global; el consentimiento por negocio vive en consent_records.';
COMMENT ON COLUMN users.status IS
    'pending_review es el estado de las cuentas creadas en masa (FR-PA-10): no pueden reservar números.';
COMMENT ON COLUMN users.deleted_at IS
    'Soft delete: la baja se anonimiza 30 días después (docs/data-model.md §8).';

-- El índice de expresión es el que garantiza la unicidad real del correo: la
-- constraint UNIQUE de la columna existe por claridad del DDL, esta es la que
-- impide dos cuentas con el mismo correo en distinta capitalización.
CREATE UNIQUE INDEX users_email_lower_idx ON users (lower(email));
CREATE INDEX users_status_idx ON users (status) WHERE deleted_at IS NULL;

CREATE TABLE push_devices (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    platform    text NOT NULL CHECK (platform IN ('android','ios','web')),
    token       text NOT NULL UNIQUE,
    app_version text,
    active      boolean NOT NULL DEFAULT true,
    created_at  timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz
);

COMMENT ON TABLE push_devices IS
    'Dispositivos push. El token es único globalmente: un token no puede apuntar a dos usuarios.';

CREATE INDEX push_devices_user_idx ON push_devices (user_id) WHERE active;
