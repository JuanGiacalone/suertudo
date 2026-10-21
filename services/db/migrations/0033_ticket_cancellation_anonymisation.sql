-- La rama pública de lectura vuelve a las políticas de `UPDATE`.
--
-- La migración `0027` las sacó: el `USING` público (un sorteo `published`, un
-- draw revelado) deja ver la fila al UPDATE de otro tenant. La escritura
-- sigue rechazada por el `WITH CHECK` (que es el que ata la fila al
-- comercio), pero el rechazo es un error explícito y no un cero filas
-- affected: una fila que el UPDATE no puede tocar tiene que decir por qué.
--
-- La rama es de lectura, no de escritura: el `WITH CHECK` no la tiene.

DROP POLICY IF EXISTS raffles_update ON raffles;
CREATE POLICY raffles_update ON raffles
    FOR UPDATE
    USING (
        business_id = current_business_id()
        OR (status = 'published' AND deleted_at IS NULL)
        OR is_system_context()
    )
    WITH CHECK (business_id = current_business_id() OR is_system_context());

DROP POLICY IF EXISTS draws_update ON draws;
CREATE POLICY draws_update ON draws
    FOR UPDATE
    USING (
        business_id = current_business_id()
        OR seed_revealed = true
        OR is_system_context()
    )
    WITH CHECK (business_id = current_business_id() OR is_system_context());

-- --------------------------------------------------------------------------
-- La anonimización también es una escritura del participante.
--
-- El trigger de la cancelación (`0031`) rechaza toda escritura del
-- participante que no sea la transición a `cancelled`, y con eso rechazaba
-- también la anonimización: la baja de la cuenta pone `user_id = NULL` en el
-- ticket, y el estado no cambia.
--
-- La anonimización es una de las dos escrituras que el participante tiene
-- sobre su propio ticket: la otra es la cancelación. Las dos están
-- acotadas: la anonimización deja el ticket sin dueño y nada más cambia; la
-- cancelación cambia el estado y nada más.

CREATE OR REPLACE FUNCTION enforce_ticket_cancellation() RETURNS trigger AS $$
DECLARE
    v_status text;
BEGIN
    -- El contexto de un participante es el que no tiene comercio: la
    -- cancelación del comercio y la de la plataforma no dependen del estado
    -- del sorteo (un sorteo cerrado se puede anular).
    IF current_user_id() IS NOT NULL AND current_business_id() IS NULL THEN
        -- (a) La anonimización: el ticket queda sin dueño y nada más cambia.
        IF OLD.user_id IS NOT NULL AND NEW.user_id IS NULL THEN
            IF (to_jsonb(NEW) - 'user_id' - 'updated_at')
               IS DISTINCT FROM
               (to_jsonb(OLD) - 'user_id' - 'updated_at') THEN
                RAISE EXCEPTION 'the anonymisation of a ticket cannot change the ticket'
                    USING ERRCODE = '42501';
            END IF;

            RETURN NEW;
        END IF;

        -- (b) Un ticket cancelado es un registro: no se toca más.
        IF OLD.status = 'cancelled' THEN
            IF (to_jsonb(NEW) - 'updated_at')
               IS DISTINCT FROM (to_jsonb(OLD) - 'updated_at') THEN
                RAISE EXCEPTION 'a cancelled ticket cannot be changed by the participant'
                    USING ERRCODE = '42501';
            END IF;

            RETURN NEW;
        END IF;

        -- (c) La cancelación: sólo mientras el sorteo esté `published`, y
        --     sólo cambia el estado del ticket.
        IF NEW.status = 'cancelled' THEN
            v_status := raffle_status(NEW.raffle_id);
            IF v_status IS DISTINCT FROM 'published' THEN
                RAISE EXCEPTION 'RAFFLE_CLOSED: the participant can only cancel a ticket while the raffle is published'
                    USING ERRCODE = 'PT003';
            END IF;

            IF (to_jsonb(NEW) - 'status' - 'cancelled_at' - 'updated_at')
               IS DISTINCT FROM
               (to_jsonb(OLD) - 'status' - 'cancelled_at' - 'updated_at') THEN
                RAISE EXCEPTION 'a cancellation can only change the status of the ticket'
                    USING ERRCODE = '42501';
            END IF;
        ELSE
            RAISE EXCEPTION 'the participant can only cancel their own ticket'
                USING ERRCODE = '42501';
        END IF;
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
