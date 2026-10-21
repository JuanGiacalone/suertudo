-- La toma de control de un dispositivo, verificada por SMS.
--
-- `push_devices.token` es UNIQUE global: un token identifica **un
-- teléfono**, no a una persona. `register_push_device` (`0038`) rechaza con
-- `42501` el token que ya es de otro participante, y ese rechazo es
-- correcto: sin él, un atacante que roba una cadena de token se lleva las
-- notificaciones de la víctima (el defecto `F-3`).
--
-- El costo real del rechazo es otro: **el mismo teléfono, una segunda
-- cuenta, y la cuenta nueva pierde el push en silencio** (`TSK-F0-10`).
--
-- La salida es una verificación: el token repetido abre un flujo de
-- verificación por SMS, y la transferencia ocurre **después** de que la
-- verificación se confirme.
--
-- La decisión de seguridad que no se toca: **el SMS va al teléfono del
-- participante que hoy es el dueño del token**, no al del que lo reclama. Si
-- fuera al del reclamante, el atacante que robó la cadena del token recibiría
-- su propio código y la transferencia se completaría. Yendo al dueño real,
-- sólo quien tiene ese teléfono puede autorizar la transferencia. El dueño
-- también puede negarse: no contesta.
--
-- El código nunca llega a la base en claro: la tabla guarda el SHA-256
-- (`code_hash`), y la función hashea el código que recibe antes de
-- compararlo (`digest(p_code, 'sha256')`, de pgcrypto).

