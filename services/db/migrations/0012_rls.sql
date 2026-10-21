-- Row Level Security: la segunda barrera del aislamiento multi-tenant (ADR-007).
--
-- La aplicación autoriza; la base garantiza. Una sesión sin `app.business_id`
-- no matchea ninguna política y no lee filas: el fallo es cerrado a propósito.
--
-- Las políticas leen los GUC que `with_tenant` define con `SET LOCAL`. Cuando
-- el GUC no está definido, `current_setting(..., true)` devuelve NULL y la
-- comparación es NULL, o sea, falsa.

-- El estado del tenant, en un solo lugar.
CREATE OR REPLACE FUNCTION current_business_id() RETURNS uuid
    LANGUAGE sql STABLE AS $$
    SELECT NULLIF(current_setting('app.business_id', true), '')::uuid
$$;

CREATE OR REPLACE FUNCTION current_user_id() RETURNS uuid
    LANGUAGE sql STABLE AS $$
    SELECT NULLIF(current_setting('app.user_id', true), '')::uuid
$$;

-- Comercios: cada negocio ve solamente el suyo.
ALTER TABLE businesses ENABLE ROW LEVEL SECURITY;
CREATE POLICY businesses_isolation ON businesses
    USING (id = current_business_id());

ALTER TABLE business_members ENABLE ROW LEVEL SECURITY;
CREATE POLICY business_members_isolation ON business_members
    USING (business_id = current_business_id() OR user_id = current_user_id());

-- Sorteos: el dueño del negocio ve los suyos; los publicados son públicos,
-- porque el feed y el link de participación son de acceso público.
ALTER TABLE raffles ENABLE ROW LEVEL SECURITY;
CREATE POLICY raffles_isolation ON raffles
    USING (
        business_id = current_business_id()
        OR (status = 'published' AND deleted_at IS NULL)
    );

-- Tickets: el negocio ve los suyos, el participante ve los propios.
ALTER TABLE tickets ENABLE ROW LEVEL SECURITY;
CREATE POLICY tickets_isolation ON tickets
    USING (
        business_id = current_business_id()
        OR user_id = current_user_id()
    );

-- Los números se heredan del ticket: un número reservado es dato del
-- participante, no del sorteo.
ALTER TABLE ticket_numbers ENABLE ROW LEVEL SECURITY;
CREATE POLICY ticket_numbers_isolation ON ticket_numbers
    USING (EXISTS (SELECT 1 FROM tickets t WHERE t.id = ticket_numbers.ticket_id));

-- El draw de un sorteo publicado y revelado es público: la verificación
-- commit-reveal es el producto, no un dato del comercio.
ALTER TABLE draws ENABLE ROW LEVEL SECURITY;
CREATE POLICY draws_isolation ON draws
    USING (
        business_id = current_business_id()
        OR seed_revealed = true
    );

ALTER TABLE prizes ENABLE ROW LEVEL SECURITY;
CREATE POLICY prizes_isolation ON prizes
    USING (
        business_id = current_business_id()
        OR user_id = current_user_id()
    );

ALTER TABLE theme_versions ENABLE ROW LEVEL SECURITY;
CREATE POLICY theme_versions_isolation ON theme_versions
    USING (business_id = current_business_id());

ALTER TABLE ai_generations ENABLE ROW LEVEL SECURITY;
CREATE POLICY ai_generations_isolation ON ai_generations
    USING (business_id = current_business_id());

ALTER TABLE notifications ENABLE ROW LEVEL SECURITY;
CREATE POLICY notifications_isolation ON notifications
    USING (user_id = current_user_id());

ALTER TABLE consent_records ENABLE ROW LEVEL SECURITY;
CREATE POLICY consent_records_isolation ON consent_records
    USING (
        business_id = current_business_id()
        OR user_id = current_user_id()
    );

ALTER TABLE validation_codes ENABLE ROW LEVEL SECURITY;
CREATE POLICY validation_codes_isolation ON validation_codes
    USING (business_id = current_business_id());
