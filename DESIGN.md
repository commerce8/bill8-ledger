# Bill8:Ledger - Detailed Design Document

## 1. Scope & Context

Bill8:Ledger owns: the ledger domain schema in the bill8
vertical database (domain tables + the service leaf schema `ledger` for
the outbox and audit journal), the balance / balance-transaction /
payout / transfer REST API, the double-entry money engine, the
**multi-currency keying** (`(account, currency)` balances, explicit FX
batches), the **destination-charge posting rules** (split-at-charge
batches), the **payout schedule engine**, and the
`bill8:ledger:*` event family.

It does NOT own: payment orchestration (Bill8:Payments calls in to
post transactions), billing (Bill8:Billing calls in for invoice
movements), connected-account lifecycle (Bill8:Merchants creates
balances via the public API), or the fee *policy* - the ledger applies
the fee model as explicit balance transactions but the rate table is
configured by the vertical.

```mermaid
graph TD
    subgraph platform["Sentin8 platform"]
        AUTH["Sentin8:Auth"]
        BUS["Sentin8:Bus"]
    end
    subgraph vertical["bill8 vertical"]
        L["Bill8:Ledger"]
        PAY["Bill8:Payments"]
        BI["Bill8:Billing"]
        M["Bill8:Merchants"]
        PG[("bill8 database")]
    end
    subgraph clients
        BFF["Sentin8:BFF<br/>/proxy/ledger/*"]
    end
    BFF -->|"user tokens"| L
    L <-->|"verify / certificate identity"| AUTH
    L -->|"ledger.outbox → relay"| BUS
    PAY -->|"public API - post transactions"| L
    BI -->|"public API - post transactions"| L
    M -->|"public API - create balances"| L
    L <-->|"schema ledger: domain + producer tables"| PG
```

## 2. Runtime Architecture

Boot is shell-owned. `build_state` adds the ledger wiring: config, Bus
client, AppState, and the sys-family env-init consumer.

**Request lifecycle - balance transaction posting** (the canonical
family; every other handler is a subset):

```mermaid
sequenceDiagram
    participant C as Caller (Payments/Billing/BFF)
    participant L as Bill8:Ledger
    participant DB as PostgreSQL (bill8)
    participant REL as outbox relay
    participant B as Sentin8:Bus
    C->>L: POST /v1/balance-transactions (M2M or user token)
    L->>L: verify token
    L->>L: Shield gate (bill8:ledger:balance-transactions:write)
    L->>DB: BEGIN + set_tenant_context(env)
    L->>DB: idempotency ON CONFLICT check
    L->>DB: double-entry validation (Σ debit == Σ credit)
    alt does not net to zero
        L-->>C: 422 (no rows written)
    end
    L->>DB: debit balance row + credit balance row (row locks)
    L->>DB: balance_transaction record (append-only)
    L->>DB: ledger.outbox insert (same tx)
    L->>DB: COMMIT
    L-->>C: 200 transaction + balance snapshot
    REL->>DB: drain unrelayed outbox row
    REL->>B: POST /v1/produce (M2M)
```

**Payout lifecycle**:

```mermaid
sequenceDiagram
    participant C as Caller (Merchants/portal)
    participant L as Bill8:Ledger
    participant DB as PostgreSQL
    C->>L: POST /v1/payouts (external_account, amount)
    L->>L: Shield gate
    L->>DB: available balance check (row lock)
    alt amount > available
        L-->>C: 422 (insufficient funds)
    end
    L->>DB: payout record status=pending
    L->>DB: balance transaction (available → pending_out)
    L->>DB: COMMIT + outbox payout:created
    L-->>C: 200 payout pending
    Note over L: external settlement updates payout → paid (guarded)
```

## 3. Data Model

Domain tables in schema `ledger`, all `env_id`-scoped + FORCE-RLS; the
same schema carries the producer tables.

```mermaid
erDiagram
    balances ||--o{ balance_transactions : "moved by"
    accounts ||--o{ balances : "holds"
    balances ||--o{ payouts : "funded by"
    accounts ||--o{ transfers : "participates in"
    balances ||--o{ transfers : "reallocated by"
```

| Table | Purpose | RLS tier |
|---|---|---|
| `accounts` | Internal ledger accounts (customer / platform / connected) | env |
| `balances` | Named money positions per currency: `available`, `pending`, `connected_reserved`, `platform_fee` - keyed `(account_id, name, currency)` | env |
| `balance_transactions` | Append-only double-entry movements | env |
| `payouts` | Withdrawals to external accounts | env |
| `payout_schedules` | Automatic payout intervals (daily/weekly/monthly + minimum) | env |
| `transfers` | Internal movements between connected accounts | env |
| `fees` | Configured fee model (per-transaction + percentage) | env |
| `fx_rates` | Conversion rates; every conversion posts as an explicit batch with a spread fee | env |

