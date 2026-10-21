-- Comercios and the people who work for them.

CREATE TABLE businesses (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    slug              text NOT NULL UNIQUE,
    name              text NOT NULL,
    tax_id            text,                    -- CUIT, ver SUPUESTO-3
    category          text NOT NULL,           -- taxonomy libre v1: 'restaurant','cafe','beauty','other'
    description       text,
    logo_url          text,
    cover_url         text,
    -- Tokens de marca: anclas fijas que la IA no puede tocar
    brand_primary     text NOT NULL DEFAULT '#1E5F74',
    brand_secondary   text NOT NULL DEFAULT '#F2A65A',
    brand_font_display text NOT NULL DEFAULT 'serif',
    brand_font_body    text NOT NULL DEFAULT 'sans',
    address_line      text,
    city              text,
    province          text,
    -- Mercado
    plan              text NOT NULL DEFAULT 'starter'
                      CHECK (plan IN ('starter','pro','business')),
    plan_status       text NOT NULL DEFAULT 'active'
                      CHECK (plan_status IN ('active','past_due','cancelled')),
    plan_renews_at    timestamptz,
    -- Moderación
    status            text NOT NULL DEFAULT 'active'
                      CHECK (status IN ('pending','active','suspended','rejected')),
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now(),
    deleted_at        timestamptz
);

COMMENT ON TABLE businesses IS
    'Comercios. Los tokens de marca son columnas y no un JSON: son el contrato con la IA (FR-AI-03) y con el renderer.';
COMMENT ON COLUMN businesses.plan IS
    'El MVP no cobra automáticamente (SUPUESTO-3); la tabla subscriptions llega en Fase 2 sin romper nada.';

CREATE INDEX businesses_status_idx ON businesses (status) WHERE deleted_at IS NULL;
CREATE INDEX businesses_plan_idx ON businesses (plan, plan_status);

CREATE TABLE business_members (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id  uuid NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    user_id      uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role         text NOT NULL CHECK (role IN ('owner','admin','staff','analyst')),
    status       text NOT NULL DEFAULT 'active'
                 CHECK (status IN ('invited','active','revoked')),
    invited_by   uuid REFERENCES users(id),
    created_at   timestamptz NOT NULL DEFAULT now(),
    UNIQUE (business_id, user_id)
);

COMMENT ON TABLE business_members IS
    'Membresía de un usuario en un comercio. La unicidad (business_id, user_id) impide dos membresías del mismo usuario.';

CREATE INDEX business_members_user_idx ON business_members (user_id) WHERE status = 'active';
