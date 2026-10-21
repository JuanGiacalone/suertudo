-- Los rechazos de la base tienen un SQLSTATE que el worker sabe traducir.
--
-- Un `RAISE EXCEPTION` sin `ERRCODE` levanta el `P0001` por omisión, y el
-- worker de identidad no lo conoce: el rechazo viaja como `500
-- INTERNAL_ERROR` en lugar de `403 RBAC_FORBIDDEN`. Un rechazo que el
-- cliente no puede manejar es un rechazo roto.
--
-- Los SQLSTATEs que el dominio mapea (`docs/data-model.md` §3.25):
--
--   PT001  TICKET_LIMIT_REACHED      el participante llegó al límite
--   PT002  RAFFLE_NOT_OPEN            el sorteo todavía no abrió
--   PT003  RAFFLE_CLOSED              el sorteo ya no acepta reservas
--   PT004  NUMBER_TAKEN               el número ya está reservado
--   PT005  TICKET_SIZE_OUT_OF_RANGE   la cantidad de números no es válida
--   42501  RBAC_FORBIDDEN             el que llama no es el dueño de la fila
--   55000  AUTH_REQUIRED              la familia de refresh está revocada
--   23503  NOT_FOUND                  la fila referenciada no existe
--   23505  CONFLICT                   violación de unicidad
--
-- Los tres rechazos que quedaban sin `ERRCODE` son de la familia `42501`:
-- el que llama no es el dueño de la fila que escribe.
--
-- Esta migración no cambia el comportamiento de las garantías: sólo el
-- código con el que la base las dice.

-- --------------------------------------------------------------------------
-- El ticket tiene que ser del mismo comercio que su sorteo.
-- --------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION enforce_ticket_business_match() RETURNS trigger AS $$
DECLARE
    raffle_business_id uuid;
BEGIN
    SELECT r.business_id INTO raffle_business_id
      FROM raffles r
     WHERE r.id = NEW.raffle_id;

    IF raffle_business_id IS NULL OR raffle_business_id <> NEW.business_id THEN
        RAISE EXCEPTION 'ticket.business_id must match raffle.business_id'
            USING ERRCODE = '42501';
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp;

COMMENT ON FUNCTION enforce_ticket_business_match() IS
    'SECURITY DEFINER: sin esto el SELECT del sorteo pasa por el RLS del que llama y el trigger es inocuo para un rol que no es el dueño de las tablas. El rechazo es `42501`: el que escribe no es el dueño del sorteo.';

-- --------------------------------------------------------------------------
-- El sorteo, el comercio y el dueño de un ticket no se mueven.
-- --------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION enforce_ticket_ownership_immutable() RETURNS trigger AS $$
BEGIN
    IF NEW.raffle_id IS DISTINCT FROM OLD.raffle_id
       OR NEW.business_id IS DISTINCT FROM OLD.business_id THEN
        RAISE EXCEPTION 'ticket.raffle_id and ticket.business_id are immutable'
            USING ERRCODE = '42501';
    END IF;

    IF NEW.user_id IS DISTINCT FROM OLD.user_id AND NEW.user_id IS NOT NULL THEN
        RAISE EXCEPTION 'ticket.user_id can only be anonymised'
            USING ERRCODE = '42501';
    END IF;

    -- La anonimización no puede cambiar nada más de la fila.
    IF OLD.user_id IS NOT NULL AND NEW.user_id IS NULL
       AND (to_jsonb(NEW) - 'user_id' - 'updated_at')
           IS DISTINCT FROM (to_jsonb(OLD) - 'user_id' - 'updated_at') THEN
        RAISE EXCEPTION 'the anonymisation of a ticket cannot change the ticket'
            USING ERRCODE = '42501';
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

COMMENT ON FUNCTION enforce_ticket_ownership_immutable() IS
    'El sorteo, el comercio y el dueño de un ticket no se mueven. Los rechazos son `42501`: el que escribe no es el dueño de la fila.';

-- --------------------------------------------------------------------------
-- El participante marca su notificación como leída, y nada más.
-- --------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION enforce_notification_content_immutable() RETURNS trigger AS $$
BEGIN
    -- El contexto de un participante es el que no tiene comercio.
    IF current_user_id() IS NOT NULL AND current_business_id() IS NULL THEN
        IF (to_jsonb(NEW) - 'read_at' - 'updated_at')
           IS DISTINCT FROM
           (to_jsonb(OLD) - 'read_at' - 'updated_at') THEN
            RAISE EXCEPTION 'a participant can only mark a notification as read'
                USING ERRCODE = '42501';
        END IF;
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

COMMENT ON FUNCTION enforce_notification_content_immutable() IS
    'El participante sólo marca su notificación como leída. El rechazo es `42501`: la notificación es de otro participante.';
