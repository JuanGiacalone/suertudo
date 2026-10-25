-- TSK-F0-07: el registro durable de la última emisión de un magic link.
--
-- El login no tiene contexto de tenant: el participante puede no existir
-- todavía, y la dirección es la credencial. Por eso la función es
-- SECURITY DEFINER, como las otras del login (`find_user_by_email`,
-- `register_user`).
--
-- La función devuelve la emisión ANTERIOR y marca la nueva, en una sola
-- operación. Dos peticiones simultáneas no pueden pasar las dos el
-- enfriamiento: la segunda espera el cerrojo de la fila y, al despertar,
-- ve la marca de la primera.
--
-- El `::citext` **no es un detalle**. Un parámetro `text` comparado con
-- una columna `citext` resuelve al operador de `text`, y la comparación
-- queda **sensible a mayúsculas**: la dirección se encontraría o no según
-- cómo la escribiera el que llama. Con el cast explícito, la comparación
-- es la del tipo de la columna.
--
-- El cerrojo se toma en la función de la izquierda y el `UPDATE` se apoya
-- en ella. Las dos cosas tienen que ser la misma fila: por eso el `WHERE`
-- repite la condición del correo.
CREATE OR REPLACE FUNCTION touch_last_magic_link(p_email text, p_at timestamptz)
RETURNS timestamptz
LANGUAGE sql SECURITY DEFINER SET search_path = public, pg_temp AS $$
  WITH previous AS (
    SELECT u.last_magic_link_at AS previous
    FROM users u
    WHERE u.email = p_email::citext AND u.deleted_at IS NULL
    FOR UPDATE
  ), marked AS (
    UPDATE users u
    SET last_magic_link_at = p_at, updated_at = now()
    FROM previous
    WHERE u.email = p_email::citext AND u.deleted_at IS NULL
    RETURNING u.id
  )
  SELECT previous.previous FROM previous;
$$;

COMMENT ON FUNCTION touch_last_magic_link(text, timestamptz) IS
  'Devuelve la última emisión de un magic link para la dirección y marca la nueva. SECURITY DEFINER: el login no tiene contexto de tenant. Es una sola operación, para que dos peticiones simultáneas no puedan pasar las dos el enfriamiento.';
