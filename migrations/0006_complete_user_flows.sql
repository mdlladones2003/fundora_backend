ALTER TABLE campaigns
    ADD COLUMN category TEXT,
    ADD COLUMN beneficiary_name TEXT,
    ADD COLUMN beneficiary_details TEXT,
    ADD COLUMN supporting_info TEXT,
    ADD COLUMN review_feedback TEXT,
    ADD COLUMN reviewed_by UUID REFERENCES users(id),
    ADD COLUMN reviewed_at TIMESTAMPTZ;

ALTER TABLE campaigns DROP CONSTRAINT campaigns_status_check;
ALTER TABLE campaigns ADD CONSTRAINT campaigns_status_check
    CHECK (status IN (
        'draft',
        'pending_review',
        'changes_requested',
        'published',
        'active',
        'paused',
        'completed',
        'rejected',
        'archived'
    ));

ALTER TABLE withdrawals ADD COLUMN review_reason TEXT;

ALTER TABLE blockchain_transactions
    ADD COLUMN review_note TEXT,
    ADD COLUMN flagged_by UUID REFERENCES users(id),
    ADD COLUMN flagged_at TIMESTAMPTZ;

CREATE TABLE notifications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    href TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    read_at TIMESTAMPTZ
);
CREATE INDEX notifications_user_created_idx ON notifications(user_id, created_at DESC);

CREATE TABLE audit_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    actor_id UUID REFERENCES users(id),
    action TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id UUID,
    details JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX audit_logs_created_idx ON audit_logs(created_at DESC);
CREATE INDEX audit_logs_entity_idx ON audit_logs(entity_type, entity_id, created_at DESC);
