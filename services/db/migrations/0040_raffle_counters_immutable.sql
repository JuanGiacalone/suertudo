-- Los contadores del sorteo los recalcula la base, no el comercio.
--
-- `raffles.numbers_reserved` y `raffles.tickets_count` son desnormalizados:
-- la verdad son los números reservados y los tickets vivos. La función
-- `apply_reservation_counters` los recalcula desde la verdad, pero la
-- política de `UPDATE` de `raffles` deja escribir la fila al comercio
-- dueño, y un `UPDATE raffles SET numbers_reserved = 999` del propio
-- comercio se guardaba: el feed mentía y la reserva daba un `PT004
-- NUMBER_TAKEN` de un número que nadie había reservado.
--
-- El trigger rechaza la escritura de los dos contadores a cualquiera que
-- no sea la base: el contexto de sistema (los jobs de la plataforma) o las
-- funciones `SECURITY DEFINER` del esquema, que corren como el dueño de la
-- tabla (`apply_reservation_counters` y
-- `release_cancelled_ticket_numbers`).

CREATE OR REPLACE FUNCTION enforce_raffle_counters_immutable() RETURNS trigger AS $$
DECLARE
    v_owner text;
BEGIN
    IF NEW.numbers_reserved IS DISTINCT FROM OLD.numbers_reserved
       OR NEW.tickets_count IS DISTINCT FROM OLD.tickets_count THEN

        -- Las funciones `SECURITY DEFINER` del esquema corren como el dueño
        -- de la tabla: ése es el camino del recálculo.
        SELECT pg_get_userbyid(c.relowner) INTO v_owner
          FROM pg_class c
         WHERE c.oid = 'raffles'::regclass;

        IF current_user::text = v_owner THEN
            RETURN NEW;
        END IF;

        RAISE EXCEPTION 'the counters of a raffle are recomputed by the database'
            USING ERRCODE = '42501';
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER raffles_counters_immutable_trg
    BEFORE UPDATE ON raffles
    FOR EACH ROW EXECUTE FUNCTION enforce_raffle_counters_immutable();

COMMENT ON FUNCTION enforce_raffle_counters_immutable() IS
    'Los contadores desnormalizados del sorteo los recalcula la base desde la verdad. Un rechazo `42501` cuando alguien que no es la base los escribe.';
