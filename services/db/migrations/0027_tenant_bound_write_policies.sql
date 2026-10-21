-- Cada política de escritura está atada al dueño de la fila.
--
-- Una política de escritura que dice `OR user_id = current_user_id()` sobre
-- una tabla que también lleva `business_id` autoriza a escribir en las filas
-- de otro tenant: la rama no está atada al comercio. Con el contexto de un
-- participante, esa rama reach de las filas del otro comercio.
--
-- La regla que queda:
--
--   * una tabla que lleva `business_id` la escribe el comercio dueño de la
--     fila, o la plataforma (el contexto de sistema);
--   * las excepciones están anotadas una por una y ninguna es una rama de
--     `user_id` sin atar al comercio.
--
-- Las tres excepciones que quedan, y por qué:
--
--   * `tickets` UPDATE: el participante cancela su propio ticket
--     (`FR-PA-07`). El trigger `tickets_cancellation_trg` deja pasar sólo la
--     transición a `cancelled` y nada más.
--   * `notifications` UPDATE: el participante marca su notificación como
--     leída. El trigger `notifications_content_immutable_trg` deja pasar sólo
--     `read_at`.
--   * las tablas que no llevan `business_id` (participantes, tokens, device
--     tokens, números de ticket): su dueño es el participante, y la fila se
--     ata a él con la función que la crea.

-- --------------------------------------------------------------------------
-- businesses: el alta es una función (`register_business`).
-- --------------------------------------------------------------------------
DROP POLICY IF EXISTS businesses_insert ON businesses;
CREATE POLICY businesses_insert ON businesses
    FOR INSERT
    WITH CHECK (id = current_business_id() OR is_system_context());

DROP POLICY IF EXISTS businesses_update ON businesses;
CREATE POLICY businesses_update ON businesses
    FOR UPDATE
    USING (id = current_business_id() OR is_system_context())
    WITH CHECK (id = current_business_id() OR is_system_context());

DROP POLICY IF EXISTS businesses_delete ON businesses;
CREATE POLICY businesses_delete ON businesses
    FOR DELETE
    USING (id = current_business_id() OR is_system_context());

-- --------------------------------------------------------------------------
-- business_members: la pertenencia la escribe el dueño del comercio.
--
-- La rama `user_id = current_user_id()` permitía que un participante se
-- nombrara `owner` de cualquier comercio: `business_members` es la única
-- fuente de pertenencia, y cualquier autorización que la lea le entregaba
-- el tenant. La invitación (el flujo `TSK-F1-01`) la hace la plataforma o
-- el dueño del comercio, no el invitado.
-- --------------------------------------------------------------------------
DROP POLICY IF EXISTS business_members_insert ON business_members;
CREATE POLICY business_members_insert ON business_members
    FOR INSERT
    WITH CHECK (business_id = current_business_id() OR is_system_context());

DROP POLICY IF EXISTS business_members_update ON business_members;
CREATE POLICY business_members_update ON business_members
    FOR UPDATE
    USING (business_id = current_business_id() OR is_system_context())
    WITH CHECK (business_id = current_business_id() OR is_system_context());

DROP POLICY IF EXISTS business_members_delete ON business_members;
CREATE POLICY business_members_delete ON business_members
    FOR DELETE
    USING (business_id = current_business_id() OR is_system_context());

-- --------------------------------------------------------------------------
-- tickets: la reserva es una función (`reserve_ticket`).
--
-- La rama `user_id = current_user_id()` permitía insertar un ticket
-- fantasma en el sorteo de otro comercio: el `numbers_count` no tenía
-- números detrás y el contador del sorteo se divergía de la verdad.
-- --------------------------------------------------------------------------
DROP POLICY IF EXISTS tickets_insert ON tickets;
CREATE POLICY tickets_insert ON tickets
    FOR INSERT
    WITH CHECK (business_id = current_business_id() OR is_system_context());

-- --------------------------------------------------------------------------
-- prizes: el premio lo crea el comercio (o la plataforma).
--
-- La rama `user_id = current_user_id()` permitía insertar un premio con el
-- `claim_code` elegido por el participante, y ocupar el lugar del premio
-- real en `prizes_raffle_position_idx`.
-- --------------------------------------------------------------------------
DROP POLICY IF EXISTS prizes_insert ON prizes;
CREATE POLICY prizes_insert ON prizes
    FOR INSERT
    WITH CHECK (business_id = current_business_id() OR is_system_context());

DROP POLICY IF EXISTS prizes_update ON prizes;
CREATE POLICY prizes_update ON prizes
    FOR UPDATE
    USING (business_id = current_business_id() OR is_system_context())
    WITH CHECK (business_id = current_business_id() OR is_system_context());

DROP POLICY IF EXISTS prizes_delete ON prizes;
CREATE POLICY prizes_delete ON prizes
    FOR DELETE
    USING (business_id = current_business_id() OR is_system_context());

