-- Notificaciones a los usuarios.

CREATE TABLE notifications (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    business_id   uuid REFERENCES businesses(id) ON DELETE CASCADE,  -- NULL = notificación de plataforma
    raffle_id     uuid REFERENCES raffles(id) ON DELETE CASCADE,
    type          text NOT NULL,
                  -- 'you_won','prize_claimed','prize_delivered','raffle_closing_soon',
                  -- 'raffle_voided','ticket_confirmed','business_message','marketing'
    channel       text NOT NULL CHECK (channel IN ('push','email','in_app','whatsapp')),
    payload       jsonb NOT NULL,
    dedupe_key    text NOT NULL UNIQUE,     -- sha1(user_id + type + entity_id)
    status        text NOT NULL DEFAULT 'queued'
                  CHECK (status IN ('queued','sending','sent','failed','skipped')),
    scheduled_at  timestamptz NOT NULL DEFAULT now(),
    sent_at       timestamptz,
    read_at       timestamptz,
    error         text,
    created_at    timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE notifications IS
    'Notificaciones. `dedupe_key` UNIQUE implementa FR-NO-03 (idempotencia) en la base: el worker puede reintentar sin miedo.';
COMMENT ON COLUMN notifications.status IS
    'skipped no es un error: un push a un usuario sin token queda skipped y el email sigue su curso.';

CREATE INDEX notifications_user_idx ON notifications (user_id, created_at DESC);
CREATE INDEX notifications_pending_idx ON notifications (status) WHERE status = 'queued';
CREATE INDEX notifications_unscheduled_idx ON notifications (scheduled_at) WHERE status = 'queued';
