-- Las políticas de escritura: el `USING` no alcanza.
--
-- Una política sin `WITH CHECK` usa el `USING` también para el `INSERT` y el
-- `UPDATE`. Las ramas públicas de lectura (un sorteo `published`, un draw con
-- `seed_revealed`) se convertían entonces en permisos de escritura: con el
-- contexto del comercio A, una fila del comercio B se insertaba o se
-- modificaba sin problema.
--
-- Esta migración separa las políticas de lectura de las de escritura:
--
--   * las políticas `SELECT` conservan las ramas públicas (el feed y la
--     verificación pública del sorteo tienen que seguir funcionando);
--   * las políticas `INSERT`, `UPDATE` y `DELETE` tienen un `WITH CHECK`
--     explícito que sólo permite escribir filas del propio tenant.
--
-- El contexto de sistema (un job que no es de un tenant: el que cierra los
-- sorteos vencidos, el que ejecuta un sorteo) escribe para cualquier
-- comercio. Es explícito: `with_tenant` deja los dos GUC en cadena vacía,
-- mientras que una transacción sin contexto los deja sin definir.

-- El estado del tenant, en un solo lugar.
CREATE OR REPLACE FUNCTION current_business_id() RETURNS uuid
    LANGUAGE sql STABLE AS $$
    SELECT NULLIF(current_setting('app.business_id', true), '')::uuid
$$;

CREATE OR REPLACE FUNCTION current_user_id() RETURNS uuid
    LANGUAGE sql STABLE AS $$
    SELECT NULLIF(current_setting('app.user_id', true), '')::uuid
$$;

-- `true` sólo cuando los dos GUC están definidos y vacíos: ése es el
-- contexto de sistema. Una transacción sin contexto no lo es.
CREATE OR REPLACE FUNCTION is_system_context() RETURNS boolean
    LANGUAGE sql STABLE AS $$
    SELECT COALESCE(current_setting('app.business_id', true), 'unset') = ''
       AND COALESCE(current_setting('app.user_id', true), 'unset') = ''
$$;

-- --------------------------------------------------------------------------
-- businesses
-- --------------------------------------------------------------------------
-- El soft delete oculta la fila también a su propio comercio: una fila
-- borrada lógicamente no es una fila viva. `raffles` filtra igual.
DROP POLICY IF EXISTS businesses_isolation ON businesses;

CREATE POLICY businesses_select ON businesses
    FOR SELECT
    USING (id = current_business_id() AND deleted_at IS NULL);

-- El alta del comercio la hace el participante que se registra (todavía no
-- tiene un contexto de comercio) o la plataforma.
CREATE POLICY businesses_insert ON businesses
    FOR INSERT
    WITH CHECK (id = current_business_id() OR is_system_context() OR current_user_id() IS NOT NULL);

CREATE POLICY businesses_update ON businesses
    FOR UPDATE
    USING (id = current_business_id())
    WITH CHECK (id = current_business_id());

CREATE POLICY businesses_delete ON businesses
    FOR DELETE
    USING (id = current_business_id());

-- --------------------------------------------------------------------------
-- business_members
-- --------------------------------------------------------------------------
DROP POLICY IF EXISTS business_members_isolation ON business_members;

CREATE POLICY business_members_select ON business_members
    FOR SELECT
    USING (business_id = current_business_id() OR user_id = current_user_id());

CREATE POLICY business_members_insert ON business_members
    FOR INSERT
    WITH CHECK (business_id = current_business_id() OR user_id = current_user_id() OR is_system_context());

CREATE POLICY business_members_update ON business_members
    FOR UPDATE
    USING (business_id = current_business_id() OR user_id = current_user_id())
    WITH CHECK (business_id = current_business_id() OR user_id = current_user_id());

CREATE POLICY business_members_delete ON business_members
    FOR DELETE
    USING (business_id = current_business_id() OR user_id = current_user_id());

