-- El límite de tickets por participante (`FR-PA-04`).
--
-- `max_tickets_per_user` estaba en el esquema y nadie lo leía. El índice
-- parcial único fijaba un ticket por `(raffle, user)`: un sorteo con
-- `max_tickets_per_user = 2` le negaba al participante su segundo ticket.
--
-- Un índice único no se puede parametrizar, así que el límite lo cuenta un
-- trigger: los tickets vivos del participante para ese sorteo, comparados
-- contra `raffles.max_tickets_per_user`. El trigger bloquea la fila del
-- sorteo, que es lo que serializa las reservas concurrentes: el conteo es
-- exacto y no hay carrera.
--
-- El índice parcial único se cae: no puede expresar "hasta N". El piso de
-- uno lo sigue poniendo el trigger (`max_tickets_per_user` es `>= 1` por el
-- CHECK de la columna), y el índice queda como índice de consulta.
--
-- El refusal es distinguible: SQLSTATE `PT001`.

DROP INDEX IF EXISTS tickets_user_raffle_limit_idx;

CREATE INDEX tickets_user_raffle_idx
    ON tickets (raffle_id, user_id) WHERE status <> 'cancelled';

CREATE OR REPLACE FUNCTION enforce_ticket_max_per_user() RETURNS trigger AS $$
DECLARE
    v_limit integer;
    v_live  integer;
BEGIN
    -- El bloqueo del sorteo serializa las reservas del mismo sorteo: es lo
    -- que hace exacto el conteo de abajo.
    PERFORM 1 FROM raffles WHERE id = NEW.raffle_id FOR UPDATE;

    SELECT r.max_tickets_per_user INTO v_limit
      FROM raffles r
     WHERE r.id = NEW.raffle_id;

    IF v_limit IS NULL THEN
        -- El sorteo no existe: la foreign key lo va a rechazar.
        RETURN NEW;
    END IF;

    SELECT count(*) INTO v_live
      FROM tickets t
     WHERE t.raffle_id = NEW.raffle_id
       AND t.user_id = NEW.user_id
       AND t.status <> 'cancelled';

    IF v_live >= v_limit THEN
        RAISE EXCEPTION 'TICKET_LIMIT_REACHED: the participant reached the limit of % tickets for this raffle',
            v_limit
            USING ERRCODE = 'PT001';
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp;

CREATE TRIGGER tickets_max_per_user_trg
    BEFORE INSERT ON tickets
    FOR EACH ROW EXECUTE FUNCTION enforce_ticket_max_per_user();

COMMENT ON FUNCTION enforce_ticket_max_per_user() IS
    'El participante no puede tener más tickets vivos que `raffles.max_tickets_per_user` (FR-PA-04). El lock del sorteo serializa las reservas: el conteo es exacto.';

-- --------------------------------------------------------------------------
-- La cancelación del participante sólo puede ser la transición.
--
-- La rama de la política de `UPDATE` permite `status = 'cancelled'`, y la
-- política no distingue "el participant cancela" de "el participant
-- modifica un ticket ya cancelado": con el ticket cancelado, la fila pasaba
-- el `WITH CHECK` con cualquier `numbers_count`.
--
-- El trigger compara la fila nueva con la vieja: si el ticket ya estaba
-- cancelado, no se toca nada; si se cancela, sólo cambian el estado y la
-- fecha de cancelación.

CREATE OR REPLACE FUNCTION enforce_ticket_cancellation() RETURNS trigger AS $$
DECLARE
    v_status text;
BEGIN
    -- El contexto de un participante es el que no tiene comercio: la
    -- cancelación del comercio y la de la plataforma no dependen del estado
    -- del sorteo (un sorteo cerrado se puede anular).
    IF current_user_id() IS NOT NULL AND current_business_id() IS NULL THEN
        IF OLD.status = 'cancelled' THEN
            -- Un ticket cancelado es un registro: no se toca más.
            IF (to_jsonb(NEW) - 'updated_at')
               IS DISTINCT FROM (to_jsonb(OLD) - 'updated_at') THEN
                RAISE EXCEPTION 'a cancelled ticket cannot be changed by the participant'
                    USING ERRCODE = '42501';
            END IF;

            RETURN NEW;
        END IF;

        IF NEW.status = 'cancelled' THEN
            v_status := raffle_status(NEW.raffle_id);
            IF v_status IS DISTINCT FROM 'published' THEN
                RAISE EXCEPTION 'RAFFLE_CLOSED: the participant can only cancel a ticket while the raffle is published'
                    USING ERRCODE = 'PT003';
            END IF;

            -- La cancelación no cambia nada más de la fila.
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

-- El trigger del sorteo publicado usa los mismos SQLSTATE que las funciones
-- de la migración `0028`.
CREATE OR REPLACE FUNCTION enforce_ticket_published_raffle() RETURNS trigger AS $$
DECLARE
    v_status text;
BEGIN
    v_status := raffle_status(NEW.raffle_id);

    IF v_status IS DISTINCT FROM 'published' THEN
        IF v_status IS NULL THEN
            RAISE EXCEPTION 'the raffle of the ticket does not exist'
                USING ERRCODE = '23503';
        ELSIF v_status = 'draft' THEN
            RAISE EXCEPTION 'RAFFLE_NOT_OPEN: a ticket can only be reserved in a published raffle'
                USING ERRCODE = 'PT002';
        ELSE
            RAISE EXCEPTION 'RAFFLE_CLOSED: a ticket can only be reserved in a published raffle'
                USING ERRCODE = 'PT003';
        END IF;
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
