-- Los contadores del sorteo se recalculan, no se incrementan.
--
-- `apply_reservation_counters` sumaba: cada llamada sumaba los contadores, no
-- estaba atada al que llamaba y podía mover los contadores del sorteo de
-- otro comercio (`numbers_reserved` de 0 a 9 sin un solo número reservado).
-- Es una escritura cruzada sin límite, que es lo mismo que la rama de
-- `user_id` que se acaba de sacar de las políticas.
--
-- La función recalcula los contadores desde la verdad: los números
-- reservados y los tickets vivos del sorteo. Un contador recalculado no se
-- deriva y es idempotente por construcción: llamarla dos veces deja lo
-- mismo.
--
-- Y está atada a quien llama: los contadores del sorteo los recalcula el
-- dueño del ticket (el participante que reservó) o la plataforma.
--
-- El tipo de retorno cambia (de `uuid` a `void`): Postgres no deja cambiar
-- el tipo de una función que ya existe, así que la función se cae y se
-- vuelve a crear.

DROP FUNCTION IF EXISTS apply_reservation_counters(uuid);

CREATE OR REPLACE FUNCTION apply_reservation_counters(p_ticket_id uuid) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_raffle_id uuid;
    v_user_id   uuid;
BEGIN
    SELECT t.raffle_id, t.user_id INTO v_raffle_id, v_user_id
      FROM tickets t
     WHERE t.id = p_ticket_id;

    IF v_raffle_id IS NULL THEN
        RAISE EXCEPTION 'the ticket % does not exist', p_ticket_id
            USING ERRCODE = '23503';
    END IF;

    -- El que llama es el dueño del ticket (o la plataforma).
    IF NOT is_system_context() AND current_user_id() IS DISTINCT FROM v_user_id THEN
        RAISE EXCEPTION 'the counters of a raffle are recomputed by the platform or by the owner of the ticket'
            USING ERRCODE = '42501';
    END IF;

    UPDATE raffles r
       SET numbers_reserved = (
               SELECT count(*) FROM ticket_numbers tn WHERE tn.raffle_id = r.id
           ),
           tickets_count = (
               SELECT count(*) FROM tickets t
                WHERE t.raffle_id = r.id
                  AND t.status <> 'cancelled'
           ),
           updated_at = now()
     WHERE r.id = v_raffle_id;
END;
$$;

COMMENT ON FUNCTION apply_reservation_counters(uuid) IS
    'Recalcula los contadores desnormalizados de un sorteo desde la verdad. SECURITY DEFINER y atada al dueño del ticket: sin eso era una escritura cruzada sin límite.';

-- La liberación de los números recalcula los contadores también, para que
-- el contador y el pool no puedan divergir. El recalculo va dentro del
-- trigger (que corre como el dueño de la tabla) y no dentro de la función
-- de arriba: la función está atada al dueño del ticket, y quien cancela
-- puede ser el comercio.
CREATE OR REPLACE FUNCTION release_cancelled_ticket_numbers() RETURNS trigger AS $$
BEGIN
    IF NEW.status = 'cancelled' AND OLD.status IS DISTINCT FROM 'cancelled' THEN
        DELETE FROM ticket_numbers WHERE ticket_id = NEW.id;

        UPDATE raffles r
           SET numbers_reserved = (
                   SELECT count(*) FROM ticket_numbers tn WHERE tn.raffle_id = r.id
               ),
               tickets_count = (
                   SELECT count(*) FROM tickets t
                    WHERE t.raffle_id = r.id
                      AND t.status <> 'cancelled'
               ),
               updated_at = now()
         WHERE r.id = NEW.raffle_id;
    END IF;

    RETURN NULL;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp;

COMMENT ON FUNCTION release_cancelled_ticket_numbers() IS
    'La cancelación libera los números del ticket y recalcula los contadores del sorteo desde la verdad.';
