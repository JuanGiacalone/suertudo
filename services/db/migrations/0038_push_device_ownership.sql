-- El token de un device no se roba: pertenece a su participante.
--
-- `register_push_device` inserta con `ON CONFLICT (token) DO UPDATE SET
-- user_id = EXCLUDED.user_id`. El token es único en la tabla, así que el
-- conflicto es con la fila de otro participante, y la actualización le
-- entregaba el device: el atacante se llevaba el token de la víctima y la
-- plataforma le mandaba las notificaciones a su teléfono.
--
-- La función es `SECURITY DEFINER`, así que las políticas de `push_devices`
-- no pueden evitarlo: la fila se escribe como el dueño de la tabla. La
-- garantía va en la función.
--
-- El conflicto sólo actualiza la fila cuando la fila es del mismo
-- participante. Cualquier otro caso es un rechazo `42501`: el token es de
-- otro.

CREATE OR REPLACE FUNCTION register_push_device(
    p_user_id uuid,
    p_platform text,
    p_token text,
    p_app_version text
) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_id uuid;
BEGIN
    IF NOT is_system_context() AND current_user_id() IS DISTINCT FROM p_user_id THEN
        RAISE EXCEPTION 'the device can only be registered by its owner'
            USING ERRCODE = '42501';
    END IF;

    INSERT INTO push_devices (user_id, platform, token, app_version)
    VALUES (p_user_id, p_platform, p_token, p_app_version)
    ON CONFLICT (token) DO UPDATE
       SET user_id = EXCLUDED.user_id,
           platform = EXCLUDED.platform,
           app_version = EXCLUDED.app_version,
           active = true,
           last_seen_at = now()
     -- El conflicto sólo se resuelve cuando la fila es del mismo
     -- participante. Con otro dueño, la cláusula no deja pasar la fila y
     -- el `INSERT` no devuelve nada.
     WHERE push_devices.user_id = EXCLUDED.user_id
    RETURNING id INTO v_id;

    IF v_id IS NULL THEN
        RAISE EXCEPTION 'the device token belongs to another participant'
            USING ERRCODE = '42501';
    END IF;

    RETURN v_id;
END;
$$;

COMMENT ON FUNCTION register_push_device(uuid, text, text, text) IS
    'Registra (o reactiva) el device token de un participante. SECURITY DEFINER porque el registro del device ocurre sin contexto de tenant. El token es único: un token que ya es de otro participante es un rechazo `42501`, no un robo.';