-- --------------------------------------------------------------------------
-- consent_records: el consentimiento lo da el participante (función
-- `record_consent`) y es append-only.
-- --------------------------------------------------------------------------
DROP POLICY IF EXISTS consent_records_insert ON consent_records;
CREATE POLICY consent_records_insert ON consent_records
    FOR INSERT
    WITH CHECK (business_id = current_business_id() OR is_system_context());

DROP POLICY IF EXISTS consent_records_update ON consent_records;
CREATE POLICY consent_records_update ON consent_records
    FOR UPDATE
    USING (is_system_context())
    WITH CHECK (is_system_context());

DROP POLICY IF EXISTS consent_records_delete ON consent_records;
CREATE POLICY consent_records_delete ON consent_records
    FOR DELETE
    USING (is_system_context());

-- --------------------------------------------------------------------------
-- notifications: las crea la plataforma o el comercio.
--
-- La rama `user_id = current_user_id()` permitía fabricar una notificación
-- "ganaste un millón" para el propio participante: la aplicación la
-- renderiza como un mensaje de la plataforma.
-- --------------------------------------------------------------------------
DROP POLICY IF EXISTS notifications_insert ON notifications;
CREATE POLICY notifications_insert ON notifications
    FOR INSERT
    WITH CHECK (is_system_context() OR business_id = current_business_id());

-- El participante marca su notificación como leída: el trigger deja pasar
-- sólo `read_at`.
CREATE OR REPLACE FUNCTION enforce_notification_content_immutable() RETURNS trigger AS $$
BEGIN
    -- El contexto de un participante es el que no tiene comercio.
    IF current_user_id() IS NOT NULL AND current_business_id() IS NULL THEN
        IF (to_jsonb(NEW) - 'read_at' - 'updated_at')
           IS DISTINCT FROM
           (to_jsonb(OLD) - 'read_at' - 'updated_at') THEN
            RAISE EXCEPTION 'a participant can only mark a notification as read';
        END IF;
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER notifications_content_immutable_trg
    BEFORE UPDATE ON notifications
    FOR EACH ROW EXECUTE FUNCTION enforce_notification_content_immutable();

-- --------------------------------------------------------------------------
-- ticket_numbers: los números los reserva la función (`reserve_ticket`).
--
-- El `EXISTS` sobre `tickets` está filtrado por el RLS de `tickets`: el
-- participante veía su propio ticket y podía insertar números sueltos,
-- sin la reserva, sin el estado del sorteo y sin el límite por usuario.
-- --------------------------------------------------------------------------
DROP POLICY IF EXISTS ticket_numbers_insert ON ticket_numbers;
CREATE POLICY ticket_numbers_insert ON ticket_numbers
    FOR INSERT
    WITH CHECK (is_system_context());

DROP POLICY IF EXISTS ticket_numbers_update ON ticket_numbers;
CREATE POLICY ticket_numbers_update ON ticket_numbers
    FOR UPDATE
    USING (is_system_context())
    WITH CHECK (is_system_context());

DROP POLICY IF EXISTS ticket_numbers_delete ON ticket_numbers;
CREATE POLICY ticket_numbers_delete ON ticket_numbers
    FOR DELETE
    USING (is_system_context());

-- --------------------------------------------------------------------------
-- auth_tokens: el enlace mágico se emite con la función
-- (`insert_magic_link`).
--
-- La rama `kind = 'magic_link'` dejaba emitir un enlace para cualquier
-- correo: el correo es un dato personal y el enlace es una credencial.
-- --------------------------------------------------------------------------
DROP POLICY IF EXISTS auth_tokens_insert ON auth_tokens;
CREATE POLICY auth_tokens_insert ON auth_tokens
    FOR INSERT
    WITH CHECK (is_system_context() OR (kind = 'refresh' AND user_id = current_user_id()));

-- --------------------------------------------------------------------------
-- audit_log: la bitácora sin comercio es de la plataforma.
-- --------------------------------------------------------------------------
DROP POLICY IF EXISTS audit_log_insert ON audit_log;
CREATE POLICY audit_log_insert ON audit_log
    FOR INSERT
    WITH CHECK (is_system_context() OR business_id = current_business_id());

-- --------------------------------------------------------------------------
-- raffles y draws: la rama pública de lectura no autoriza escrituras.
--
-- El `USING` público (un sorteo `published`, un draw revelado) se queda en
-- las políticas de `SELECT`. En las de `UPDATE` no hace falta: la reserva
-- bloquea la fila desde la función, que corre como el dueño de la tabla.
-- --------------------------------------------------------------------------
DROP POLICY IF EXISTS raffles_update ON raffles;
CREATE POLICY raffles_update ON raffles
    FOR UPDATE
    USING (business_id = current_business_id() OR is_system_context())
    WITH CHECK (business_id = current_business_id() OR is_system_context());

DROP POLICY IF EXISTS draws_update ON draws;
CREATE POLICY draws_update ON draws
    FOR UPDATE
    USING (business_id = current_business_id() OR is_system_context())
    WITH CHECK (business_id = current_business_id() OR is_system_context());