-- --------------------------------------------------------------------------
-- raffles
-- --------------------------------------------------------------------------
-- El `USING` del `UPDATE` conserva la rama pública a propósito: la reserva
-- bloquea la fila del sorteo (`SELECT ... FOR UPDATE`) y la reserva la hace
-- un participante, que no es el dueño del sorteo. El `WITH CHECK` es el que
-- impide escribir la fila de otro comercio.
DROP POLICY IF EXISTS raffles_isolation ON raffles;

CREATE POLICY raffles_select ON raffles
    FOR SELECT
    USING (
        (business_id = current_business_id() AND deleted_at IS NULL)
        OR (status = 'published' AND deleted_at IS NULL)
        OR is_system_context()
    );

CREATE POLICY raffles_insert ON raffles
    FOR INSERT
    WITH CHECK (business_id = current_business_id() OR is_system_context());

CREATE POLICY raffles_update ON raffles
    FOR UPDATE
    USING (
        business_id = current_business_id()
        OR (status = 'published' AND deleted_at IS NULL)
        OR is_system_context()
    )
    WITH CHECK (business_id = current_business_id() OR is_system_context());

CREATE POLICY raffles_delete ON raffles
    FOR DELETE
    USING (business_id = current_business_id() OR is_system_context());

-- --------------------------------------------------------------------------
-- tickets
-- --------------------------------------------------------------------------
-- La reserva del participante escribe una fila del comercio del sorteo: el
-- `INSERT` permite el `user_id` del propio participante. La baja de la cuenta
-- anonimiza el ticket (`user_id = NULL`), y para eso hace falta una política
-- de `UPDATE` propia: la fila deja de ser del participante y sigue siendo
-- del comercio, así que la rama del participante no la alcanza.
DROP POLICY IF EXISTS tickets_isolation ON tickets;

CREATE POLICY tickets_select ON tickets
    FOR SELECT
    USING (business_id = current_business_id() OR user_id = current_user_id());

CREATE POLICY tickets_insert ON tickets
    FOR INSERT
    WITH CHECK (business_id = current_business_id() OR user_id = current_user_id() OR is_system_context());

-- El comercio actualiza sus tickets (el sorteo marca al ganador).
CREATE POLICY tickets_update ON tickets
    FOR UPDATE
    USING (business_id = current_business_id() OR is_system_context())
    WITH CHECK (business_id = current_business_id() OR is_system_context());

-- La anonimización: el participante borra su cuenta y sus tickets sobreviven
-- sin dueño (`FR-ID-05`). El `WITH CHECK` exige que la fila quede sin
-- `user_id`: el participante no puede cambiar nada más de su ticket (lo
-- garantiza `tickets_ownership_immutable_trg`) ni quedarse con el de otro.
CREATE POLICY tickets_update_anonymisation ON tickets
    FOR UPDATE
    USING (user_id = current_user_id())
    WITH CHECK (user_id IS NULL);

CREATE POLICY tickets_delete ON tickets
    FOR DELETE
    USING (business_id = current_business_id() OR is_system_context());

-- --------------------------------------------------------------------------
-- ticket_numbers
-- --------------------------------------------------------------------------
-- Los números se heredan del ticket: un número reservado es dato del
-- participante, no del sorteo.
DROP POLICY IF EXISTS ticket_numbers_isolation ON ticket_numbers;

CREATE POLICY ticket_numbers_select ON ticket_numbers
    FOR SELECT
    USING (EXISTS (SELECT 1 FROM tickets t WHERE t.id = ticket_numbers.ticket_id));

CREATE POLICY ticket_numbers_insert ON ticket_numbers
    FOR INSERT
    WITH CHECK (EXISTS (SELECT 1 FROM tickets t WHERE t.id = ticket_numbers.ticket_id));

