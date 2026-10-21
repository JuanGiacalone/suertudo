-- Las funciones que atan la fila a su dueño.
--
-- Una función `SECURITY DEFINER` que escribe una fila de un tenant y no está
-- atada al que llama es una escritura cruzada sin límite: es exactamente lo
-- que era `apply_reservation_counters`. Todas las funciones de este archivo:
--
--   * son `SECURITY DEFINER` con `search_path` fijo, para que el RLS del que
--     llama no las filtre;
--   * verifican quién llama: el participante del contexto (o la plataforma,
--     con el contexto de sistema) es el dueño de la fila que escriben.
--
-- Los SQLSTATE que usan (el worker de identidad los mapea a códigos HTTP):
--
--   PT001  TICKET_LIMIT_REACHED      el participante llegó al límite de
--                                      tickets del sorteo
--   PT002  RAFFLE_NOT_OPEN            el sorteo todavía no abrió
--   PT003  RAFFLE_CLOSED              el sorteo ya no acepta reservas
--   PT004  NUMBER_TAKEN               el número ya está reservado
--   PT005  TICKET_SIZE_OUT_OF_RANGE   la cantidad de números no es válida
--   42501  (insufficient_privilege)   el que llama no es el dueño de la fila

-- --------------------------------------------------------------------------
-- El alta del negocio con su dueño (FR-ID-02).
-- --------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION register_business(
    p_slug text,
    p_name text,
    p_category text,
    p_user_id uuid
) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_business_id uuid;
BEGIN
    -- El que llama es el dueño.
    IF NOT is_system_context() AND current_user_id() IS DISTINCT FROM p_user_id THEN
        RAISE EXCEPTION 'the business can only be registered by its owner'
            USING ERRCODE = '42501';
    END IF;

    INSERT INTO businesses (slug, name, category)
    VALUES (p_slug, p_name, p_category)
    RETURNING id INTO v_business_id;

    INSERT INTO business_members (business_id, user_id, role, status)
    VALUES (v_business_id, p_user_id, 'owner', 'active');

    RETURN v_business_id;
END;
$$;

COMMENT ON FUNCTION register_business(text, text, text, uuid) IS
    'Alta del negocio con su dueño. El alta y la pertenencia ocurren en la misma transacción: un negocio sin dueño no existe.';

-- --------------------------------------------------------------------------
-- La emisión del enlace mágico.
--
-- El solicitante todavía no está autenticado: no hay contexto de tenant que
-- habilite la escritura, y el correo es un dato personal.
-- --------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION insert_magic_link(
    p_email text,
    p_token_hash bytea,
    p_expires_at timestamptz
) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_id uuid;
BEGIN
    INSERT INTO auth_tokens (email, kind, token_hash, expires_at)
    VALUES (p_email, 'magic_link', p_token_hash, p_expires_at)
    RETURNING id INTO v_id;

    RETURN v_id;
END;
$$;

COMMENT ON FUNCTION insert_magic_link(text, bytea, timestamptz) IS
    'Emite el enlace mágico de un correo. SECURITY DEFINER porque el solicitante todavía no está autenticado.';

-- --------------------------------------------------------------------------
-- El registro del device token.
-- --------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION register_push_device(
    p_user_id uuid,
    p_platform text,
    p_token text,
    p_app_version text
) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_id uuid;
BEGIN
    IF NOT is_system_context() AND current_user_id() IS DISTINCT FROM p_user_id THEN
        RAISE EXCEPTION 'the device can only be registered by its owner'
            USING ERRCODE = '42501';
    END IF;

    INSERT INTO push_devices (user_id, platform, token, app_version)
    VALUES (p_user_id, p_platform, p_token, p_app_version)
    ON CONFLICT (token) DO UPDATE
       SET user_id = EXCLUDED.user_id,
           platform = EXCLUDED.platform,
           app_version = EXCLUDED.app_version,
           active = true,
           last_seen_at = now()
    RETURNING id INTO v_id;

    RETURN v_id;
END;
$$;

COMMENT ON FUNCTION register_push_device(uuid, text, text, text) IS
    'Registra (o reactiva) el device token de un participante. SECURITY DEFINER porque el registro del device ocurre sin contexto de tenant.';

-- --------------------------------------------------------------------------
-- El consentimiento (append-only).
-- --------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION record_consent(
    p_user_id uuid,
    p_business_id uuid,
    p_scope text,
    p_granted boolean,
    p_source text,
    p_user_agent text
) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_id uuid;
BEGIN
    -- El consentimiento lo da el participante.
    IF NOT is_system_context() AND current_user_id() IS DISTINCT FROM p_user_id THEN
        RAISE EXCEPTION 'the consent can only be recorded by the participant'
            USING ERRCODE = '42501';
    END IF;

    INSERT INTO consent_records (user_id, business_id, scope, granted, source, user_agent)
    VALUES (p_user_id, p_business_id, p_scope, p_granted, p_source, p_user_agent)
    RETURNING id INTO v_id;

    RETURN v_id;
END;
$$;

COMMENT ON FUNCTION record_consent(uuid, uuid, text, boolean, text, text) IS
    'Registra el consentimiento de un participante con un comercio. Append-only: cada cambio es una fila nueva.';

