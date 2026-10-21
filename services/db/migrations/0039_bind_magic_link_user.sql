-- El enlace mágico se ata a la cuenta que lo ganó.
--
-- `bind_magic_link_user(p_token_id, p_user_id)` ligaba el enlace a la
-- cuenta que le dijera el que llamaba, sin atar al que llama y sin mirar el
-- correo: con un `id` de token filtrado, la toma de una cuenta era una
-- línea de SQL.
--
-- El enlace es la credencial, y la credencial es del correo del enlace.
-- Ahora la función exige dos cosas:
--
--   * el que llama es el participante al que se ata (o la plataforma);
--   * el correo del enlace es el correo de esa cuenta.
--
-- El enlace que no existe, o que no está consumido, no se ata: la función
-- devuelve `false`, como antes.

CREATE OR REPLACE FUNCTION bind_magic_link_user(p_token_id uuid, p_user_id uuid) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_count      integer;
    v_link_email citext;
    v_user_email citext;
BEGIN
    -- El que llama es el participante (o la plataforma). Sin esta atadura,
    -- un `id` de token filtrado es una toma de cuenta.
    IF NOT is_system_context() AND current_user_id() IS DISTINCT FROM p_user_id THEN
        RAISE EXCEPTION 'the link can only be bound to the participant of the context'
            USING ERRCODE = '42501';
    END IF;

    -- El enlace consumido, con su correo.
    SELECT t.email INTO v_link_email
      FROM auth_tokens t
     WHERE t.id = p_token_id
       AND t.kind = 'magic_link'
       AND t.consumed_at IS NOT NULL;

    IF v_link_email IS NULL THEN
        RETURN FALSE;
    END IF;

    -- El correo de la cuenta que se ata.
    SELECT u.email INTO v_user_email
      FROM users u
     WHERE u.id = p_user_id;

    IF v_user_email IS NULL OR v_link_email <> v_user_email THEN
        RAISE EXCEPTION 'the link belongs to another email'
            USING ERRCODE = '42501';
    END IF;

    UPDATE auth_tokens
       SET user_id = p_user_id
     WHERE id = p_token_id
       AND kind = 'magic_link'
       AND consumed_at IS NOT NULL;

    GET DIAGNOSTICS v_count = ROW_COUNT;
    RETURN v_count = 1;
END;
$$;

COMMENT ON FUNCTION bind_magic_link_user(uuid, uuid) IS
    'Ata el enlace mágico consumido a la cuenta del participante que lo ganó. La función ata al que llama y exige que el correo del enlace sea el de la cuenta. Los rechazos son `42501`.';
