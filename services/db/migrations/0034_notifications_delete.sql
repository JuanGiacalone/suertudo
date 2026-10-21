-- La notificación no la borra el participante.
--
-- La fila es el registro de lo que la plataforma le notificó al
-- participante: el participante la marca como leída (el `read_at`, y nada
-- más) y la plataforma es la que la borra. La rama `user_id =
-- current_user_id()` del borrado dejaba que el participante borrara el
-- registro de lo que se le notificó, en el sorteo de otro comercio.

DROP POLICY IF EXISTS notifications_delete ON notifications;

CREATE POLICY notifications_delete ON notifications
    FOR DELETE
    USING (is_system_context() OR business_id = current_business_id());
