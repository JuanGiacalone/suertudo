-- La anonimización del ticket, con una función y no con una política.
--
-- Postgres no sólo exige que la fila nueva satisfaga el `WITH CHECK` de la
-- política de `UPDATE`: también exige que la fila nueva sea *visible* para
-- quien escribe, o sea, que satisfaga las políticas de `SELECT`. Un ticket
-- anonimizado (`user_id = NULL`) es invisible para el participante que lo
-- anonimiza: no hay ningún `user_id` que lo vuelva suyo. Por eso ninguna
-- política alcanza para que el participante se quite el ticket de encima:
-- la fila nueva deja de ser visible en el mismo momento en que la
-- escritura la deja sin dueño.
--
-- La garantía va en la base, y la base la ejecuta como el dueño de las
-- tablas: una función `SECURITY DEFINER` que sólo anonimiza los tickets del
-- participante que llama (o de cualquiera, si el contexto es de sistema).
--
-- El `WITH CHECK` de la política de `UPDATE` sigue admitiendo la fila sin
-- dueño (`user_id IS NULL`): el comercio y el sistema pueden anonimizar un
-- ticket de su propio comercio.

CREATE OR REPLACE FUNCTION anonymise_user_tickets(p_user_id uuid) RETURNS integer
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_count integer;
BEGIN
    -- Un participante sólo puede anonimizar sus propios tickets. El
    -- contexto de sistema (los jobs de la plataforma) puede anonimizar los
    -- de cualquiera.
    IF NOT is_system_context() AND current_user_id() IS DISTINCT FROM p_user_id THEN
        RAISE EXCEPTION 'the tickets of another participant cannot be anonymised'
            USING ERRCODE = '42501';
    END IF;

    UPDATE tickets
       SET user_id = NULL
     WHERE user_id = p_user_id;

    GET DIAGNOSTICS v_count = ROW_COUNT;
    RETURN v_count;
END;
$$;

COMMENT ON FUNCTION anonymise_user_tickets(uuid) IS
    'Anonimiza los tickets de un participante (FR-ID-05). SECURITY DEFINER porque la fila anonimizada deja de ser visible para el participante y ninguna política de UPDATE puede autorizar la escritura.';

-- El contexto de sistema ve los tickets de todos los comercios: es el
-- contexto de los jobs de la plataforma (el sorteo marca al ganador, la
-- baja anonimiza).
DROP POLICY IF EXISTS tickets_select ON tickets;

CREATE POLICY tickets_select ON tickets
    FOR SELECT
    USING (
        business_id = current_business_id()
        OR user_id = current_user_id()
        OR is_system_context()
    );

-- El `UPDATE` de un ticket es del comercio (el sorteo marca al ganador) o
-- del sistema (los jobs de la plataforma). El participante no escribe
-- tickets: su baja pasa por `anonymise_user_tickets`.
DROP POLICY IF EXISTS tickets_update ON tickets;

CREATE POLICY tickets_update ON tickets
    FOR UPDATE
    USING (business_id = current_business_id() OR is_system_context())
    WITH CHECK (business_id = current_business_id() OR is_system_context() OR user_id IS NULL);
