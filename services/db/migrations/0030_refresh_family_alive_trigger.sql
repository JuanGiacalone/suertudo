-- El trigger de la familia de refresh tiene que ver la familia.
--
-- `enforce_refresh_family_alive()` leía la fila de `token_families` con un
-- `SELECT ... FOR SHARE`, y esa lectura estaba filtrada por el RLS de
-- `token_families` (que la migración `0023` le puso): el trigger no era
-- `SECURITY DEFINER`, así que veía la familia filtrada, el bloqueo que el
-- comentario promete nunca se tomaba, y un participante podía insertar un
-- token de refresh en la familia revocada de otro participante.
--
-- Es la misma clase de defecto que el trigger del business match (B2): un
-- trigger que lee otra tabla tiene que leerla como el dueño de la tabla.

CREATE OR REPLACE FUNCTION enforce_refresh_family_alive() RETURNS trigger AS $$
DECLARE
    v_revoked_at timestamptz;
BEGIN
    IF NEW.family_id IS NULL THEN
        RETURN NEW;
    END IF;

    -- El bloqueo serializa la inserción contra la revocación: la revocación
    -- actualiza la fila de la familia y espera a esta transacción. Si la
    -- revocación se confirma primero, esta inserción falla; si esta
    -- inserción se confirma primero, la revocación la alcanza y revoca el
    -- sucesor.
    SELECT revoked_at INTO v_revoked_at
      FROM token_families
     WHERE id = NEW.family_id
     FOR SHARE;

    IF v_revoked_at IS NOT NULL THEN
        RAISE EXCEPTION 'the refresh token family is revoked'
            USING ERRCODE = '55000';
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp;

COMMENT ON FUNCTION enforce_refresh_family_alive() IS
    'Un sucesor de rotación no puede entrar en una familia revocada. SECURITY DEFINER: sin esto el SELECT de la familia pasa por el RLS del que llama y el trigger es inocuo.';
