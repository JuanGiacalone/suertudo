-- Notification preferences.
--
-- `users.marketing_opt_in` only carries the marketing choice. The
-- operational channels (push and email) need their own place, because a
-- participant can refuse the marketing mail and still want the push that
-- says they won a prize.
--
-- The defaults keep the operational channels on: a participant who never
-- expressed a preference still wants to know they won.
ALTER TABLE users
    ADD COLUMN notification_preferences jsonb NOT NULL DEFAULT
        '{"push": true, "email": true, "marketing": false}'::jsonb;

ALTER TABLE users
    ADD CONSTRAINT users_notification_preferences_ck CHECK (
        notification_preferences ? 'push'
        AND notification_preferences ? 'email'
        AND notification_preferences ? 'marketing'
        AND jsonb_typeof(notification_preferences -> 'push') = 'boolean'
        AND jsonb_typeof(notification_preferences -> 'email') = 'boolean'
        AND jsonb_typeof(notification_preferences -> 'marketing') = 'boolean'
    );
