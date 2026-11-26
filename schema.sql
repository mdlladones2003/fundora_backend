-- Canonical schema applied transactionally by the backend at startup.
-- The database must be fresh or already match this schema; IF NOT EXISTS does not upgrade columns.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE IF NOT EXISTS roles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL UNIQUE
        CHECK (name IN ('guest', 'donor', 'organizer', 'administrator')),
    description TEXT
);

INSERT INTO roles(name, description) VALUES
    ('donor', 'Can donate and view own activity'),
    ('organizer', 'Can manage own campaigns and request withdrawals'),
    ('administrator', 'Platform administrator')
ON CONFLICT(name) DO NOTHING;

CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email TEXT NOT NULL UNIQUE,
    display_name TEXT,
    avatar_url TEXT,
    wallet_address TEXT UNIQUE,
    oauth_provider TEXT,
    oauth_subject TEXT,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(oauth_provider, oauth_subject)
);

CREATE TABLE IF NOT EXISTS user_roles (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role_id UUID NOT NULL REFERENCES roles(id),
    PRIMARY KEY(user_id, role_id)
);

CREATE TABLE IF NOT EXISTS campaigns (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organizer_id UUID NOT NULL REFERENCES users(id),
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    image_url TEXT,
    goal_amount NUMERIC(96, 18) NOT NULL CHECK (goal_amount > 0),
    raised_amount NUMERIC(96, 18) NOT NULL DEFAULT 0 CHECK (raised_amount >= 0),
    currency VARCHAR(12) NOT NULL DEFAULT 'PHP',
    network TEXT,
    contract_address TEXT,
    status TEXT NOT NULL DEFAULT 'draft'
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
        )),
    starts_at TIMESTAMPTZ,
    ends_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    category TEXT,
    beneficiary_name TEXT,
    beneficiary_details TEXT,
    supporting_info TEXT,
    review_feedback TEXT,
    reviewed_by UUID REFERENCES users(id),
    reviewed_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS campaigns_status_created_idx
    ON campaigns(status, created_at DESC);

CREATE TABLE IF NOT EXISTS campaign_views (
    campaign_id UUID NOT NULL REFERENCES campaigns(id) ON DELETE CASCADE,
    session_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (campaign_id, session_id)
);

CREATE TABLE IF NOT EXISTS donations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    campaign_id UUID NOT NULL REFERENCES campaigns(id),
    donor_id UUID REFERENCES users(id),
    amount NUMERIC(96, 18) NOT NULL CHECK (amount > 0),
    currency VARCHAR(12) NOT NULL DEFAULT 'PHP',
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'confirmed', 'failed', 'refunded')),
    is_anonymous BOOLEAN NOT NULL DEFAULT FALSE,
    message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at TIMESTAMPTZ,
    donor_wallet_address VARCHAR(42),
    recipient_wallet_address VARCHAR(42),
    payment_amount NUMERIC(96, 18) CHECK (payment_amount IS NULL OR payment_amount > 0),
    payment_currency VARCHAR(12),
    exchange_rate NUMERIC(96, 18) CHECK (exchange_rate IS NULL OR exchange_rate > 0),
    exchange_rate_provider TEXT,
    exchange_rate_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS donations_campaign_status_idx
    ON donations(campaign_id, status);
CREATE INDEX IF NOT EXISTS donations_donor_idx
    ON donations(donor_id, created_at DESC);
CREATE INDEX IF NOT EXISTS donations_campaign_status_payment_currency_idx
    ON donations(campaign_id, status, payment_currency);

CREATE TABLE IF NOT EXISTS payment_attempts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    donation_id UUID NOT NULL UNIQUE REFERENCES donations(id) ON DELETE CASCADE,
    provider TEXT NOT NULL CHECK (provider IN ('mock', 'xendit')),
    method TEXT NOT NULL CHECK (method IN ('GCASH', 'MAYA')),
    reference_id TEXT NOT NULL UNIQUE,
    session_id TEXT UNIQUE,
    checkout_url TEXT,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'paid', 'failed', 'expired')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    paid_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_payment_attempts_xendit_pending
    ON payment_attempts(updated_at)
    WHERE provider='xendit' AND status='pending';