-- --------------------------------------------------------------------------
-- El premio, sobre un sorteo y un draw del mismo comercio.
-- --------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION create_prize(
    p_raffle_id uuid,
    p_draw_id uuid,
    p_business_id uuid,
    p_user_id uuid,
    p_prize_name text,
    p_winning_number int
) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_id uuid;
BEGIN
    -- El sorteo es del comercio.
    IF NOT EXISTS (
        SELECT 1 FROM raffles r
         WHERE r.id = p_raffle_id
           AND r.business_id = p_business_id
    ) THEN
        RAISE EXCEPTION 'the raffle does not belong to the business'
            USING ERRCODE = '42501';
    END IF;

    -- El draw es de ese sorteo y de ese comercio.
    IF NOT EXISTS (
        SELECT 1 FROM draws d
         WHERE d.id = p_draw_id
           AND d.raffle_id = p_raffle_id
           AND d.business_id = p_business_id
    ) THEN
        RAISE EXCEPTION 'the draw does not belong to the raffle and the business'
            USING ERRCODE = '42501';
    END IF;

    -- El premio lo crea el comercio (o la plataforma).
    IF NOT is_system_context() AND current_business_id() IS DISTINCT FROM p_business_id THEN
        RAISE EXCEPTION 'the prize can only be created by the business of the raffle'
            USING ERRCODE = '42501';
    END IF;

    INSERT INTO prizes (raffle_id, draw_id, business_id, user_id, prize_name, winning_number)
    VALUES (p_raffle_id, p_draw_id, p_business_id, p_user_id, p_prize_name, p_winning_number)
    RETURNING id INTO v_id;

    RETURN v_id;
END;
$$;

COMMENT ON FUNCTION create_prize(uuid, uuid, uuid, uuid, text, int) IS
    'Crea el premio de un sorteo ejecutado. Verifica que el sorteo, el draw y el negocio son el mismo.';

-- --------------------------------------------------------------------------
-- La reserva atómica de números.
-- --------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION reserve_ticket(
    p_raffle_id uuid,
    p_user_id uuid,
    p_code text,
    p_numbers int[],
    p_auto_picked boolean DEFAULT false,
    p_source text DEFAULT 'app'
) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_business_id uuid;
    v_status      text;
    v_pool        integer;
    v_min         integer;
    v_max         integer;
    v_ticket_id   uuid;
    v_number      integer;
    v_count       integer;
BEGIN
    -- El que llama es el participante (o la plataforma).
    IF NOT is_system_context() AND current_user_id() IS DISTINCT FROM p_user_id THEN
        RAISE EXCEPTION 'the ticket can only be reserved for the participant of the context'
            USING ERRCODE = '42501';
    END IF;

    -- El sorteo existe y está publicado. El bloqueo serializa las reservas
    -- del mismo sorteo: es lo que hace exacto el conteo del límite por
    -- participante (`tickets_max_per_user_trg`).
    SELECT r.business_id, r.status, r.number_pool_size,
           r.numbers_per_ticket_min, r.numbers_per_ticket_max
      INTO v_business_id, v_status, v_pool, v_min, v_max
      FROM raffles r
     WHERE r.id = p_raffle_id
       AND r.deleted_at IS NULL
     FOR UPDATE;

    IF v_business_id IS NULL THEN
        RAISE EXCEPTION 'the raffle does not exist'
            USING ERRCODE = '23503';
    END IF;

    IF v_status = 'draft' THEN
        RAISE EXCEPTION 'RAFFLE_NOT_OPEN: the raffle is not open for reservations'
            USING ERRCODE = 'PT002';
    END IF;

    IF v_status IS DISTINCT FROM 'published' THEN
        RAISE EXCEPTION 'RAFFLE_CLOSED: the raffle no longer accepts reservations'
            USING ERRCODE = 'PT003';
    END IF;

    -- La cantidad de números.
    v_count := coalesce(array_length(p_numbers, 1), 0);

    IF v_count < v_min OR v_count > v_max THEN
        RAISE EXCEPTION 'TICKET_SIZE_OUT_OF_RANGE: the raffle allows between % and % numbers',
            v_min, v_max
            USING ERRCODE = 'PT005';
    END IF;

    -- Los números están en el pool y libres.
    FOREACH v_number IN ARRAY p_numbers LOOP
        IF v_number < 0 OR v_number >= v_pool THEN
            RAISE EXCEPTION 'TICKET_SIZE_OUT_OF_RANGE: the number % is outside the pool of %',
                v_number, v_pool
                USING ERRCODE = 'PT005';
        END IF;

        IF EXISTS (
            SELECT 1 FROM ticket_numbers tn
             WHERE tn.raffle_id = p_raffle_id
               AND tn.number = v_number
        ) THEN
            RAISE EXCEPTION 'NUMBER_TAKEN: the number % is already reserved', v_number
                USING ERRCODE = 'PT004';
        END IF;
    END LOOP;

    -- El ticket.
    INSERT INTO tickets (raffle_id, user_id, business_id, code, numbers_count,
                         auto_picked, source)
    VALUES (p_raffle_id, p_user_id, v_business_id, p_code, v_count,
            p_auto_picked, p_source)
    RETURNING id INTO v_ticket_id;

    -- Los números.
    INSERT INTO ticket_numbers (ticket_id, raffle_id, number)
    SELECT v_ticket_id, p_raffle_id, unnest(p_numbers);

    -- Los contadores desnormalizados del sorteo.
    PERFORM apply_reservation_counters(v_ticket_id);

    RETURN v_ticket_id;
END;
$$;

COMMENT ON FUNCTION reserve_ticket(uuid, uuid, text, int[], boolean, text) IS
    'Reserva atómica de números para un participante. La función es la que bloquea la fila del sorteo, valida el estado y el límite por usuario, inserta el ticket con sus números y recalcula los contadores.';
