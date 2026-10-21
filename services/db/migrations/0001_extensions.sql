-- Extensions the schema depends on.
--
-- pgcrypto backs gen_random_uuid(), the documented source of every primary key
-- (docs/data-model.md §2): an id the application generates keeps a client from
-- guessing another tenant's ids.
-- citext makes the login case-insensitive without storing the value twice.
-- pg_trgm backs the fuzzy search over business and raffle names.
CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS citext;
CREATE EXTENSION IF NOT EXISTS pg_trgm;
