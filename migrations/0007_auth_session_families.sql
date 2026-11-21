ALTER TABLE auth_sessions
    ADD COLUMN IF NOT EXISTS family_id UUID;

UPDATE auth_sessions
SET family_id = gen_random_uuid()
WHERE family_id IS NULL;

ALTER TABLE auth_sessions
    ALTER COLUMN family_id SET DEFAULT gen_random_uuid(),
    ALTER COLUMN family_id SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_auth_sessions_family_active
    ON auth_sessions(family_id)
    WHERE revoked_at IS NULL;
