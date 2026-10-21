-- Los datos de identidad son del participante.
--
-- `users` y `auth_tokens` no tenían políticas: una conexión con el rol de la
-- aplicación leía todos los participantes (correo, teléfono, nombre) y todos
-- los hashes de los tokens. Un token no se recupera del hash, pero el hash
-- es una superficie de ataque offline y el correo es un dato personal.
--
-- El login es el caso difícil: el participante todavía no existe (o todavía
-- no está autenticado) cuando se pide o se consume el enlace mágico, así que
-- no hay contexto de tenant que lo habilite. Por eso la ruta de login no
-- son políticas sino funciones `SECURITY DEFINER` con un cuerpo fijo: la
-- base resuelve el correo o el hash y devuelve exactamente lo que el login
-- necesita, y nada más. El resto de la vida del token (la rotación, la
-- revocación) sí tiene contexto: el del participante.
--
-- `token_families` entra en el mismo grupo: una familia es del participante
-- que la abrió, y sin políticas cualquiera podía revocarle la sesión.

ALTER TABLE users ENABLE ROW LEVEL SECURITY;
ALTER TABLE auth_tokens ENABLE ROW LEVEL SECURITY;
ALTER TABLE token_families ENABLE ROW LEVEL SECURITY;

-- --------------------------------------------------------------------------
-- La ruta del login: funciones y no acceso directo.
-- --------------------------------------------------------------------------

-- El participante que se registra todavía no tiene contexto: el alta pasa
-- por la función.
CREATE OR REPLACE FUNCTION register_user(p_email text, p_display_name text) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_id uuid;
BEGIN
    INSERT INTO users (email, display_name)
    VALUES (p_email, p_display_name)
    RETURNING id INTO v_id;

    RETURN v_id;
END;
$$;

COMMENT ON FUNCTION register_user(text, text) IS
    'El alta del participante. SECURITY DEFINER porque el registro ocurre antes de que el participante exista y no hay contexto de tenant que lo habilite.';

-- El login resuelve el correo. La función devuelve lo que el login necesita
-- y nada más: la fila de un correo, nunca una lista.
CREATE OR REPLACE FUNCTION find_user_by_email(p_email text)
RETURNS TABLE (
    id                       uuid,
    email                    citext,
    display_name             text,
    role                     text,
    status                   text,
    phone_e164               text,
    locale                   text,
    timezone                 text,
    marketing_opt_in         boolean,
    notification_preferences jsonb
)
LANGUAGE sql SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT u.id, u.email, u.display_name, u.role, u.status, u.phone_e164,
           u.locale, u.timezone, u.marketing_opt_in, u.notification_preferences
      FROM users u
     WHERE u.email = p_email
       AND u.deleted_at IS NULL;
$$;

COMMENT ON FUNCTION find_user_by_email(text) IS
    'Resuelve un correo en la fila del participante. SECURITY DEFINER: el login no tiene contexto de tenant, y la función devuelve una fila y no una lista. La contraseña no se devuelve.';

-- El enlace mágico se busca por el hash del token: quien lo pide tiene el
-- token, y el hash es lo que se guarda.
CREATE OR REPLACE FUNCTION find_token_by_hash(p_token_hash bytea)
RETURNS TABLE (
    id          uuid,
    user_id     uuid,
    email       citext,
    kind        text,
    family_id   uuid,
    expires_at  timestamptz,
    consumed_at timestamptz,
    revoked_at  timestamptz
)
LANGUAGE sql SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT t.id, t.user_id, t.email, t.kind, t.family_id,
           t.expires_at, t.consumed_at, t.revoked_at
      FROM auth_tokens t
     WHERE t.token_hash = p_token_hash;
$$;

COMMENT ON FUNCTION find_token_by_hash(bytea) IS
    'La fila del token de un hash. El hash es el secreto: quien lo tiene, tiene el token.';

-- El enlace se consume una sola vez.
CREATE OR REPLACE FUNCTION consume_magic_link(p_token_id uuid) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_count integer;
BEGIN
    UPDATE auth_tokens
       SET consumed_at = now()
     WHERE id = p_token_id
       AND kind = 'magic_link'
       AND consumed_at IS NULL
       AND revoked_at IS NULL;

    GET DIAGNOSTICS v_count = ROW_COUNT;
    RETURN v_count = 1;
END;
$$;

COMMENT ON FUNCTION consume_magic_link(uuid) IS
    'Consume el enlace mágico una sola vez. SECURITY DEFINER: el participante todavía no está autenticado cuando se consume.';