CREATE POLICY ticket_numbers_update ON ticket_numbers
    FOR UPDATE
    USING (EXISTS (SELECT 1 FROM tickets t WHERE t.id = ticket_numbers.ticket_id))
    WITH CHECK (EXISTS (SELECT 1 FROM tickets t WHERE t.id = ticket_numbers.ticket_id));

CREATE POLICY ticket_numbers_delete ON ticket_numbers
    FOR DELETE
    USING (EXISTS (SELECT 1 FROM tickets t WHERE t.id = ticket_numbers.ticket_id));

-- --------------------------------------------------------------------------
-- draws
-- --------------------------------------------------------------------------
-- El draw de un sorteo publicado y revelado es público: la verificación
-- commit-reveal es el producto, no un dato del comercio.
DROP POLICY IF EXISTS draws_isolation ON draws;

CREATE POLICY draws_select ON draws
    FOR SELECT
    USING (business_id = current_business_id() OR seed_revealed = true OR is_system_context());

CREATE POLICY draws_insert ON draws
    FOR INSERT
    WITH CHECK (business_id = current_business_id() OR is_system_context());

CREATE POLICY draws_update ON draws
    FOR UPDATE
    USING (business_id = current_business_id() OR seed_revealed = true OR is_system_context())
    WITH CHECK (business_id = current_business_id() OR is_system_context());

CREATE POLICY draws_delete ON draws
    FOR DELETE
    USING (business_id = current_business_id() OR is_system_context());

-- --------------------------------------------------------------------------
-- prizes
-- --------------------------------------------------------------------------
DROP POLICY IF EXISTS prizes_isolation ON prizes;

CREATE POLICY prizes_select ON prizes
    FOR SELECT
    USING (business_id = current_business_id() OR user_id = current_user_id());

CREATE POLICY prizes_insert ON prizes
    FOR INSERT
    WITH CHECK (business_id = current_business_id() OR user_id = current_user_id() OR is_system_context());

CREATE POLICY prizes_update ON prizes
    FOR UPDATE
    USING (business_id = current_business_id() OR user_id = current_user_id())
    WITH CHECK (business_id = current_business_id() OR user_id = current_user_id());

CREATE POLICY prizes_delete ON prizes
    FOR DELETE
    USING (business_id = current_business_id() OR is_system_context());

-- --------------------------------------------------------------------------
-- theme_versions
-- --------------------------------------------------------------------------
DROP POLICY IF EXISTS theme_versions_isolation ON theme_versions;

CREATE POLICY theme_versions_select ON theme_versions
    FOR SELECT
    USING (business_id = current_business_id());

CREATE POLICY theme_versions_insert ON theme_versions
    FOR INSERT
    WITH CHECK (business_id = current_business_id() OR is_system_context());

CREATE POLICY theme_versions_update ON theme_versions
    FOR UPDATE
    USING (business_id = current_business_id() OR is_system_context())
    WITH CHECK (business_id = current_business_id() OR is_system_context());

CREATE POLICY theme_versions_delete ON theme_versions
    FOR DELETE
    USING (business_id = current_business_id() OR is_system_context());

-- --------------------------------------------------------------------------
-- ai_generations
-- --------------------------------------------------------------------------
DROP POLICY IF EXISTS ai_generations_isolation ON ai_generations;

CREATE POLICY ai_generations_select ON ai_generations
    FOR SELECT
    USING (business_id = current_business_id());

CREATE POLICY ai_generations_insert ON ai_generations
    FOR INSERT
    WITH CHECK (business_id = current_business_id() OR is_system_context());

CREATE POLICY ai_generations_update ON ai_generations
    FOR UPDATE
    USING (business_id = current_business_id() OR is_system_context())
    WITH CHECK (business_id = current_business_id() OR is_system_context());

CREATE POLICY ai_generations_delete ON ai_generations
    FOR DELETE
    USING (business_id = current_business_id() OR is_system_context());

