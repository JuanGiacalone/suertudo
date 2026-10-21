-- Los device tokens son del participante.
--
-- `push_devices` no tenía políticas: una conexión con el rol de la
-- aplicación leía el token de push de todos los participantes y podía
-- desactivar el de otro (`UPDATE push_devices SET active = false`).
--
-- El token del device es una credencial: la plataforma tiene que poder
-- mandar la notificación, y nadie más.

ALTER TABLE push_devices ENABLE ROW LEVEL SECURITY;

CREATE POLICY push_devices_select ON push_devices
    FOR SELECT
    USING (user_id = current_user_id() OR is_system_context());

CREATE POLICY push_devices_insert ON push_devices
    FOR INSERT
    WITH CHECK (user_id = current_user_id() OR is_system_context());

CREATE POLICY push_devices_update ON push_devices
    FOR UPDATE
    USING (user_id = current_user_id() OR is_system_context())
    WITH CHECK (user_id = current_user_id() OR is_system_context());

CREATE POLICY push_devices_delete ON push_devices
    FOR DELETE
    USING (user_id = current_user_id() OR is_system_context());

COMMENT ON TABLE push_devices IS
    'Dispositivos push. `token` es una credencial: sólo el participante dueño y la plataforma los ven.';

-- --------------------------------------------------------------------------
-- La baja de la cuenta se lleva los device tokens (`FR-ID-05`).
--
-- La anonimización de los tickets no tocaba los dispositivos: el token de
-- push y su `user_id` sobrevivían a la baja. El token es un dato personal
-- (identifica el teléfono) y una credencial (permite mandar push).
--
-- La función es aparte y no una extensión de `anonymise_user_tickets`: el
-- worker de identidad está programando contra ese nombre ahora mismo.
-- --------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION anonymise_user_devices(p_user_id uuid) RETURNS integer
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_count integer;
BEGIN
    IF NOT is_system_context() AND current_user_id() IS DISTINCT FROM p_user_id THEN
        RAISE EXCEPTION 'the devices of another participant cannot be anonymised'
            USING ERRCODE = '42501';
    END IF;

    DELETE FROM push_devices WHERE user_id = p_user_id;

    GET DIAGNOSTICS v_count = ROW_COUNT;
    RETURN v_count;
END;
$$;

COMMENT ON FUNCTION anonymise_user_devices(uuid) IS
    'Borra los device tokens de un participante (FR-ID-05). El token de push es un dato personal y una credencial: no sobrevive a la baja de la cuenta.';
