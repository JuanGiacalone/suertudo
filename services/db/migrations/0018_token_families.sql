-- La muerte durable de una familia de refresh.
--
-- Revocar la familia revocaba sus tokens, pero una rotación concurrente
-- podía insertar su sucesor después de la revocación: el token válido
-- sobrevivía. La familia necesita una muerte que no dependa del orden de
-- las escrituras.
--
-- `token_families` es esa muerte: una fila por familia, con `revoked_at`.
-- El sucesor de una rotación sólo entra en una familia que no está revocada,
-- y el trigger que lo vigila bloquea la fila de la familia para serializar
-- la inserción contra la revocación.

CREATE TABLE token_families (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id        uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at     timestamptz NOT NULL DEFAULT now(),
    revoked_at     timestamptz,
    revoke_reason  text
);

COMMENT ON TABLE token_families IS
    'Familias de refresh. `revoked_at` es la muerte de la familia: mientras sea NULL, la familia puede rotar.';

CREATE INDEX token_families_user_idx ON token_families (user_id);

-- El token pertenece a su familia.
--
-- Las filas que ya existían pueden apuntar a una familia que nunca se
-- creó (la familia nació con esta migración): la constraint se agrega sin
-- validar y se valida sólo si no hay filas huérfanas.
ALTER TABLE auth_tokens
    ADD CONSTRAINT auth_tokens_family_id_fkey
    FOREIGN KEY (family_id) REFERENCES token_families (id) ON DELETE CASCADE
    NOT VALID;

DO $$
BEGIN
    ALTER TABLE auth_tokens VALIDATE CONSTRAINT auth_tokens_family_id_fkey;
EXCEPTION
    WHEN foreign_key_violation THEN
        RAISE NOTICE 'auth_tokens has rows with a family that does not exist: the constraint stays NOT VALID';
END;
$$;

-- El sucesor de una rotación sólo entra en una familia viva.
--
-- El error es distinguible del "el participante no existe": SQLSTATE 55000
-- (`object_not_in_prerequisite_state`) contra el error de la aplicación.
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
$$ LANGUAGE plpgsql;

CREATE TRIGGER auth_tokens_family_alive_trg
    BEFORE INSERT ON auth_tokens
    FOR EACH ROW EXECUTE FUNCTION enforce_refresh_family_alive();

COMMENT ON FUNCTION enforce_refresh_family_alive() IS
    'Un sucesor de rotación no puede entrar en una familia revocada. El bloqueo de la fila de la familia serializa la inserción contra la revocación.';

-- Revocar la familia revoca sus tokens.
--
-- La familia es la que muere; los tokens se van con ella. Así el estado de
-- la familia es la fuente de la verdad y no una lista de filas revocadas.
CREATE OR REPLACE FUNCTION revoke_family_tokens() RETURNS trigger AS $$
BEGIN
    IF NEW.revoked_at IS NOT NULL AND OLD.revoked_at IS NULL THEN
        UPDATE auth_tokens
           SET revoked_at = now()
         WHERE family_id = NEW.id
           AND revoked_at IS NULL;
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER token_families_revoke_tokens_trg
    AFTER UPDATE ON token_families
    FOR EACH ROW EXECUTE FUNCTION revoke_family_tokens();
