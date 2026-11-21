CREATE TABLE IF NOT EXISTS auth_identities (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider VARCHAR(24) NOT NULL CHECK(provider IN ('email','google','apple','metamask')),
    provider_subject VARCHAR(255) NOT NULL,
    email VARCHAR(254), email_verified BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(provider,provider_subject)
);

CREATE INDEX IF NOT EXISTS idx_auth_identities_user ON auth_identities(user_id);

CREATE TABLE IF NOT EXISTS email_otps (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(), email VARCHAR(254) NOT NULL,
    code_hash CHAR(64) NOT NULL, attempts SMALLINT NOT NULL DEFAULT 0,
    expires_at TIMESTAMPTZ NOT NULL, consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_email_otps_email_created ON email_otps(email,created_at DESC);

CREATE TABLE IF NOT EXISTS siwe_nonces (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(), wallet_address VARCHAR(42) NOT NULL,
    nonce VARCHAR(64) NOT NULL UNIQUE, expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ, created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
