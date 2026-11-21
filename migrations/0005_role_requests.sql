CREATE TABLE role_requests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    requested_role TEXT NOT NULL CHECK (requested_role = 'organizer'),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'rejected')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    reviewed_at TIMESTAMPTZ,
    reviewed_by UUID REFERENCES users(id)
);

CREATE UNIQUE INDEX role_requests_one_pending_per_user_idx
    ON role_requests(user_id)
    WHERE status = 'pending';

CREATE INDEX role_requests_status_created_idx
    ON role_requests(status, created_at DESC);
