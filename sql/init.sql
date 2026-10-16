-- SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
-- SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
-- SPDX-FileCopyrightText: Copyright Contributors to the Bill8:Ledger
-- SPDX-License-Identifier: Apache-2.0

-- Bill8:Ledger — the service's own schema `ledger` (schema-per-service):
-- domain tables + ledger.outbox / ledger.audit_journal producer tables,
-- all FORCE-RLS env-scoped.
--
-- Double-entry invariants (ledger DESIGN.md §8): every balance transaction
-- nets to zero (validated pre-write); balance rows are mutated only via
-- balance transactions; balances are currency-keyed and transact only in
-- their own currency.

CREATE SCHEMA IF NOT EXISTS ledger;

-- Internal ledger accounts (customer / platform / connected).
CREATE TABLE IF NOT EXISTS ledger.accounts (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    env_id          UUID NOT NULL,
    account_type    TEXT NOT NULL CHECK (account_type IN ('customer', 'platform', 'connected')),
    display_name    TEXT NOT NULL,
    metadata        JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    resource_version INTEGER NOT NULL DEFAULT 1,
    deleted_at      TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_ledger_accounts_env ON ledger.accounts (env_id);

-- Named money positions per currency: available, pending,
-- connected_reserved, platform_fee. Keyed (account_id, name, currency).
CREATE TABLE IF NOT EXISTS ledger.balances (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    env_id          UUID NOT NULL,
    account_id      UUID NOT NULL REFERENCES ledger.accounts(id),
    name            TEXT NOT NULL CHECK (name IN ('available', 'pending', 'connected_reserved', 'platform_fee')),
    currency        TEXT NOT NULL,
    amount          DECIMAL(20,2) NOT NULL DEFAULT 0 CHECK (amount >= 0),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (account_id, name, currency)
);
CREATE INDEX IF NOT EXISTS idx_ledger_balances_env ON ledger.balances (env_id);
CREATE INDEX IF NOT EXISTS idx_ledger_balances_account ON ledger.balances (account_id);

-- Append-only double-entry movements. No UPDATE path exists; the service
-- only ever INSERTs. The pre-write validation guarantees debit ==
-- credit legs with matching amount/currency per batch.
CREATE TABLE IF NOT EXISTS ledger.balance_transactions (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    env_id              UUID NOT NULL,
    debit_balance_id    UUID NOT NULL REFERENCES ledger.balances(id),
    credit_balance_id   UUID NOT NULL REFERENCES ledger.balances(id),
    amount              DECIMAL(20,2) NOT NULL CHECK (amount > 0),
    currency            TEXT NOT NULL,
    transaction_type    TEXT NOT NULL CHECK (transaction_type IN ('charge', 'refund', 'payout', 'transfer', 'fee', 'fx', 'dispute_reversal', 'split')),
    source_type         TEXT,
    source_id           UUID,
    idempotency_key     TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_ledger_balance_transactions_env ON ledger.balance_transactions (env_id);
CREATE INDEX IF NOT EXISTS idx_ledger_balance_transactions_debit ON ledger.balance_transactions (debit_balance_id);
CREATE INDEX IF NOT EXISTS idx_ledger_balance_transactions_credit ON ledger.balance_transactions (credit_balance_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_ledger_balance_transactions_idempotency
    ON ledger.balance_transactions (env_id, idempotency_key) WHERE idempotency_key IS NOT NULL;

-- Payouts: withdrawals to external accounts. State machine
-- pending → paid | failed; terminal states never regress.
CREATE TABLE IF NOT EXISTS ledger.payouts (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    env_id                UUID NOT NULL,
    account_id            UUID NOT NULL REFERENCES ledger.accounts(id),
    external_account_ref  TEXT NOT NULL,
    amount                DECIMAL(20,2) NOT NULL CHECK (amount > 0),
    currency              TEXT NOT NULL,
    status                TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'paid', 'failed')),
    balance_transaction_id UUID REFERENCES ledger.balance_transactions(id),
    idempotency_key       TEXT,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    resource_version     INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_ledger_payouts_env ON ledger.payouts (env_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_ledger_payouts_idempotency
    ON ledger.payouts (env_id, idempotency_key) WHERE idempotency_key IS NOT NULL;

-- Payout schedules: automatic payouts via Cron proxy-back.
CREATE TABLE IF NOT EXISTS ledger.payout_schedules (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    env_id          UUID NOT NULL,
    account_id      UUID NOT NULL REFERENCES ledger.accounts(id),
    external_account_ref TEXT NOT NULL,
    interval        TEXT NOT NULL CHECK (interval IN ('daily', 'weekly', 'monthly')),
    minimum_amount  DECIMAL(20,2) NOT NULL DEFAULT 0,
    currency        TEXT NOT NULL,
    enabled         BOOLEAN NOT NULL DEFAULT true,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    resource_version INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_ledger_payout_schedules_env ON ledger.payout_schedules (env_id);

-- Transfers: internal movements between connected-account balances.
CREATE TABLE IF NOT EXISTS ledger.transfers (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    env_id              UUID NOT NULL,
    from_balance_id     UUID NOT NULL REFERENCES ledger.balances(id),
    to_balance_id       UUID NOT NULL REFERENCES ledger.balances(id),
    amount              DECIMAL(20,2) NOT NULL CHECK (amount > 0),
    currency            TEXT NOT NULL,
    balance_transaction_id UUID REFERENCES ledger.balance_transactions(id),
    idempotency_key     TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_ledger_transfers_env ON ledger.transfers (env_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_ledger_transfers_idempotency
    ON ledger.transfers (env_id, idempotency_key) WHERE idempotency_key IS NOT NULL;

-- Fees: the configured fee model (fixed + bps per transaction type).
-- The fee charged on a charge is captured as an explicit balance
-- transaction crediting the platform_fee balance — never implicit.
CREATE TABLE IF NOT EXISTS ledger.fees (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    env_id          UUID NOT NULL,
    transaction_type TEXT NOT NULL,
    fixed_amount    DECIMAL(20,2) NOT NULL DEFAULT 0,
    bps             INTEGER NOT NULL DEFAULT 0 CHECK (bps >= 0 AND bps <= 10000),
    currency        TEXT NOT NULL DEFAULT 'USD',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (env_id, transaction_type, currency)
);
CREATE INDEX IF NOT EXISTS idx_ledger_fees_env ON ledger.fees (env_id);

-- ── Row-level security: every tenant table FORCE-RLS on env_id ────────
ALTER TABLE ledger.accounts ENABLE ROW LEVEL SECURITY;
ALTER TABLE ledger.accounts FORCE ROW LEVEL SECURITY;
ALTER TABLE ledger.balances ENABLE ROW LEVEL SECURITY;
ALTER TABLE ledger.balances FORCE ROW LEVEL SECURITY;
ALTER TABLE ledger.balance_transactions ENABLE ROW LEVEL SECURITY;
ALTER TABLE ledger.balance_transactions FORCE ROW LEVEL SECURITY;
ALTER TABLE ledger.payouts ENABLE ROW LEVEL SECURITY;
ALTER TABLE ledger.payouts FORCE ROW LEVEL SECURITY;
ALTER TABLE ledger.payout_schedules ENABLE ROW LEVEL SECURITY;
ALTER TABLE ledger.payout_schedules FORCE ROW LEVEL SECURITY;
ALTER TABLE ledger.transfers ENABLE ROW LEVEL SECURITY;
ALTER TABLE ledger.transfers FORCE ROW LEVEL SECURITY;
ALTER TABLE ledger.fees ENABLE ROW LEVEL SECURITY;
ALTER TABLE ledger.fees FORCE ROW LEVEL SECURITY;

-- Tenant isolation policy: env_id must match the tenant context set on
-- the SAME connection/transaction (`sentin8_set_tenant_context` —
-- transaction-local `set_config`).
CREATE POLICY tenant_isolation ON ledger.accounts
    USING (env_id = current_setting('app.env_id', true)::uuid);
CREATE POLICY tenant_isolation ON ledger.balances
    USING (env_id = current_setting('app.env_id', true)::uuid);
CREATE POLICY tenant_isolation ON ledger.balance_transactions
    USING (env_id = current_setting('app.env_id', true)::uuid);
CREATE POLICY tenant_isolation ON ledger.payouts
    USING (env_id = current_setting('app.env_id', true)::uuid);
CREATE POLICY tenant_isolation ON ledger.payout_schedules
    USING (env_id = current_setting('app.env_id', true)::uuid);
CREATE POLICY tenant_isolation ON ledger.transfers
    USING (env_id = current_setting('app.env_id', true)::uuid);
CREATE POLICY tenant_isolation ON ledger.fees
    USING (env_id = current_setting('app.env_id', true)::uuid);

-- ── Producer tables (DBX path, architecture-principles §6.7) ──────────
-- Drained by the shell's post-commit relay to Bus POST /v1/produce.
CREATE TABLE IF NOT EXISTS ledger.outbox (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    env_id              UUID NOT NULL,
    event_type          TEXT NOT NULL,
    stream_family       TEXT NOT NULL,
    payload             JSONB NOT NULL,
    publisher_id_token  TEXT,
    publisher_subject   TEXT,
    relayed             BOOLEAN NOT NULL DEFAULT false,
    relayed_at          TIMESTAMPTZ,
    relay_attempts      INT NOT NULL DEFAULT 0,
    next_retry_at       TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_ledger_outbox_unrelayed ON ledger.outbox (created_at) WHERE relayed = false;

CREATE OR REPLACE FUNCTION ledger.notify_outbox_changed() RETURNS trigger AS $$
BEGIN
    PERFORM pg_notify('ledger_outbox_changed', NEW.id::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS ledger_outbox_notify ON ledger.outbox;
CREATE TRIGGER ledger_outbox_notify AFTER INSERT ON ledger.outbox
    FOR EACH ROW EXECUTE FUNCTION ledger.notify_outbox_changed();

-- Local audit journal (audit crate §6.8, boot step 8c): pushed to Sys's
-- ingest API by the shell's journal pusher; a Sys outage never drops
-- audit (the journal retries).
CREATE TABLE IF NOT EXISTS ledger.audit_journal (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    env_id          UUID NOT NULL,
    action          TEXT NOT NULL,
    target_resource_type TEXT,
    target_resource_id    TEXT,
    actor_user_id   TEXT,
    actor_subject_type TEXT,
    severity        TEXT NOT NULL DEFAULT 'info',
    correlation_id  TEXT,
    before_state    JSONB,
    after_state     JSONB,
    details         JSONB,
    pushed          BOOLEAN NOT NULL DEFAULT false,
    pushed_at       TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_ledger_audit_journal_unpushed
    ON ledger.audit_journal (created_at) WHERE pushed = false;
ALTER TABLE ledger.audit_journal ENABLE ROW LEVEL SECURITY;
ALTER TABLE ledger.audit_journal FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS audit_journal_dml ON ledger.audit_journal;
CREATE POLICY audit_journal_dml ON ledger.audit_journal
    USING (sentin8_context_initialized() AND env_id = sentin8_current_env_id())
    WITH CHECK (sentin8_context_initialized() AND env_id = sentin8_current_env_id());
-- ── Role grants (schema-ownership convention, bank8-ledger's form) ─────
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'bill8_ledger_service') THEN
        ALTER ROLE bill8_ledger_service SET search_path = ledger, public;
        GRANT USAGE ON SCHEMA ledger TO bill8_ledger_service;
        GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA ledger TO bill8_ledger_service;
        GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA ledger TO bill8_ledger_service;
    END IF;
END $$;
