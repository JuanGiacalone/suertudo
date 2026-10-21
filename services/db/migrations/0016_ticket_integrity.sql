-- La integridad del ticket: el sorteo, el comercio y el dueño no se mueven.
--
-- `tickets_business_match_trg` comparaba `tickets.business_id` con el
-- `business_id` del sorteo leyéndolo con un `SELECT` que el RLS filtra: con el
-- contexto de un comercio, el sorteo de otro devolvía `NULL`, y
-- `NULL <> 'A'` es `NULL`, o sea, falso. El trigger no levantaba nada y un
-- ticket de otro comercio entraba sin problema.
--
-- El trigger pasa a ser `SECURITY DEFINER`: ve la fila del sorteo sin
-- importar el contexto del que llama. Y el trigger deja de ser la única
-- garantía: la foreign key compuesta `(raffle_id, business_id)` es
-- declarativa y sobrevive a cualquier cambio del trigger.

-- La pareja (id, business_id) del sorteo es única: es lo que permite que la
-- foreign key compuesta apunte a ella.
ALTER TABLE raffles
    ADD CONSTRAINT raffles_id_business_id_key UNIQUE (id, business_id);

-- La garantía declarativa: el ticket pertenece al mismo comercio que su
-- sorteo, siempre, sin importar quién escribe.
ALTER TABLE tickets
    ADD CONSTRAINT tickets_raffle_business_fkey
    FOREIGN KEY (raffle_id, business_id)
    REFERENCES raffles (id, business_id)
    ON DELETE CASCADE;

-- El trigger deja de depender del RLS del que llama.
CREATE OR REPLACE FUNCTION enforce_ticket_business_match() RETURNS trigger AS $$
DECLARE
    raffle_business_id uuid;
BEGIN
    SELECT r.business_id INTO raffle_business_id
      FROM raffles r
     WHERE r.id = NEW.raffle_id;

    IF raffle_business_id IS NULL OR raffle_business_id <> NEW.business_id THEN
        RAISE EXCEPTION 'ticket.business_id must match raffle.business_id';
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp;

COMMENT ON FUNCTION enforce_ticket_business_match() IS
    'SECURITY DEFINER: sin esto el SELECT del sorteo pasa por el RLS del que llama y el trigger es inocuo para un rol que no es el dueño de las tablas.';

-- El sorteo, el comercio y el dueño de un ticket no se mueven.
--
-- La baja de la cuenta anonimiza el ticket (`user_id = NULL`) y nada más: un
-- participante no puede cambiar el estado de su ticket, pasarlo al sorteo de
-- otro comercio ni quedarse con el de otro.
CREATE OR REPLACE FUNCTION enforce_ticket_ownership_immutable() RETURNS trigger AS $$
BEGIN
    IF NEW.raffle_id IS DISTINCT FROM OLD.raffle_id
       OR NEW.business_id IS DISTINCT FROM OLD.business_id THEN
        RAISE EXCEPTION 'ticket.raffle_id and ticket.business_id are immutable';
    END IF;

    IF NEW.user_id IS DISTINCT FROM OLD.user_id AND NEW.user_id IS NOT NULL THEN
        RAISE EXCEPTION 'ticket.user_id can only be anonymised';
    END IF;

    -- La anonimización no puede cambiar nada más de la fila.
    IF OLD.user_id IS NOT NULL AND NEW.user_id IS NULL
       AND (to_jsonb(NEW) - 'user_id' - 'updated_at')
           IS DISTINCT FROM (to_jsonb(OLD) - 'user_id' - 'updated_at') THEN
        RAISE EXCEPTION 'the anonymisation of a ticket cannot change the ticket';
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER tickets_ownership_immutable_trg
    BEFORE UPDATE ON tickets
    FOR EACH ROW EXECUTE FUNCTION enforce_ticket_ownership_immutable();

-- Los contadores desnormalizados del sorteo los mantiene la base.
--
-- La reserva la hace un participante, que no es el dueño del sorteo: con el
-- RLS de escritura (cada tenancy escribe sus propias filas) esa actualización
-- sería rechazada. La función es `SECURITY DEFINER` y su cuerpo es fijo:
-- sólo mueve los dos contadores, y los toma de la fila del ticket.
CREATE OR REPLACE FUNCTION apply_reservation_counters(p_ticket_id uuid) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_raffle_id uuid;
    v_numbers   integer;
BEGIN
    SELECT t.raffle_id, t.numbers_count INTO v_raffle_id, v_numbers
      FROM tickets t
     WHERE t.id = p_ticket_id
       AND t.status = 'reserved';

    IF v_raffle_id IS NULL THEN
        RAISE EXCEPTION 'the ticket % does not exist', p_ticket_id;
    END IF;

    UPDATE raffles
       SET numbers_reserved = numbers_reserved + v_numbers,
           tickets_count = tickets_count + 1,
           updated_at = now()
     WHERE id = v_raffle_id
       AND status = 'published'
       AND deleted_at IS NULL;

    RETURN v_raffle_id;
END;
$$;

COMMENT ON FUNCTION apply_reservation_counters(uuid) IS
    'Mantiene los contadores desnormalizados del sorteo. SECURITY DEFINER porque la reserva la hace un participante y el RLS de escritura sólo deja escribir al dueño del sorteo.';
