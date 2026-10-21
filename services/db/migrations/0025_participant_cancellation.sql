-- El participante cancela su ticket (`FR-PA-07`).
--
-- "El participante puede cancelar su ticket mientras el sorteo esté
-- `published`." La rama de la política de `UPDATE` le permite cambiar el
-- estado de su propio ticket y nada más: el resto de la fila no se toca.
--
-- La cancelación **libera los números**: los números del ticket vuelven al
-- pool y el contador desnormalizado del sorteo baja con ellos. La liberación
-- es un trigger y no una línea del handler, porque la garantía tiene que
-- valer para cualquier camino que cancele un ticket (el participante, el
-- comercio, la plataforma que anula un sorteo).

DROP POLICY IF EXISTS tickets_update ON tickets;

CREATE POLICY tickets_update ON tickets
    FOR UPDATE
    USING (
        business_id = current_business_id()
        OR user_id = current_user_id()
        OR is_system_context()
    )
    WITH CHECK (
        business_id = current_business_id()
        OR is_system_context()
        OR user_id IS NULL
        OR (user_id = current_user_id() AND status = 'cancelled')
    );

COMMENT ON POLICY tickets_update ON tickets IS
    'El comercio actualiza sus tickets, el sistema los de todos y el participante cancela los suyos (`FR-PA-07`): la rama del participante sólo admite un `status = ''cancelled''`.';

-- La cancelación del participante: sólo mientras el sorteo esté `published`,
-- y sólo cambia el estado del ticket.
CREATE OR REPLACE FUNCTION enforce_ticket_cancellation() RETURNS trigger AS $$
DECLARE
    v_status text;
BEGIN
    IF NEW.status = 'cancelled' AND OLD.status IS DISTINCT FROM 'cancelled' THEN
        -- El contexto de un participante es el que no tiene comercio: la
        -- cancelación del comercio y la de la plataforma no dependen del
        -- estado del sorteo (un sorteo cerrado se puede anular).
        IF current_user_id() IS NOT NULL AND current_business_id() IS NULL THEN
            v_status := raffle_status(NEW.raffle_id);
            IF v_status IS DISTINCT FROM 'published' THEN
                RAISE EXCEPTION 'RAFFLE_CLOSED: the participant can only cancel a ticket while the raffle is published';
            END IF;

            -- La cancelación no cambia nada más de la fila.
            IF (to_jsonb(NEW) - 'status' - 'cancelled_at' - 'updated_at')
               IS DISTINCT FROM
               (to_jsonb(OLD) - 'status' - 'cancelled_at' - 'updated_at') THEN
                RAISE EXCEPTION 'a cancellation can only change the status of the ticket';
            END IF;
        END IF;
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER tickets_cancellation_trg
    BEFORE UPDATE ON tickets
    FOR EACH ROW EXECUTE FUNCTION enforce_ticket_cancellation();

-- La cancelación libera los números.
--
-- Los números vuelven al pool (`UNIQUE (raffle_id, number)` deja de
-- bloquearlos) y el contador desnormalizado del sorteo baja con ellos: el
-- feed muestra "quedan N números" y el número tiene que ser el mismo en el
-- contador y en la reserva.
CREATE OR REPLACE FUNCTION release_cancelled_ticket_numbers() RETURNS trigger AS $$
DECLARE
    v_released integer;
BEGIN
    IF NEW.status = 'cancelled' AND OLD.status IS DISTINCT FROM 'cancelled' THEN
        DELETE FROM ticket_numbers WHERE ticket_id = NEW.id;
        GET DIAGNOSTICS v_released = ROW_COUNT;

        UPDATE raffles
           SET numbers_reserved = GREATEST(numbers_reserved - v_released, 0),
               tickets_count = GREATEST(tickets_count - 1, 0),
               updated_at = now()
         WHERE id = NEW.raffle_id;
    END IF;

    RETURN NULL;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp;

CREATE TRIGGER tickets_release_numbers_trg
    AFTER UPDATE ON tickets
    FOR EACH ROW EXECUTE FUNCTION release_cancelled_ticket_numbers();

COMMENT ON FUNCTION release_cancelled_ticket_numbers() IS
    'La cancelación libera los números del ticket y baja el contador del sorteo. Es un trigger para que la garantía valga para cualquier camino que cancele un ticket.';
