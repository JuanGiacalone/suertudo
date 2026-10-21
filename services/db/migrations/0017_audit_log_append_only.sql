-- `audit_log`: append-only sin romper la integridad referencial.
--
-- Las reglas `audit_log_no_update` y `audit_log_no_delete` convertían el
-- `UPDATE` y el `DELETE` en un no-op silencioso. El problema es que la
-- acción referencial `ON DELETE SET NULL` de las foreign keys es un
-- `UPDATE` sobre `audit_log`: la regla lo reescribía, la fila no se
-- anulaba y el borrado del comercio fallaba con
--
--   ERROR: referential integrity query on "businesses" from constraint
--   "audit_log_business_id_fkey" on "audit_log" gave unexpected result
--
-- El `DELETE` sigue bloqueado por regla: la bitácora no se borra. El
-- `UPDATE` lo reemplaza un trigger que distingue quién escribe:
--
--   * `pg_trigger_depth() = 1` es una escritura del usuario: se ignora
--     (mismo efecto que la regla: cero filas afectadas);
--   * `pg_trigger_depth() > 1` es la acción referencial de la foreign key:
--     se deja pasar.

DROP RULE IF EXISTS audit_log_no_update ON audit_log;

CREATE OR REPLACE FUNCTION audit_log_append_only() RETURNS trigger AS $$
BEGIN
    IF pg_trigger_depth() > 1 THEN
        -- La acción referencial `ON DELETE SET NULL`: la fila deja de
        -- apuntar al comercio o al participante que se van.
        RETURN NEW;
    END IF;

    -- Una escritura del usuario: la fila no se toca.
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER audit_log_append_only_trg
    BEFORE UPDATE ON audit_log
    FOR EACH ROW EXECUTE FUNCTION audit_log_append_only();

COMMENT ON FUNCTION audit_log_append_only() IS
    'Append-only. La escritura del usuario se ignora (cero filas afectadas); la acción referencial ON DELETE SET NULL pasa, porque si no el borrado del comercio o del participante es imposible.';
