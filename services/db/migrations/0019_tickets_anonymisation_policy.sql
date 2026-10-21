-- La anonimización del ticket, en una sola política de `UPDATE`.
--
-- Postgres combina con OR los `USING` de las políticas del mismo comando,
-- pero la fila nueva tiene que satisfacer el `WITH CHECK` de *todas* las
-- políticas de `UPDATE` que aplican: son condiciones encadenadas, no
-- alternativas. Con dos políticas de `UPDATE` (la del comercio y la de la
-- anonimización) la anonimación no pasaba: el `WITH CHECK` del comercio
-- exigía el contexto del comercio, y el participante que borra su cuenta no
-- lo tiene.
--
-- Con una sola política de `UPDATE` las dos cosas caben. El `WITH CHECK`
-- permite escribir:
--
--   * una fila del propio comercio (el comercio actualiza sus tickets: el
--     sorteo marca al ganador);
--   * una fila del contexto de sistema (los jobs de la plataforma);
--   * una fila que quedó sin dueño (`user_id IS NULL`): la anonimización.
--
-- El `INSERT` sigue siendo estricto: insertar un ticket del comercio de
-- otro con el `user_id` nulo sigue siendo un INSERT de otro tenant, y por
-- eso la política de `INSERT` no cambia.
--
-- Lo que el participante no puede hacer con esta política es cambiar el
-- estado de su propio ticket: para que la fila nueva pase el `WITH CHECK`
-- tiene que quedar sin dueño, y `tickets_ownership_immutable_trg` impide
-- que la anonimización cambie algo más de la fila.

DROP POLICY IF EXISTS tickets_update ON tickets;
DROP POLICY IF EXISTS tickets_update_anonymisation ON tickets;

CREATE POLICY tickets_update ON tickets
    FOR UPDATE
    USING (
        business_id = current_business_id()
        OR user_id = current_user_id()
        OR is_system_context()
    )
    WITH CHECK (
        business_id = current_business_id()
        OR is_system_context()
        OR user_id IS NULL
    );
