-- La reserva del participante y los números que reserva.
--
-- Las dos invariantes duras del producto viven acá:
--   * UNIQUE (raffle_id, number) en ticket_numbers: un número se reserva una
--     sola vez por sorteo (FR-PA-03).
--   * el índice parcial único tickets_user_raffle_limit_idx: un participante
--     tiene como máximo un ticket por sorteo (FR-PA-04), incluso con dos
--     requests concurrentes.

CREATE TABLE tickets (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    raffle_id         uuid NOT NULL REFERENCES raffles(id) ON DELETE CASCADE,
    user_id           uuid REFERENCES users(id) ON DELETE SET NULL,  -- NULL si el usuario se dio de baja
    business_id       uuid NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    code              text NOT NULL UNIQUE,        -- SU-TKU4F9K
    numbers_count     integer NOT NULL CHECK (numbers_count > 0),
    status            text NOT NULL DEFAULT 'reserved'
                      CHECK (status IN ('reserved','winner','not_winner','cancelled')),
    validation_code_id uuid,  -- FK agregada en 0009 (validation_codes es posterior)
    auto_picked       boolean NOT NULL DEFAULT false,
    source            text NOT NULL DEFAULT 'app'
                      CHECK (source IN ('app','web','admin')),
    ip_hash           text,                        -- hash, no la IP cruda
    user_agent        text,
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now(),
    cancelled_at      timestamptz
);

COMMENT ON TABLE tickets IS
    'Reservas. `business_id` está desnormalizado por performance y DEBE coincidir con raffles.business_id: lo garantiza el trigger tickets_business_match_trg.';
COMMENT ON COLUMN tickets.ip_hash IS
    'Hash con salt rotativo, nunca la IP en claro: sirve para rate limiting sin retener un dato personal identificable.';

CREATE INDEX tickets_user_idx ON tickets (user_id, created_at DESC) WHERE status <> 'cancelled';
CREATE INDEX tickets_raffle_idx ON tickets (raffle_id, status);
CREATE INDEX tickets_business_idx ON tickets (business_id, created_at DESC);
CREATE UNIQUE INDEX tickets_user_raffle_limit_idx
    ON tickets (raffle_id, user_id) WHERE status <> 'cancelled';

CREATE TABLE ticket_numbers (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    ticket_id   uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    raffle_id   uuid NOT NULL REFERENCES raffles(id) ON DELETE CASCADE,
    number      integer NOT NULL CHECK (number >= 0),
    created_at  timestamptz NOT NULL DEFAULT now(),
    UNIQUE (raffle_id, number)
);

COMMENT ON TABLE ticket_numbers IS
    'Los números reservados. La constraint UNIQUE (raffle_id, number) es la pieza central de la integridad del producto.';

CREATE INDEX ticket_numbers_ticket_idx ON ticket_numbers (ticket_id);
CREATE INDEX ticket_numbers_raffle_number_idx ON ticket_numbers (raffle_id, number);

-- La invariante dura: el ticket pertenece al mismo comercio que su sorteo.
-- Un trigger, y no un chequeo en la aplicación, porque la garantía tiene que
-- valer también para las escrituras que no pasan por el handler.
CREATE FUNCTION enforce_ticket_business_match() RETURNS trigger AS $$
BEGIN
    IF (SELECT business_id FROM raffles WHERE id = NEW.raffle_id) <> NEW.business_id THEN
        RAISE EXCEPTION 'ticket.business_id must match raffle.business_id';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER tickets_business_match_trg
    BEFORE INSERT OR UPDATE ON tickets
    FOR EACH ROW EXECUTE FUNCTION enforce_ticket_business_match();
