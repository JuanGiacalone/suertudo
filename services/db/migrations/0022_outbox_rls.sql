-- El outbox es un dato de tenant.
--
-- `outbox` es el patrón del transactional outbox (`ADR-002`): el evento se
-- escribe en la misma transacción que confirma el sorteo, y el payload
-- lleva datos del comercio y del participante. Sin políticas, una conexión
-- con el rol de la aplicación leía el outbox entero: el historial de lo que
-- la plataforma le\notificó a cada comercio.
--
-- La columna `business_id` no existía: el evento se escribía sin saber a qué
-- comercio pertenecía. Los eventos de plataforma (sin comercio) siguen
-- existiendo y sólo el contexto de sistema los ve.

ALTER TABLE outbox
    ADD COLUMN business_id uuid REFERENCES businesses(id) ON DELETE CASCADE;

COMMENT ON COLUMN outbox.business_id IS
    'El comercio al que pertenece el evento. NULL en los eventos de plataforma.';

CREATE INDEX outbox_business_idx ON outbox (business_id, created_at DESC);

ALTER TABLE outbox ENABLE ROW LEVEL SECURITY;

-- Una fila es visible para el comercio al que pertenece. Una fila sin
-- comercio (un evento de plataforma) sólo es visible para el contexto de
-- sistema.
CREATE POLICY outbox_select ON outbox
    FOR SELECT
    USING (is_system_context() OR business_id = current_business_id());

-- El outbox lo escribe el sistema (el worker que confirma el sorteo) o el
-- propio comercio.
CREATE POLICY outbox_insert ON outbox
    FOR INSERT
    WITH CHECK (is_system_context() OR business_id = current_business_id());

-- El publicador marca el evento como publicado: es el sistema.
CREATE POLICY outbox_update ON outbox
    FOR UPDATE
    USING (is_system_context() OR business_id = current_business_id())
    WITH CHECK (is_system_context() OR business_id = current_business_id());

CREATE POLICY outbox_delete ON outbox
    FOR DELETE
    USING (is_system_context() OR business_id = current_business_id());