-- El enlace queda ligado al participante que lo ganó.
CREATE OR REPLACE FUNCTION bind_magic_link_user(p_token_id uuid, p_user_id uuid) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_count integer;
BEGIN
    UPDATE auth_tokens
       SET user_id = p_user_id
     WHERE id = p_token_id
       AND kind = 'magic_link'
       AND consumed_at IS NOT NULL;

    GET DIAGNOSTICS v_count = ROW_COUNT;
    RETURN v_count = 1;
END;
$$;

-- El enlace pedido antes que éste queda sin efecto.
CREATE OR REPLACE FUNCTION revoke_pending_magic_links(p_email text) RETURNS integer
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_count integer;
BEGIN
    UPDATE auth_tokens
       SET consumed_at = now(),
           revoked_at = now()
     WHERE email = p_email
       AND kind = 'magic_link'
       AND consumed_at IS NULL;

    GET DIAGNOSTICS v_count = ROW_COUNT;
    RETURN v_count;
END;
$$;

-- --------------------------------------------------------------------------
-- users
-- --------------------------------------------------------------------------

-- El participante ve su propia cuenta. El comercio ve a su equipo y a los
-- participantes de sus sorteos (el panel y la exportación de contactos). El
-- sistema ve a todos: es el contexto de los jobs de la plataforma.
CREATE POLICY users_select ON users
    FOR SELECT
    USING (
        id = current_user_id()
        OR is_system_context()
        OR EXISTS (
            SELECT 1 FROM business_members m
             WHERE m.business_id = current_business_id()
               AND m.user_id = users.id
        )
        OR EXISTS (
            SELECT 1 FROM tickets t
             WHERE t.business_id = current_business_id()
               AND t.user_id = users.id
        )
    );

-- El alta pasa por `register_user`.
CREATE POLICY users_insert ON users
    FOR INSERT
    WITH CHECK (id = current_user_id() OR is_system_context());

-- El participante actualiza su propia cuenta (el perfil y la baja). El
-- sistema actualiza las cuentas (la moderación).
CREATE POLICY users_update ON users
    FOR UPDATE
    USING (id = current_user_id() OR is_system_context())
    WITH CHECK (id = current_user_id() OR is_system_context());

-- El participante se da de baja con la anonimización (soft delete): el
-- borrado duro es de la plataforma.
CREATE POLICY users_delete ON users
    FOR DELETE
    USING (is_system_context());

-- El rol de la cuenta no lo cambia el participante: si lo cambiara, se
-- nombraría platform_admin a sí mismo.
CREATE OR REPLACE FUNCTION enforce_user_role_immutable() RETURNS trigger AS $$
BEGIN
    IF NEW.role IS DISTINCT FROM OLD.role AND NOT is_system_context() THEN
        RAISE EXCEPTION 'the role of a participant is changed by the platform only';
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER users_role_immutable_trg
    BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION enforce_user_role_immutable();

-- --------------------------------------------------------------------------
-- auth_tokens
-- --------------------------------------------------------------------------

-- El participante ve sus propios tokens; el sistema ve todos.
CREATE POLICY auth_tokens_select ON auth_tokens
    FOR SELECT
    USING (user_id = current_user_id() OR is_system_context());

-- El enlace mágico lo pide cualquiera, y a veces el participante todavía no
-- existe. El refresh lo inserta el participante, con su contexto: sin él, un
-- token de sesión sería falsificable.
CREATE POLICY auth_tokens_insert ON auth_tokens
    FOR INSERT
    WITH CHECK (
        kind = 'magic_link'
        OR (kind = 'refresh' AND user_id = current_user_id())
    );

-- La rotación y la revocación del refresh son del participante (o del
-- sistema). El consumo del enlace mágico pasa por la función.
CREATE POLICY auth_tokens_update ON auth_tokens
    FOR UPDATE
    USING (user_id = current_user_id() OR is_system_context())
    WITH CHECK (user_id = current_user_id() OR is_system_context());

CREATE POLICY auth_tokens_delete ON auth_tokens
    FOR DELETE
    USING (is_system_context());

-- --------------------------------------------------------------------------
-- token_families
-- --------------------------------------------------------------------------

CREATE POLICY token_families_select ON token_families
    FOR SELECT
    USING (user_id = current_user_id() OR is_system_context());

CREATE POLICY token_families_insert ON token_families
    FOR INSERT
    WITH CHECK (user_id = current_user_id() OR is_system_context());

CREATE POLICY token_families_update ON token_families
    FOR UPDATE
    USING (user_id = current_user_id() OR is_system_context())
    WITH CHECK (user_id = current_user_id() OR is_system_context());

CREATE POLICY token_families_delete ON token_families
    FOR DELETE
    USING (is_system_context());
