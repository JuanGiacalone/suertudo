-- El sorteo: la tabla que sostiene el producto.

CREATE TABLE raffles (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id       uuid NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    public_id         uuid NOT NULL DEFAULT gen_random_uuid() UNIQUE,  -- id público (share link)
    slug              text NOT NULL,                                  -- legible, único por business
    title             text NOT NULL,
    subtitle          text,
    description       text,
    rules_md          text,
    -- Premio
    prize_name        text NOT NULL,
    prize_description text,
    prize_quantity    integer NOT NULL DEFAULT 1 CHECK (prize_quantity > 0),
    prize_delivery_instructions text,
    prize_image_url   text,
    category          text NOT NULL DEFAULT 'other',
    -- Configuración del sorteo
    status            text NOT NULL DEFAULT 'draft'
                      CHECK (status IN ('draft','published','closed','drawn','settled','void','archived')),
    participation_mode text NOT NULL DEFAULT 'open'
                      CHECK (participation_mode IN ('open','validated')),
    draw_source       text NOT NULL DEFAULT 'own_draw'
                      CHECK (draw_source IN ('own_draw','external_lottery')),
    draw_unit         text NOT NULL DEFAULT 'number'
                      CHECK (draw_unit IN ('number','ticket')),
    no_winner_policy  text NOT NULL DEFAULT 'redraw'
                      CHECK (no_winner_policy IN ('redraw','shared_prize','void')),
    number_pool_size  integer NOT NULL DEFAULT 1000 CHECK (number_pool_size BETWEEN 10 AND 1000000),
    numbers_per_ticket_min integer NOT NULL DEFAULT 1 CHECK (numbers_per_ticket_min >= 1),
    numbers_per_ticket_max integer NOT NULL DEFAULT 5,
    max_tickets_per_user   integer NOT NULL DEFAULT 1 CHECK (max_tickets_per_user >= 1),
    min_coverage_pct numeric(5,2) NOT NULL DEFAULT 0,
    -- Fechas
    opens_at          timestamptz,
    closes_at         timestamptz,
    draws_at          timestamptz,
    published_at      timestamptz,
    closed_at         timestamptz,
    drawn_at          timestamptz,
    settled_at        timestamptz,
    -- Presentación
    theme_id          uuid,   -- FK agregada en 0007 (theme_versions es posterior)
    cover_image_url   text,
    -- Métricas desnormalizadas (mantenidas por el worker de draws)
    tickets_count     integer NOT NULL DEFAULT 0,
    numbers_reserved  integer NOT NULL DEFAULT 0,
    -- Control
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now(),
    deleted_at        timestamptz,
    UNIQUE (business_id, slug),
    CONSTRAINT raffles_dates_ck CHECK (
        (closes_at IS NULL OR draws_at IS NULL) OR draws_at >= closes_at
    )
);

COMMENT ON TABLE raffles IS
    'Sorteos. `public_id` es el que va en los links públicos: el id interno puede filtrar volumen y fechas.';
COMMENT ON COLUMN raffles.draw_source IS
    'Costura para el adaptador de lotería externa (Fase 3). El campo existe desde el día 1 para no migrar datos después (R3 del PRD).';
COMMENT ON COLUMN raffles.tickets_count IS
    'Métrica desnormalizada: el feed muestra "quedan N números" sin un COUNT por card.';

CREATE INDEX raffles_status_closes_idx ON raffles (status, closes_at) WHERE deleted_at IS NULL;
CREATE INDEX raffles_business_idx ON raffles (business_id, created_at DESC) WHERE deleted_at IS NULL;
CREATE INDEX raffles_draws_at_idx ON raffles (draws_at) WHERE status = 'closed';
CREATE INDEX raffles_category_idx ON raffles (category, status) WHERE deleted_at IS NULL;
