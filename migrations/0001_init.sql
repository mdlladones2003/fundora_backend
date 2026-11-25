CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE roles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL UNIQUE CHECK (name IN ('guest','donor','organizer','administrator')),
    description TEXT
);
INSERT INTO roles(name,description) VALUES
    ('donor','Can donate and view own activity'),
    ('organizer','Can manage own campaigns and request withdrawals'),
    ('administrator','Platform administrator')
ON CONFLICT(name) DO NOTHING;

CREATE TABLE users (
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
    UNIQUE(oauth_provider,oauth_subject)
);

CREATE TABLE user_roles (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role_id UUID NOT NULL REFERENCES roles(id),
    PRIMARY KEY(user_id,role_id)
);

CREATE TABLE campaigns (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organizer_id UUID NOT NULL REFERENCES users(id),
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    image_url TEXT,
    goal_amount NUMERIC(78,0) NOT NULL CHECK(goal_amount > 0),
    raised_amount NUMERIC(78,0) NOT NULL DEFAULT 0 CHECK(raised_amount >= 0),
    currency VARCHAR(12) NOT NULL DEFAULT 'PHP',
    network TEXT,
    contract_address TEXT,
    status TEXT NOT NULL DEFAULT 'draft' CHECK(status IN ('draft','published','active','paused','completed','rejected','archived')),
    starts_at TIMESTAMPTZ,
    ends_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX campaigns_status_created_idx ON campaigns(status,created_at DESC);

CREATE TABLE donations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    campaign_id UUID NOT NULL REFERENCES campaigns(id),
    donor_id UUID REFERENCES users(id),
    amount NUMERIC(78,0) NOT NULL CHECK(amount > 0),
    currency VARCHAR(12) NOT NULL DEFAULT 'PHP',
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','confirmed','failed','refunded')),
    is_anonymous BOOLEAN NOT NULL DEFAULT FALSE,
    message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at TIMESTAMPTZ
);
CREATE INDEX donations_campaign_status_idx ON donations(campaign_id,status);
CREATE INDEX donations_donor_idx ON donations(donor_id,created_at DESC);

CREATE TABLE withdrawals (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    campaign_id UUID NOT NULL REFERENCES campaigns(id),
    requester_id UUID NOT NULL REFERENCES users(id),
    amount NUMERIC(78,0) NOT NULL CHECK(amount > 0),
    destination_address TEXT NOT NULL,
    purpose TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','approved','rejected','processing','completed','failed')),
    reviewed_by UUID REFERENCES users(id),
    reviewed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE blockchain_transactions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    donation_id UUID REFERENCES donations(id),
    withdrawal_id UUID REFERENCES withdrawals(id),
    network TEXT NOT NULL,
    tx_hash TEXT NOT NULL,
    from_address TEXT,
    to_address TEXT,
    block_number BIGINT,
    confirmations INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','confirmed','failed')),
    verified BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at TIMESTAMPTZ,
    CHECK ((donation_id IS NOT NULL)::int + (withdrawal_id IS NOT NULL)::int = 1),
    UNIQUE(network,tx_hash)
);

CREATE INDEX tx_donation_idx ON blockchain_transactions(donation_id);
CREATE INDEX tx_withdrawal_idx ON blockchain_transactions(withdrawal_id);
