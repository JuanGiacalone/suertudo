-- Las columnas de la cuenta que son de la plataforma.
--
-- `users_update` es `id = current_user_id()`: el participante escribe su
-- propia fila. El trigger congelaba el `role`, y el resto de las columnas de
-- la cuenta quedaban a disposición del participante:
--
--   * `status`: una cuenta en `pending_review` (`FR-PA-10`) se limpiaba a
--     sí misma con un `UPDATE users SET status = 'active'`;
--   * `deleted_at`: una cuenta borrada se resucitaba;
--   * `email`: la dirección de la cuenta se cambiaba sin verificación;
--   * `password_hash`: la contraseña se escribía a mano.
--
-- El trigger congela las cuatro. Quién puede escribirlas:
--
--   * la plataforma, con el contexto de sistema (los jobs de moderación);
--   * las funciones `SECURITY DEFINER` del esquema (el camino de la baja y
--     el del cambio de contraseña): se reconocen porque corren como el
--     dueño de la tabla, que no es el rol de la aplicación;
--   * el participante, en un solo caso: la baja de su propia cuenta
--     (`FR-ID-05`), que anonimiza la fila y no deja escapar nada más.
--
-- La baja se reconoce por sus tres columnas a la vez: el estado pasa a
-- `deleted`, la fecha de baja aparece y el correo pasa a ser el marcador
-- de anonimización de esa misma cuenta. Cualquier otra escritura de estas
-- columnas por un participante es un rechazo `42501`.

DROP TRIGGER IF EXISTS users_role_immutable_trg ON users;

DROP FUNCTION IF EXISTS enforce_user_role_immutable();

CREATE OR REPLACE FUNCTION enforce_user_platform_columns_immutable() RETURNS trigger AS $$
DECLARE
    v_owner text;
BEGIN
    -- La plataforma: los jobs de moderación y de la baja.
    IF is_system_context() THEN
        RETURN NEW;
    END IF;

    -- Las funciones `SECURITY DEFINER` del esquema corren como el dueño de
    -- la tabla. Es el camino de la baja y el del cambio de contraseña.
    SELECT pg_get_userbyid(c.relowner) INTO v_owner
      FROM pg_class c
     WHERE c.oid = 'users'::regclass;

    IF current_user::text = v_owner THEN
        RETURN NEW;
    END IF;

    -- El rol lo cambia la plataforma, siempre.
    IF NEW.role IS DISTINCT FROM OLD.role THEN
        RAISE EXCEPTION 'the role of a participant is changed by the platform only'
            USING ERRCODE = '42501';
    END IF;

    -- La contraseña la cambia la plataforma (o la función del esquema que la
    -- cambia). El participante no escribe su contraseña.
    IF NEW.password_hash IS DISTINCT FROM OLD.password_hash THEN
        RAISE EXCEPTION 'the password of a participant is changed by the platform only'
            USING ERRCODE = '42501';
    END IF;

    -- La baja de la cuenta (`FR-ID-05`): el participante anonimiza su
    -- propia fila y nada más.
    IF NEW.status = 'deleted'
       AND OLD.status <> 'deleted'
       AND OLD.deleted_at IS NULL
       AND NEW.deleted_at IS NOT NULL
       AND NEW.email = 'deleted+' || OLD.id::text || '@suertudo.invalid' THEN
        RETURN NEW;
    END IF;

    -- La moderación es de la plataforma: `pending_review` y `suspended` no
    -- los limpia la cuenta.
    IF NEW.status IS DISTINCT FROM OLD.status THEN
        RAISE EXCEPTION 'the status of a participant is changed by the platform only'
            USING ERRCODE = '42501';
    END IF;

    -- Una cuenta borrada no vuelve.
    IF NEW.deleted_at IS DISTINCT FROM OLD.deleted_at THEN
        RAISE EXCEPTION 'a deleted participant cannot be resurrected'
            USING ERRCODE = '42501';
    END IF;

    -- El correo es la identidad de la cuenta: su cambio necesita un camino
    -- verificado, y ese camino no es el participante.
    IF NEW.email IS DISTINCT FROM OLD.email THEN
        RAISE EXCEPTION 'the email of a participant is changed by the platform only'
            USING ERRCODE = '42501';
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER users_platform_columns_immutable_trg
    BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION enforce_user_platform_columns_immutable();

COMMENT ON FUNCTION enforce_user_platform_columns_immutable() IS
    'El participante no escribe el rol, el estado, la fecha de baja, el correo ni la contraseña de su cuenta. La excepción es la baja de la cuenta, que anonimiza la fila. Los rechazos son `42501`.';
