-- Identity tokens: magic links and refresh token families.
--
-- The access token is a stateless JWT and needs no storage. The other two
-- do: a magic link has to be single use, and a refresh token has to rotate
-- with its whole family revoked on reuse.
--
-- The email is stored on the token because a magic link is issued before
-- the user exists: `user_id` is filled in when the link is consumed.
CREATE TABLE auth_tokens (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      uuid REFERENCES users(id) ON DELETE CASCADE,
    email        citext NOT NULL,
    kind         text NOT NULL CHECK (kind IN ('magic_link','refresh')),
    token_hash   bytea NOT NULL,
    family_id    uuid,
    expires_at   timestamptz NOT NULL,
    consumed_at  timestamptz,
    revoked_at   timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now(),
    ip_hash      text,
    user_agent   text
);

-- Only the hash of the token is stored: the plaintext exists once, in the
-- email, and is never recoverable from the database.
CREATE UNIQUE INDEX auth_tokens_hash_idx ON auth_tokens (token_hash);
CREATE INDEX auth_tokens_user_kind_idx ON auth_tokens (user_id, kind);
CREATE INDEX auth_tokens_family_idx ON auth_tokens (family_id) WHERE family_id IS NOT NULL;
CREATE INDEX auth_tokens_pending_idx ON auth_tokens (expires_at)
    WHERE consumed_at IS NULL AND revoked_at IS NULL;

-- A magic link belongs to exactly one email and one user at most, and it
-- is consumed at most once.
CREATE UNIQUE INDEX auth_tokens_magic_link_once_idx
    ON auth_tokens (email) WHERE kind = 'magic_link' AND consumed_at IS NULL;

-- A refresh family has at most one live token at a time.
--
-- The rotation revokes the presented token before its successor is
-- inserted, so a second rotation of the same token would leave two live
-- tokens in the family. The index makes that impossible: the second insert
-- fails, and the service reads the zero rows of the revoke as a reuse and
-- takes the family down (`FR-ID-04`).
CREATE UNIQUE INDEX auth_tokens_one_live_per_family_idx
    ON auth_tokens (family_id)
    WHERE kind = 'refresh' AND revoked_at IS NULL;

ALTER TABLE users
    ADD COLUMN last_magic_link_at timestamptz;