**Multi-currency** - balances and movements are currency-keyed; a
balance transacts only in its own currency - cross-currency movement
is rejected pre-write (422, nothing written). Conversion between
currencies is an **explicit** double-entry batch (rate + spread fee
posted, never implicit), owned by the FX story.

**Destination-charge posting rules** - a split-at-charge
(`on_behalf_of` + `application_fee_amount`) is a single validated
batch: debit the payer's side, credit the connected account's
`available`, credit the platform `platform_fee` - Σ legs == charge
amount, net-zero enforced by the same pre-write check.

**State machine**:

```mermaid
stateDiagram-v2
    [*] --> pending: create payout
    pending --> paid: settle
    pending --> failed: reject
    paid --> [*]
```

**Producer tables** (platform contracts, `ledger` leaf schema):
`ledger.outbox` (DBX producer path - architecture-principles §6.7) and
`ledger.audit_journal` (the 15-column contract of
`sentin8_audit::audit_journal_sql`, AUDJOUR-1 - env-pinned RLS, drained
to Sys `POST /v1/audit` by the shell's boot-8c pusher).

## 4. API Surface

| Endpoint group | Prefix | Authorization | Events |
|---|---|---|---|
| Accounts | `/v1/accounts` | `bill8:ledger:accounts:{read,write}` | `account:created` |
| Balances | `/v1/balances` | `bill8:ledger:balances:{read,write}` | `balance:updated` |
| Balance transactions | `/v1/balance-transactions` | `bill8:ledger:balance-transactions:{read,write}` | `balance_transaction:created` |
| Payouts | `/v1/payouts` | `bill8:ledger:payouts:{read,write}` | `payout:created/updated` |
| Payout schedules | `/v1/payout-schedules` | `bill8:ledger:payout-schedules:{read,write}` | `payout_schedule:created/updated` |
| Transfers | `/v1/transfers` | `bill8:ledger:transfers:{read,write}` | `transfer:created` |
| Fees | `/v1/fees` | `bill8:ledger:fees:{read,write}` | `fee:updated` |
| Internal (proxy-back) | `/v1/internal/*` | M2M | - |

## 5. Cross-Cutting Concerns

- **Authorization**: Shield-gated; scope-mirror sets seeded at enroll.
- **Tenant isolation**: `env_id` RLS + explicit `WHERE env_id`.
- **M2M**: certificate identity per workload (CERTFREE-4V) (`ledger`, `ledger-policy`,
  `ledger-audit`).
- **Audit**: local `ledger.audit_journal` → Sys pusher.
- **Idempotency**: `(env_id, idempotency_key)` unique on postings.
- **TLS**: in-process PQ-TLS.

## 6. Failure Modes & Recovery

```mermaid
flowchart TD
    IMBALANCE["batch does not net to zero"] -->|"pre-write"| REJ["422 - nothing written"]
    INSUF["payout > available"] -->|"row lock + check"| REJ2["422 - nothing written"]
    CRASH["crash mid-transaction"] -->|"atomic"| RB["balances + tx + outbox roll back"]
    RELAY_FAIL["Bus outage"] -->|"producer"| OUTBOX["rows persist in ledger.outbox<br/>relay retries ≤10"]
```

## 7. Concurrency & Performance Model

- Pool cap 3; row locks on balance rows serialize concurrent movements
  per balance (correctness over throughput).
- Double-entry validation is pure and runs before any write.

## 8. Invariants, Insecure-Defaults Rejected, Unsafe Rationale

- Every balance transaction nets to zero (Σ debit == Σ credit).
- Balance transactions are append-only; a balance is only ever mutated
  via a balance transaction.
- A balance transacts only in its own currency; every FX conversion is
  an explicit batch with a posted spread fee.
- No service other than the ledger writes balances; a settlement
  channel never mutates balances (payments DESIGN.md §2b).
- Destination-charge splits sum exactly to the charge amount.
- `#![forbid(unsafe_code)]`.
- Rejected: single-sided ledger rows; a shared superuser DSN; blanket
  schema grants.

## 9. Testing Strategy

```mermaid
graph TD
    U["Unit - double-entry math, fee model,<br/>payout state machine"] --> I["Integration - DB-backed posting/payout<br/>suites, registration checks"]
    I --> E["E2E - e2e-ts UI + BFF proxy only"]
```

## 10. Open Questions

- FX conversion and settlement currency per connected account are
  sequenced as two stories - presentment first (`LED-5a`), conversion
  after (`LED-5b`) - both at final target state when built; no phased
  rollouts (see `backlog.md`).

---

**Design attribution:** bill8:ledger and its underlying concepts were designed by Hamid Jahanzad <Hamid@Jahanzad.com>.