CREATE TABLE IF NOT EXISTS withdrawals (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    campaign_id UUID NOT NULL REFERENCES campaigns(id),
    requester_id UUID NOT NULL REFERENCES users(id),
    amount NUMERIC(78, 0) NOT NULL CHECK (amount > 0),
    destination_address TEXT NOT NULL,
    purpose TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'approved', 'rejected', 'processing', 'completed', 'failed')),
    reviewed_by UUID REFERENCES users(id),
    reviewed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    review_reason TEXT
);

CREATE TABLE IF NOT EXISTS blockchain_transactions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    donation_id UUID REFERENCES donations(id),
    withdrawal_id UUID REFERENCES withdrawals(id),
    network TEXT NOT NULL,
    tx_hash TEXT NOT NULL,
    from_address TEXT,
    to_address TEXT,
    block_number BIGINT,
    confirmations INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'confirmed', 'failed')),
    verified BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at TIMESTAMPTZ,
    CHECK (
        (donation_id IS NOT NULL)::int + (withdrawal_id IS NOT NULL)::int = 1
    ),
    UNIQUE(network, tx_hash),
    review_note TEXT,
    flagged_by UUID REFERENCES users(id),
    flagged_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS tx_donation_idx ON blockchain_transactions(donation_id);
CREATE INDEX IF NOT EXISTS tx_withdrawal_idx ON blockchain_transactions(withdrawal_id);

CREATE TABLE IF NOT EXISTS auth_identities (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider VARCHAR(24) NOT NULL
        CHECK (provider IN ('email', 'google', 'apple', 'metamask')),
    provider_subject VARCHAR(255) NOT NULL,
    email VARCHAR(254),
    email_verified BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(provider, provider_subject)
);

CREATE INDEX IF NOT EXISTS idx_auth_identities_user ON auth_identities(user_id);

CREATE TABLE IF NOT EXISTS email_otps (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email VARCHAR(254) NOT NULL,
    code_hash CHAR(64) NOT NULL,
    attempts SMALLINT NOT NULL DEFAULT 0,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_email_otps_email_created
    ON email_otps(email, created_at DESC);

CREATE TABLE IF NOT EXISTS siwe_nonces (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    wallet_address VARCHAR(42) NOT NULL,
    nonce VARCHAR(64) NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    purpose TEXT NOT NULL DEFAULT 'login'
        CHECK (purpose IN ('login', 'wallet_link'))
);

CREATE TABLE IF NOT EXISTS auth_sessions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    refresh_token_hash CHAR(64) NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    replaced_by UUID REFERENCES auth_sessions(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    family_id UUID NOT NULL DEFAULT gen_random_uuid()
);

CREATE INDEX IF NOT EXISTS idx_auth_sessions_user ON auth_sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_auth_sessions_expiry ON auth_sessions(expires_at);
CREATE INDEX IF NOT EXISTS idx_auth_sessions_family_active
    ON auth_sessions(family_id)
    WHERE revoked_at IS NULL;

CREATE TABLE IF NOT EXISTS role_requests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    requested_role TEXT NOT NULL CHECK (requested_role = 'organizer'),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'approved', 'rejected')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    reviewed_at TIMESTAMPTZ,
    reviewed_by UUID REFERENCES users(id)
);

CREATE UNIQUE INDEX IF NOT EXISTS role_requests_one_pending_per_user_idx
    ON role_requests(user_id)
    WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS role_requests_status_created_idx
    ON role_requests(status, created_at DESC);

CREATE TABLE IF NOT EXISTS notifications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    href TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    read_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS notifications_user_created_idx
    ON notifications(user_id, created_at DESC);

CREATE TABLE IF NOT EXISTS audit_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    actor_id UUID REFERENCES users(id),
    action TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id UUID,
    details JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS audit_logs_created_idx ON audit_logs(created_at DESC);
CREATE INDEX IF NOT EXISTS audit_logs_entity_idx
    ON audit_logs(entity_type, entity_id, created_at DESC);
