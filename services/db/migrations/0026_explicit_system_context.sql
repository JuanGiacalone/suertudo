-- El contexto de sistema necesita un marcador propio.
--
-- El marcador anterior era la cadena vacía, y eso no distinguía dos
-- estados que tienen que ser distintos:
--
--   - "no hay contexto"   → las políticas fallan cerradas
--   - "contexto de sistema" → las políticas de plataforma aplican
--
-- Un `SET LOCAL` dentro de una transacción deja un **placeholder vacío** a
-- nivel de sesión: la conexión que ya corrió una transacción reportaba
-- `is_system_context() = true` para siempre después. Medido: 1258 filas de
-- `users` visibles sin contexto en una conexión reusada. El aislamiento
-- se podía esquivar por accidente, sin querer.
--
-- El marcador explícito 'system' separa los dos estados: un placeholder
-- vacío es "no hay contexto" y falla cerrado.
--
-- `current_business_id()` y `current_user_id()` tienen que ignorar el
-- marcador: el contexto de sistema no es un tenant.

CREATE OR REPLACE FUNCTION current_business_id() RETURNS uuid
    LANGUAGE sql STABLE AS $$
    SELECT NULLIF(NULLIF(current_setting('app.business_id', true), ''), 'system')::uuid
$$;

CREATE OR REPLACE FUNCTION current_user_id() RETURNS uuid
    LANGUAGE sql STABLE AS $$
    SELECT NULLIF(NULLIF(current_setting('app.user_id', true), ''), 'system')::uuid
$$;

-- `true` sólo cuando los dos GUC llevan el marcador explícito. Un GUC sin
-- definir (NULL) y un placeholder vacío son los dos "no hay contexto".
CREATE OR REPLACE FUNCTION is_system_context() RETURNS boolean
    LANGUAGE sql STABLE AS $$
    SELECT COALESCE(current_setting('app.business_id', true), '') = 'system'
       AND COALESCE(current_setting('app.user_id', true), '') = 'system'
$$;
