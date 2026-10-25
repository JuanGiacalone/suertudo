-- TSK-F0-07: la comparación del correo, sin depender de quién llama.
--
-- Una columna `citext` comparada con un **parámetro `text`** resuelve al
-- operador de `text`, y la comparación queda **sensible a mayúsculas**: la
-- dirección se encuentra o no según cómo la escribiera el que llama.
--
-- Hoy el login normaliza la dirección antes de llamar, así que el
-- comportamiento es el correcto. Pero la corrección depende de un detalle
-- del llamador: si mañana alguien pasa la dirección sin normalizar, el
-- participante no se encuentra y el enlace mágico **nunca llega** — y la
-- respuesta es `202`, así que el fallo es silencioso.
--
-- El `::citext` explícito deja la comparación en el tipo de la columna y
-- hace que la función sea correcta por sí sola.
--
-- Las dos funciones se redefinen con `CREATE OR REPLACE`: el cuerpo
-- cambia, la firma y el tipo de retorno no.
CREATE OR REPLACE FUNCTION register_user(p_email text, p_display_name text) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_id uuid;
BEGIN
    INSERT INTO users (email, display_name)
    VALUES (p_email::citext, p_display_name)
    RETURNING id INTO v_id;

    RETURN v_id;
END;
$$;

COMMENT ON FUNCTION register_user(text, text) IS
  'El alta del participante. SECURITY DEFINER porque el registro ocurre antes de que el participante exista y no hay contexto de tenant que lo habilite. El ::citext hace que la comparación sea la del tipo de la columna y no dependa de cómo escribió el llamador la dirección.';

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
    WHERE u.email = p_email::citext
      AND u.deleted_at IS NULL;
$$;

COMMENT ON FUNCTION find_user_by_email(text) IS
  'Resuelve un correo en la fila del participante. SECURITY DEFINER: el login no tiene contexto de tenant, y la función devuelve una fila y no una lista. La contraseña no se devuelve. El ::citext hace que la comparación sea la del tipo de la columna y no dependa de cómo escribió el llamador la dirección.';
