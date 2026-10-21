-- La anonimización de los tickets no devuelve el conteo.
--
-- El contrato que el worker de identidad está programando contra declara
-- `anonymise_user_tickets(p_user_id uuid) RETURNS void`. La función
-- devolvía el número de tickets anonimizados, y Postgres no deja cambiar el
-- tipo de retorno de una función que ya existe: la función se cae y se
-- vuelve a crear.
--
-- El número de tickets anonimizados se lee con un `count`, que es lo que
-- importa de todas formas.

DROP FUNCTION IF EXISTS anonymise_user_tickets(uuid);

CREATE OR REPLACE FUNCTION anonymise_user_tickets(p_user_id uuid) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
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
END;
$$;

COMMENT ON FUNCTION anonymise_user_tickets(uuid) IS
    'Anonimiza los tickets de un participante (FR-ID-05). SECURITY DEFINER porque la fila anonimizada deja de ser visible para el participante y ninguna política de UPDATE puede autorizar la escritura.';
