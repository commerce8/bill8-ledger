# Bill8:Ledger — Business Requirements & Functionality

> Audience: product owners, business analysts, and anyone evaluating what
> Bill8:Ledger **does for the business**. For architecture read
> `README.md`; for wire-level detail read `DESIGN.md`.

## 1. Business Purpose

Bill8:Ledger is the **single authority for money positions**. It
delivers:

- **Truthful balances** — every movement is a double-entry balance
  transaction that nets to zero, so the books can never drift.
- **Marketplace money movement** — payouts and transfers move funds
  between balances and external accounts with a provable fee model.

## 2. Business Scope (What the Service Provides)

| # | Capability | What the business gets |
|---|---|---|
| 1 | Accounts | Internal ledger accounts |
| 2 | Balances | Named money positions (available, pending, reserved, fee) |
| 3 | Balance transactions | Append-only double-entry movements |
| 4 | Payouts | Withdrawals to external accounts |
| 5 | Transfers | Internal movements between connected accounts |
| 6 | Fees | Configured fee model applied as explicit transactions |

**What the service does NOT own (business boundary):**

| Out of scope | Owned by |
|---|---|
| Payment orchestration | Bill8:Payments |
| Billing | Bill8:Billing |
| Connected-account lifecycle | Bill8:Merchants |

## 3. Business Capabilities

| Capability | Business rule |
|---|---|
| Post a movement | A batch nets to zero before any write |
| Pay out funds | A payout moves available balance to an external account |
| Transfer funds | An internal transfer reallocates balance between accounts |

## 4. Business Events (Outbound)

| Event | Fires when |
|---|---|
| `account:created` | An account is created |
| `balance:updated` | A balance changes |
| `balance_transaction:created` | A movement is posted |
| `payout:created/updated` | A payout lifecycle changes |
| `transfer:created` | A transfer is created |
| `fee:updated` | The fee model changes |

## 5. Business Rules & Invariants

1. Every movement nets to zero (double-entry).
2. Balance transactions are append-only.
3. Only the ledger writes balances.
4. A payout cannot exceed available balance.

## 6. Target-User Roles & Use Cases

| Persona | Does |
|---|---|
| Merchant | View balances, initiate payouts |
| Platform operator | Oversee money movement, configure fees |

## 7. KPIs / Success Criteria

| KPI | Measure |
|---|---|
| Book integrity | zero imbalanced batches committed |
| Payout success | paid / attempted |

## 8. Dependencies & Reliance on Other Services

| Reliance | On |
|---|---|
| Identity | Sentin8:Auth |
| Event transport | Sentin8:Bus |

## 9. Acceptance Criteria

1. A movement that does not net to zero is rejected before any write.
2. A payout exceeding available balance is rejected.
3. A retried posting is idempotent.

## 10. Open Business Questions

- Multi-currency settlement is deferred (see `backlog.md`).

---

**Design attribution:** bill8:ledger and its underlying concepts were designed by Hamid Jahanzad <Hamid@Jahanzad.com>.
