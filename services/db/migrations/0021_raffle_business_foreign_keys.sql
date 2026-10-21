-- El draw y el premio nombran el sorteo de su propio comercio.
--
-- `tickets` ya lo tenía (la foreign key compuesta `(raffle_id,
-- business_id)` de la migración `0016`). El draw es la tabla más
-- importante del producto: decide quién gana. Un draw que apunta al sorteo
-- de otro comercio rompe el aislamiento en el lugar donde no se puede
-- tolerar, y la foreign key compuesta es la garantía declarativa que no
-- depende del handler.
--
-- La pareja `(id, business_id)` de `raffles` ya es única (la misma
-- migración), así que las dos foreign keys compuestas se apoyan en ella.

ALTER TABLE draws
    ADD CONSTRAINT draws_raffle_business_fkey
    FOREIGN KEY (raffle_id, business_id)
    REFERENCES raffles (id, business_id)
    ON DELETE CASCADE
    NOT VALID;

ALTER TABLE prizes
    ADD CONSTRAINT prizes_raffle_business_fkey
    FOREIGN KEY (raffle_id, business_id)
    REFERENCES raffles (id, business_id)
    ON DELETE CASCADE
    NOT VALID;

-- Las filas que ya existían se validan sólo si no hay huérfanas: la
-- constraint sigue siendo la garantía para toda escritura nueva.
DO $$
BEGIN
    ALTER TABLE draws VALIDATE CONSTRAINT draws_raffle_business_fkey;
EXCEPTION
    WHEN foreign_key_violation THEN
        RAISE NOTICE 'draws has rows with a raffle of another business: the constraint stays NOT VALID';
END;
$$;

DO $$
BEGIN
    ALTER TABLE prizes VALIDATE CONSTRAINT prizes_raffle_business_fkey;
EXCEPTION
    WHEN foreign_key_violation THEN
        RAISE NOTICE 'prizes has rows with a raffle of another business: the constraint stays NOT VALID';
END;
$$;

-- El lado que referencia necesita su índice: sin él, el borrado de un
-- sorteo recorre la tabla entera para encontrar las filas que lo nombran.
CREATE INDEX draws_raffle_business_idx ON draws (raffle_id, business_id);
CREATE INDEX prizes_raffle_business_idx ON prizes (raffle_id, business_id);

COMMENT ON CONSTRAINT draws_raffle_business_fkey ON draws IS
    'El draw es del mismo comercio que su sorteo. La foreign key compuesta sobrevive a cualquier cambio del handler.';
COMMENT ON CONSTRAINT prizes_raffle_business_fkey ON prizes IS
    'El premio es del mismo comercio que su sorteo. La foreign key compuesta sobrevive a cualquier cambio del handler.';
