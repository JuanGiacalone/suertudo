-- El trigger de los contadores reconoce al dueño del esquema y al contexto
-- de sistema.
--
-- El comentario de `0040` promete las dos cosas y el código sólo hacía la
-- primera: un job de plataforma que recalculara los contadores in situ
-- recibía `42501`. La verdad de los contadores la calcula la base, pero
-- **quién** la calcula puede ser la plataforma.
--
-- Un tenant no puede escribirlos: sigue siendo la garantía de `0040`.

CREATE OR REPLACE FUNCTION enforce_raffle_counters_immutable() RETURNS trigger AS $$
DECLARE
    v_owner text;
BEGIN
    IF NEW.numbers_reserved IS DISTINCT FROM OLD.numbers_reserved
       OR NEW.tickets_count IS DISTINCT FROM OLD.tickets_count THEN

        -- El contexto de sistema: un job de la plataforma.
        IF is_system_context() THEN
            RETURN NEW;
        END IF;

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

COMMENT ON FUNCTION enforce_raffle_counters_immutable() IS
    'Los contadores desnormalizados del sorteo los recalcula la base desde la verdad. Los escribe el contexto de sistema o las funciones SECURITY DEFINER del esquema; un tenant que los escriba recibe 42501.';