-- --------------------------------------------------------------------------
-- notifications
-- --------------------------------------------------------------------------
DROP POLICY IF EXISTS notifications_isolation ON notifications;

CREATE POLICY notifications_select ON notifications
    FOR SELECT
    USING (user_id = current_user_id() OR is_system_context());

CREATE POLICY notifications_insert ON notifications
    FOR INSERT
    WITH CHECK (user_id = current_user_id() OR is_system_context());

CREATE POLICY notifications_update ON notifications
    FOR UPDATE
    USING (user_id = current_user_id() OR is_system_context())
    WITH CHECK (user_id = current_user_id() OR is_system_context());

CREATE POLICY notifications_delete ON notifications
    FOR DELETE
    USING (user_id = current_user_id() OR is_system_context());

-- --------------------------------------------------------------------------
-- consent_records
-- --------------------------------------------------------------------------
DROP POLICY IF EXISTS consent_records_isolation ON consent_records;

CREATE POLICY consent_records_select ON consent_records
    FOR SELECT
    USING (business_id = current_business_id() OR user_id = current_user_id());

CREATE POLICY consent_records_insert ON consent_records
    FOR INSERT
    WITH CHECK (business_id = current_business_id() OR user_id = current_user_id() OR is_system_context());

CREATE POLICY consent_records_update ON consent_records
    FOR UPDATE
    USING (business_id = current_business_id() OR user_id = current_user_id())
    WITH CHECK (business_id = current_business_id() OR user_id = current_user_id());

CREATE POLICY consent_records_delete ON consent_records
    FOR DELETE
    USING (business_id = current_business_id() OR user_id = current_user_id());

-- --------------------------------------------------------------------------
-- validation_codes
-- --------------------------------------------------------------------------
DROP POLICY IF EXISTS validation_codes_isolation ON validation_codes;

CREATE POLICY validation_codes_select ON validation_codes
    FOR SELECT
    USING (business_id = current_business_id());

CREATE POLICY validation_codes_insert ON validation_codes
    FOR INSERT
    WITH CHECK (business_id = current_business_id() OR is_system_context());

CREATE POLICY validation_codes_update ON validation_codes
    FOR UPDATE
    USING (business_id = current_business_id() OR is_system_context())
    WITH CHECK (business_id = current_business_id() OR is_system_context());

CREATE POLICY validation_codes_delete ON validation_codes
    FOR DELETE
    USING (business_id = current_business_id() OR is_system_context());

-- --------------------------------------------------------------------------
-- audit_log
-- --------------------------------------------------------------------------
-- La bitácora no era sólo append-only: tampoco estaba aislada. Una conexión
-- con el rol de la aplicación leía el diff de auditoría de todos los
-- comercios.
--
-- Una fila es visible para el comercio al que pertenece. Una fila sin
-- comercio (una acción de plataforma, la baja de una cuenta) sólo es visible
-- para el contexto de sistema.
ALTER TABLE audit_log ENABLE ROW LEVEL SECURITY;

CREATE POLICY audit_log_select ON audit_log
    FOR SELECT
    USING (is_system_context() OR business_id = current_business_id());

-- Una fila sin comercio la escribe un job de plataforma o la baja de una
-- cuenta (el participante no tiene contexto de comercio en ese momento).
CREATE POLICY audit_log_insert ON audit_log
    FOR INSERT
    WITH CHECK (is_system_context() OR business_id = current_business_id() OR business_id IS NULL);

-- El `UPDATE` sólo lo necesita la acción referencial `ON DELETE SET NULL` de
-- las foreign keys: la fila pasa a no tener comercio. El trigger append-only
-- (`audit_log_append_only_trg`) es el que impide reescribir la bitácora.
CREATE POLICY audit_log_update ON audit_log
    FOR UPDATE
    USING (true)
    WITH CHECK (true);

CREATE POLICY audit_log_delete ON audit_log
    FOR DELETE
    USING (false);