-- --------------------------------------------------------------------------
-- La tabla.
--
-- Es una tabla de plataforma, no de tenant: no tiene `business_id`. Los dos
-- participantes del flujo (el dueño actual y el reclamante) son del
-- perímetro del participante, y por eso tiene RLS igual: un participante ve
-- sus propias verificaciones y ninguna más.
-- --------------------------------------------------------------------------
CREATE TABLE device_verifications (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    device_token      text NOT NULL,
    current_owner_id  uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    claimant_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash         bytea NOT NULL,
    status            text NOT NULL DEFAULT 'pending'
                      CHECK (status IN ('pending','confirmed','expired','refused','cancelled')),
    attempts          integer NOT NULL DEFAULT 0,
    expires_at        timestamptz NOT NULL,
    confirmed_at      timestamptz,
    created_at        timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE device_verifications IS
    'Verificaciones de la toma de control de un dispositivo. El código nunca se guarda en claro: `code_hash` es su SHA-256. `current_owner_id` es el dueño actual del token (a él le llega el SMS) y `claimant_id` el participante que reclama.';

-- El flujo no deja un montón de verificaciones pendientes para el mismo
-- par (token, reclamante): la segunda petición reutiliza la primera. El
-- índice parcial es la garantía, no la consulta que lo busca.
CREATE UNIQUE INDEX device_verifications_pending_idx
    ON device_verifications (device_token, claimant_id)
    WHERE status = 'pending';

COMMENT ON INDEX device_verifications_pending_idx IS
    'Una sola verificación pendiente por par (token, reclamante). La segunda petición reutiliza la fila en vez de apilar.';

CREATE INDEX device_verifications_owner_idx
    ON device_verifications (current_owner_id);

CREATE INDEX device_verifications_expiry_idx
    ON device_verifications (expires_at)
    WHERE status = 'pending';

-- --------------------------------------------------------------------------
-- El aislamiento.
--
-- Un participante ve las verificaciones en las que es parte (el dueño
-- actual o el reclamante) y ninguna más. La plataforma ve todas.
--
-- Las escrituras son de la plataforma: las funciones de abajo son
-- `SECURITY DEFINER` y escriben como el dueño de la tabla, así que las
-- políticas de escritura no las frenan. Un participante que escriba la
-- tabla a mano recibe un rechazo de RLS.
-- --------------------------------------------------------------------------
ALTER TABLE device_verifications ENABLE ROW LEVEL SECURITY;

CREATE POLICY device_verifications_select ON device_verifications
    FOR SELECT
    USING (claimant_id = current_user_id()
        OR current_owner_id = current_user_id()
        OR is_system_context());

CREATE POLICY device_verifications_insert ON device_verifications
    FOR INSERT
    WITH CHECK (is_system_context());

CREATE POLICY device_verifications_update ON device_verifications
    FOR UPDATE
    USING (is_system_context())
    WITH CHECK (is_system_context());

CREATE POLICY device_verifications_delete ON device_verifications
    FOR DELETE
    USING (is_system_context());

-- --------------------------------------------------------------------------
-- Pedir la toma de control.
--
-- Registra el token si está libre o si ya es del participante, y abre la
-- verificación si el token es de otro. Devuelve `NULL` cuando el token
-- termina siendo del participante, y el id de la verificación pendiente
-- cuando no.
--
-- `p_code_hash` es el SHA-256 del código que va en el SMS. La función no
-- genera el código: lo genera el worker, que es el que lo manda.
-- --------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION request_device_takeover(
    p_token       text,
    p_platform    text,
    p_app_version text,
    p_user_id     uuid,
    p_code_hash   bytea,
    p_expires_at  timestamptz
) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_owner_id uuid;
    v_verification_id uuid;
BEGIN
    -- La función ata al que llama: el dispositivo es del participante del
    -- contexto, y nadie más puede abrir una verificación en su nombre.
    IF NOT is_system_context() AND current_user_id() IS DISTINCT FROM p_user_id THEN
        RAISE EXCEPTION 'the device can only be registered by its owner'
            USING ERRCODE = '42501';
    END IF;

    SELECT pd.user_id INTO v_owner_id
      FROM push_devices pd
     WHERE pd.token = p_token
       FOR UPDATE;

    -- El token está libre: se registra y no hay nada que verificar.
    IF v_owner_id IS NULL THEN
        INSERT INTO push_devices (user_id, platform, token, app_version, active, last_seen_at)
        VALUES (p_user_id, p_platform, p_token, p_app_version, true, now());
        RETURN NULL;
    END IF;

    -- El token ya es del participante: se reactiva.
    IF v_owner_id = p_user_id THEN
        UPDATE push_devices
           SET platform = p_platform,
               app_version = p_app_version,
               active = true,
               last_seen_at = now()
         WHERE token = p_token;
        RETURN NULL;
    END IF;

    -- El token es de otro participante: **el dueño no se mueve**. Lo que se
    -- abre es la verificación, y la transferencia depende de que se
    -- confirme.
    SELECT dv.id INTO v_verification_id
      FROM device_verifications dv
     WHERE dv.device_token = p_token
       AND dv.claimant_id = p_user_id
       AND dv.status = 'pending'
       FOR UPDATE;

    IF v_verification_id IS NULL THEN
        INSERT INTO device_verifications
            (device_token, current_owner_id, claimant_id, code_hash, status, expires_at)
        VALUES (p_token, v_owner_id, p_user_id, p_code_hash, 'pending', p_expires_at)
        RETURNING id INTO v_verification_id;
    ELSE
        -- La segunda petición del mismo par reutiliza la verificación: se
        -- refrescan el código y el vencimiento, y se reinician los intentos.
        UPDATE device_verifications
           SET code_hash = p_code_hash,
               expires_at = p_expires_at,
               attempts = 0
         WHERE id = v_verification_id;
    END IF;

    RETURN v_verification_id;
END;
$$;

COMMENT ON FUNCTION request_device_takeover(text, text, text, uuid, bytea, timestamptz) IS
    'Registra el device token del participante, o abre la verificación de la toma de control cuando el token es de otro. Devuelve NULL si el token termina siendo del participante, y el id de la verificación pendiente si no. La verificación se manda al teléfono del dueño actual del token, nunca al del reclamante.';

-- --------------------------------------------------------------------------
-- Confirmar la toma de control.
--
-- Un código incorrecto devuelve `false` y **no** levanta una excepción: el
-- contador de intentos tiene que sobrevivir al intento fallido, y una
-- excepción aborta la transacción y deshace el contador. El worker traduce
-- ese `false` al rechazo `42501` de su catálogo.
--
-- La verificación que no es del reclamante, o que no existe, y la que ya
-- venció, sí levantan: son estados que el cliente tiene que distinguir.
-- --------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION confirm_device_takeover(
    p_verification_id uuid,
    p_user_id        uuid,
    p_code           bytea
) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_verification device_verifications%ROWTYPE;
BEGIN
    IF NOT is_system_context() AND current_user_id() IS DISTINCT FROM p_user_id THEN
        RAISE EXCEPTION 'the device can only be registered by its owner'
            USING ERRCODE = '42501';
    END IF;

    SELECT * INTO v_verification
      FROM device_verifications
     WHERE id = p_verification_id
       FOR UPDATE;

    IF NOT FOUND THEN
        RETURN false;
    END IF;

    -- La verificación es del reclamante. El dueño actual no la confirma: no
    -- es suya, y confirmarla sería robar el teléfono del otro.
    IF v_verification.claimant_id IS DISTINCT FROM p_user_id THEN
        RAISE EXCEPTION 'the verification belongs to another participant'
            USING ERRCODE = '42501';
    END IF;

    -- Una verificación confirmada, cancelada o rehusada no se vuelve a
    -- confirmar.
    IF v_verification.status <> 'pending' THEN
        RETURN false;
    END IF;

    IF v_verification.expires_at <= now() THEN
        RAISE EXCEPTION 'the verification of the device expired'
            USING ERRCODE = 'PT006';
    END IF;

    -- Todo intento cuenta, el bueno y el malo.
    UPDATE device_verifications
       SET attempts = attempts + 1
     WHERE id = p_verification_id;

    -- El código no se compara en claro: se hashea acá y se compara con el
    -- hash de la fila.
    IF v_verification.code_hash <> digest(p_code, 'sha256') THEN
        RETURN false;
    END IF;

    -- El token se mueve al reclamante, y sólo si el dueño que figura en la
    -- verificación es todavía el dueño real: si el dueño anterior soltó el
    -- dispositivo en el logout, el token está libre y el reclamante lo
    -- registra solo.
    UPDATE push_devices
       SET user_id = p_user_id,
           active = true,
           last_seen_at = now()
     WHERE token = v_verification.device_token
       AND user_id = v_verification.current_owner_id;

    IF NOT FOUND THEN
        RETURN false;
    END IF;

    UPDATE device_verifications
       SET status = 'confirmed',
           confirmed_at = now()
     WHERE id = p_verification_id;

    RETURN true;
END;
$$;

COMMENT ON FUNCTION confirm_device_takeover(uuid, uuid, bytea) IS
    'Confirma la verificación con el código que recibió el participante. Devuelve true cuando el token cambió de dueño, false cuando el código es incorrecto (y el intento queda contado). Levanta `42501` si la verificación no es del reclamante y `PT006` si venció.';

-- --------------------------------------------------------------------------
-- El teléfono del dueño actual, para el SMS.
--
-- El SMS de la verificación va al teléfono del participante que **hoy** es
-- el dueño del token, no al del que lo reclama: si fuera al del reclamante,
-- el atacante que robó la cadena del token recibiría su propio código y la
-- transferencia se completaría. Yendo al dueño real, sólo quien tiene ese
-- teléfono puede autorizar la transferencia.
--
-- El teléfono no se puede leer de otra manera: `users.phone_e164` está
-- detrás de las políticas de `users`, y un participante sólo ve su propia
-- fila. La función es el camino, y entrega **sólo** el teléfono del dueño
-- actual, **sólo** al reclamante de esa verificación.
--
-- `p_user_id` no elige el teléfono: el teléfono es el de
-- `current_owner_id`, el de la fila de la verificación. El parámetro sólo
-- ata al que llama.
--
-- Devuelve NULL cuando el dueño actual no tiene teléfono en la ficha: una
-- toma de control cuyo dueño no se puede alcanzar por SMS no se puede
-- verificar, y eso lo tiene que decidir el worker, no la base.
-- --------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION device_takeover_phone(
    p_verification_id uuid,
    p_user_id        uuid
) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_claimant_id uuid;
    v_current_owner_id uuid;
    v_phone text;
BEGIN
    -- El que llama es la plataforma o el participante de la petición.
    IF NOT is_system_context() AND current_user_id() IS DISTINCT FROM p_user_id THEN
        RAISE EXCEPTION 'the phone of a device can only be asked by its claimant'
            USING ERRCODE = '42501';
    END IF;

    SELECT dv.claimant_id, dv.current_owner_id
      INTO v_claimant_id, v_current_owner_id
      FROM device_verifications dv
     WHERE dv.id = p_verification_id;

    -- Sin verificación no hay reclamante al que atar la llamada: el
    -- rechazo es el mismo, y así nadie puede enumerar verificaciones.
    IF NOT FOUND THEN
        RAISE EXCEPTION 'the phone of a device can only be asked by its claimant'
            USING ERRCODE = '42501';
    END IF;

    -- Y el que llama es el reclamante de esa verificación. El dueño actual
    -- no lo es: pedir el teléfono de otro es pedir el teléfono de otro.
    IF NOT is_system_context() AND v_claimant_id IS DISTINCT FROM current_user_id() THEN
        RAISE EXCEPTION 'the phone of a device can only be asked by its claimant'
            USING ERRCODE = '42501';
    END IF;

    -- El teléfono es el del dueño actual del token: el que tiene que
    -- recibir el SMS.
    SELECT u.phone_e164 INTO v_phone
      FROM users u
     WHERE u.id = v_current_owner_id;

    RETURN v_phone;
END;
$$;

COMMENT ON FUNCTION device_takeover_phone(uuid, uuid) IS
    'El teléfono (`phone_e164`) del participante que hoy es el dueño del token, para mandar el SMS de la verificación. SECURITY DEFINER porque `users.phone_e164` está detrás de las políticas de `users` y el reclamante sólo ve su propia fila. Sólo el reclamante de la verificación (o la plataforma) lo obtiene; el resto recibe `42501`. Devuelve NULL si el dueño actual no tiene teléfono en la ficha.';

-- --------------------------------------------------------------------------
-- Soltar los dispositivos.
--
-- El logout llama a esto: un dispositivo pertenece a una sesión, no a una
-- persona. La cuenta nueva del mismo teléfono registra el token sin
-- conflicto, que es el problema que `TSK-F0-10` describe.
--
-- Borra (no desactiva): una fila desactivada sigue siendo dueña del token,
-- y el `UNIQUE` global no la libera. La fila del token es una credencial,
-- y el logout la entrega.
-- --------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION release_user_devices(p_user_id uuid) RETURNS integer
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_count integer;
BEGIN
    IF NOT is_system_context() AND current_user_id() IS DISTINCT FROM p_user_id THEN
        RAISE EXCEPTION 'the devices of another participant cannot be released'
            USING ERRCODE = '42501';
    END IF;

    DELETE FROM push_devices WHERE user_id = p_user_id;

    GET DIAGNOSTICS v_count = ROW_COUNT;
    RETURN v_count;
END;
$$;

COMMENT ON FUNCTION release_user_devices(uuid) IS
    'Suelta los dispositivos de un participante (el logout) y devuelve cuántos soltó. Borra las filas: una fila desactivada seguiría siendo la dueña del token y el `UNIQUE` global no lo liberaría.';

-- --------------------------------------------------------------------------
-- Expirar las verificaciones que nadie confirmó.
--
-- El estado `expired` existe en el `CHECK` y alguien tiene que ponerlo:
-- sin esto, una verificación vencida queda `pending` para siempre. Es una
-- función de plataforma: no ata al que llama porque escribe filas de la
-- plataforma, y por eso sólo la plataforma la llama.
-- --------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION expire_device_verifications() RETURNS integer
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_count integer;
BEGIN
    IF NOT is_system_context() THEN
        RAISE EXCEPTION 'the verifications of the devices can only be expired by the platform'
            USING ERRCODE = '42501';
    END IF;

    UPDATE device_verifications
       SET status = 'expired'
     WHERE status = 'pending'
       AND expires_at <= now();

    GET DIAGNOSTICS v_count = ROW_COUNT;
    RETURN v_count;
END;
$$;

COMMENT ON FUNCTION expire_device_verifications() IS
    'Marca como `expired` las verificaciones pendientes que vencieron, y devuelve cuántas. La expiration la decide el reloj, no el cliente: la confirmación de una vencida levanta `PT006`.';
