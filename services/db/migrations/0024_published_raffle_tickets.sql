-- Un ticket sólo existe en un sorteo publicado.
--
-- El sorteo es `draft` hasta que el comercio lo publica: un sorteo en
-- borrador no es público y no acepta reservas. La aplicación lo valida
-- (`RepositoryError::RaffleNotOpen`), pero una garantía que depende del
-- handler es una sugerencia: la base la sostiene con un trigger.
--
-- El trigger tiene que ver el sorteo sin el filtro del RLS del que llama:
-- un sorteo en borrador es invisible para un contexto de participant, así
-- que la comprobación tiene que leer la fila como el dueño de la tabla.

-- El estado del sorteo, sin el RLS del que llama.
--
-- `NULL` si el sorteo no existe o fue borrado lógicamente.
CREATE OR REPLACE FUNCTION raffle_status(p_raffle_id uuid) RETURNS text
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT r.status
      FROM raffles r
     WHERE r.id = p_raffle_id
       AND r.deleted_at IS NULL;
$$;

COMMENT ON FUNCTION raffle_status(uuid) IS
    'El estado del sorteo, leído sin el RLS del que llama. SECURITY DEFINER porque un sorteo en borrador es invisible para un contexto de participant.';

CREATE OR REPLACE FUNCTION enforce_ticket_published_raffle() RETURNS trigger AS $$
DECLARE
    v_status text;
BEGIN
    v_status := raffle_status(NEW.raffle_id);

    IF v_status IS DISTINCT FROM 'published' THEN
        IF v_status IS NULL THEN
            RAISE EXCEPTION 'the raffle of the ticket does not exist';
        ELSIF v_status = 'draft' THEN
            RAISE EXCEPTION 'RAFFLE_NOT_OPEN: a ticket can only be reserved in a published raffle';
        ELSE
            RAISE EXCEPTION 'RAFFLE_CLOSED: a ticket can only be reserved in a published raffle';
        END IF;
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER tickets_published_raffle_trg
    BEFORE INSERT ON tickets
    FOR EACH ROW EXECUTE FUNCTION enforce_ticket_published_raffle();

COMMENT ON FUNCTION enforce_ticket_published_raffle() IS
    'Un ticket sólo existe en un sorteo `published` (FR-PA-01). El mensaje lleva el código del contrato de API: RAFFLE_NOT_OPEN para el borrador, RAFFLE_CLOSED para el resto.';
