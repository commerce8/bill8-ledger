// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Bill8:Ledger
// SPDX-License-Identifier: Apache-2.0

//! Balance-transaction handlers - the double-entry posting engine.
//!
//! The pre-write validation (§2 of the DDD): a posting debits one balance
//! and credits another of the SAME currency; any imbalance or currency
//! mismatch writes NOTHING. Balance rows are locked (`FOR UPDATE`) for the
//! duration of the posting so concurrent movements serialize per balance.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use bill8::ledger::contracts::{BalanceTransaction, PostTransactionRequest};
use rust_decimal::Decimal;
use sqlx::Row;
use uuid::Uuid;

use crate::auth;
use crate::handlers::{err_db, set_tenant_context, AppState};
use crate::types::LedgerError;

/// Query params for transaction reads.
#[derive(Debug, serde::Deserialize)]
pub struct ListTransactionsQuery {
    pub balance_id: Option<Uuid>,
}

/// `POST /v1/balance-transactions` - post one double-entry movement.
///
/// The money path: BEGIN → tenant context → idempotency check →
/// currency/currency validation → row-lock both balances → append the
/// transaction → mutate both balance rows → outbox (same tx) → COMMIT.
/// A rejected batch writes NOTHING (imbalanced, currency mismatch, or
/// insufficient debit funds all fail pre-write).
#[utoipa::path(
    post,
    path = "/v1/balance-transactions",
    request_body = PostTransactionRequest,
    responses(
        (status = 200, description = "Transaction posted", body = BalanceTransaction),
        (status = 403, description = "Missing bill8:ledger:balance-transactions:write"),
        (status = 422, description = "Imbalanced/currency-mismatch - nothing written"),
    ),
    security(("bill8:ledger:balance-transactions:write" = [])),
    tag = "ledger",
)]
pub async fn post_transaction(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PostTransactionRequest>,
) -> Result<Json<BalanceTransaction>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("bill8:ledger:balance-transactions:write"),
    )
    .await?;

    let amount: Decimal = req.amount.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "amount must be a decimal string".into(),
        )
    })?;
    if amount <= Decimal::ZERO {
        return Err((StatusCode::BAD_REQUEST, "amount must be positive".into()));
    }

    let mut conn = state.pool.begin().await.map_err(err_db)?;
    set_tenant_context(&mut conn, a.env_id())
        .await
        .map_err(err_db)?;

    // Idempotency: a retried posting returns the original row.
    if let Some(key) = req.idempotency_key.as_deref() {
        if let Ok(existing) = sqlx::query(
            r#"SELECT id, debit_balance_id, credit_balance_id, amount, currency,
                      transaction_type, source_type, source_id, created_at
               FROM ledger.balance_transactions WHERE env_id = $1 AND idempotency_key = $2"#,
        )
        .bind(a.env_id())
        .bind(key)
        .fetch_optional(&mut *conn)
        .await
        {
            if let Some(row) = existing {
                conn.commit().await.map_err(err_db)?;
                return Ok(Json(row_to_tx(&row)));
            }
        }
    }

    // Row-lock both legs; validate currency + debit funds before writing.
    let debit = fetch_balance_for_update(&mut conn, req.debit_balance_id, a.env_id()).await?;
    let credit = fetch_balance_for_update(&mut conn, req.credit_balance_id, a.env_id()).await?;

    if debit.currency != req.currency || credit.currency != req.currency {
        return Err(map_domain(LedgerError::CurrencyMismatch {
            balance: if debit.currency != req.currency {
                debit.id
            } else {
                credit.id
            },
            balance_currency: if debit.currency != req.currency {
                debit.currency.clone()
            } else {
                credit.currency.clone()
            },
            posting_currency: req.currency.clone(),
        }));
    }
    if debit.amount < amount {
        return Err(map_domain(LedgerError::InsufficientFunds {
            available: debit.amount.to_string(),
            requested: req.amount.clone(),
        }));
    }

    let tx_id = Uuid::new_v4();
    let inserted = sqlx::query(
        r#"INSERT INTO ledger.balance_transactions
               (id, env_id, debit_balance_id, credit_balance_id, amount, currency,
                transaction_type, source_type, source_id, idempotency_key)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING id, debit_balance_id, credit_balance_id, amount, currency,
                     transaction_type, source_type, source_id, created_at"#,
    )
    .bind(tx_id)
    .bind(a.env_id())
    .bind(req.debit_balance_id)
    .bind(req.credit_balance_id)
    .bind(amount)
    .bind(&req.currency)
    .bind(&req.transaction_type)
    .bind(req.source_type.as_deref())
    .bind(req.source_id)
    .bind(req.idempotency_key.as_deref())
    .fetch_one(&mut *conn)
    .await
    .map_err(err_db)?;

    // Mutate both balance rows (locked - serialized per balance).
    sqlx::query(
        "UPDATE ledger.balances SET amount = amount - $1, updated_at = now() WHERE id = $2",
    )
    .bind(amount)
    .bind(req.debit_balance_id)
    .execute(&mut *conn)
    .await
    .map_err(err_db)?;
    sqlx::query(
        "UPDATE ledger.balances SET amount = amount + $1, updated_at = now() WHERE id = $2",
    )
    .bind(amount)
    .bind(req.credit_balance_id)
    .execute(&mut *conn)
    .await
    .map_err(err_db)?;

    // Outbox-first: the event rides in the same transaction.
    sqlx::query(
        r#"INSERT INTO ledger.outbox (env_id, event_type, payload)
           VALUES ($1, 'bill8:ledger:balance_transaction:created', $2)"#,
    )
    .bind(a.env_id())
    .bind(serde_json::json!({
        "transaction_id": tx_id,
        "debit_balance_id": req.debit_balance_id,
        "credit_balance_id": req.credit_balance_id,
        "amount": req.amount,
        "currency": req.currency,
        "transaction_type": req.transaction_type,
        "source_type": req.source_type,
        "source_id": req.source_id,
    }))
    .execute(&mut *conn)
    .await
    .map_err(err_db)?;

    conn.commit().await.map_err(err_db)?;
    Ok(Json(row_to_tx(&inserted)))
}

/// `GET /v1/balance-transactions` - list movements (optionally by balance).
#[utoipa::path(
    get,
    path = "/v1/balance-transactions",
    params(
        ("balance_id" = Option<Uuid>, Query, description = "Filter by balance leg"),
    ),
    responses(
        (status = 200, description = "List of transactions", body = [BalanceTransaction]),
        (status = 403, description = "Missing bill8:ledger:balance-transactions:read"),
    ),
    security(("bill8:ledger:balance-transactions:read" = [])),
    tag = "ledger",
)]
pub async fn list_transactions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListTransactionsQuery>,
) -> Result<Json<Vec<BalanceTransaction>>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("bill8:ledger:balance-transactions:read"),
    )
    .await?;

    let mut conn = state.pool.acquire().await.map_err(err_db)?;
    set_tenant_context(&mut conn, a.env_id())
        .await
        .map_err(err_db)?;

    let rows = sqlx::query(
        r#"SELECT id, debit_balance_id, credit_balance_id, amount, currency,
                  transaction_type, source_type, source_id, created_at
           FROM ledger.balance_transactions
           WHERE ($1::uuid IS NULL OR debit_balance_id = $1 OR credit_balance_id = $1)
           ORDER BY created_at DESC LIMIT 200"#,
    )
    .bind(q.balance_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(err_db)?;

    Ok(Json(rows.iter().map(row_to_tx).collect()))
}

/// A balance row locked for update.
pub(crate) struct LockedBalance {
    pub(crate) id: Uuid,
    pub(crate) currency: String,
    pub(crate) amount: Decimal,
}

/// In-transaction posting - the double-entry core shared by the HTTP path
/// and internal callers (transfers, destination-charge splits). The caller
/// owns the transaction (BEGIN/tenant-context/COMMIT); this function only
/// validates and writes, so a rejection never leaves partial state.
///
/// # Errors
/// Returns `422`-class domain errors on imbalance, currency mismatch, or
/// insufficient funds - with NOTHING written on the caller's connection.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn post_transaction_in_tx(
    conn: &mut sqlx::PgConnection,
    env_id: Uuid,
    req: PostTransactionRequest,
) -> Result<BalanceTransaction, (StatusCode, String)> {
    let amount: Decimal = req.amount.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "amount must be a decimal string".into(),
        )
    })?;
    if amount <= Decimal::ZERO {
        return Err((StatusCode::BAD_REQUEST, "amount must be positive".into()));
    }

    let debit = fetch_balance_for_update(conn, req.debit_balance_id, env_id).await?;
    let credit = fetch_balance_for_update(conn, req.credit_balance_id, env_id).await?;

    if debit.currency != req.currency || credit.currency != req.currency {
        return Err(map_domain(LedgerError::CurrencyMismatch {
            balance: if debit.currency != req.currency {
                debit.id
            } else {
                credit.id
            },
            balance_currency: if debit.currency != req.currency {
                debit.currency.clone()
            } else {
                credit.currency.clone()
            },
            posting_currency: req.currency.clone(),
        }));
    }
    if debit.amount < amount {
        return Err(map_domain(LedgerError::InsufficientFunds {
            available: debit.amount.to_string(),
            requested: req.amount.clone(),
        }));
    }

    let tx_id = Uuid::new_v4();
    let inserted = sqlx::query(
        r#"INSERT INTO ledger.balance_transactions
               (id, env_id, debit_balance_id, credit_balance_id, amount, currency,
                transaction_type, source_type, source_id, idempotency_key)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING id, debit_balance_id, credit_balance_id, amount, currency,
                     transaction_type, source_type, source_id, created_at"#,
    )
    .bind(tx_id)
    .bind(env_id)
    .bind(req.debit_balance_id)
    .bind(req.credit_balance_id)
    .bind(amount)
    .bind(&req.currency)
    .bind(&req.transaction_type)
    .bind(req.source_type.as_deref())
    .bind(req.source_id)
    .bind(req.idempotency_key.as_deref())
    .fetch_one(&mut *conn)
    .await
    .map_err(err_db)?;

    sqlx::query(
        "UPDATE ledger.balances SET amount = amount - $1, updated_at = now() WHERE id = $2",
    )
    .bind(amount)
    .bind(req.debit_balance_id)
    .execute(&mut *conn)
    .await
    .map_err(err_db)?;
    sqlx::query(
        "UPDATE ledger.balances SET amount = amount + $1, updated_at = now() WHERE id = $2",
    )
    .bind(amount)
    .bind(req.credit_balance_id)
    .execute(&mut *conn)
    .await
    .map_err(err_db)?;

    sqlx::query(
        r#"INSERT INTO ledger.outbox (env_id, event_type, payload)
           VALUES ($1, 'bill8:ledger:balance_transaction:created', $2)"#,
    )
    .bind(env_id)
    .bind(serde_json::json!({
        "transaction_id": tx_id,
        "amount": req.amount,
        "currency": req.currency,
        "transaction_type": req.transaction_type,
    }))
    .execute(&mut *conn)
    .await
    .map_err(err_db)?;

    Ok(row_to_tx(&inserted))
}

/// Row-lock (`FOR UPDATE`) a balance for the posting transaction; the lock
/// serializes concurrent movements per balance (correctness over
/// throughput on the hot money path).
pub(crate) async fn fetch_balance_for_update(
    conn: &mut sqlx::PgConnection,
    balance_id: Uuid,
    env_id: Uuid,
) -> Result<LockedBalance, (StatusCode, String)> {
    let row = sqlx::query(
        "SELECT id, currency, amount FROM ledger.balances WHERE id = $1 AND env_id = $2 FOR UPDATE",
    )
    .bind(balance_id)
    .bind(env_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(err_db)?
    .ok_or_else(|| map_domain(LedgerError::BalanceNotFound(balance_id)))?;
    Ok(LockedBalance {
        id: row.get("id"),
        currency: row.get("currency"),
        amount: row.get("amount"),
    })
}

/// Map a domain error to its HTTP status. Every domain error guarantees
/// zero rows written.
pub(crate) fn map_domain(e: LedgerError) -> (StatusCode, String) {
    match e {
        LedgerError::Imbalanced { .. }
        | LedgerError::CurrencyMismatch { .. }
        | LedgerError::InsufficientFunds { .. }
        | LedgerError::IllegalTransition { .. }
        | LedgerError::BalanceNotFound(_) => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()),
        LedgerError::Db(db) => err_db(db),
    }
}

pub(crate) fn row_to_tx(row: &sqlx::postgres::PgRow) -> BalanceTransaction {
    BalanceTransaction {
        id: row.get("id"),
        debit_balance_id: row.get("debit_balance_id"),
        credit_balance_id: row.get("credit_balance_id"),
        amount: row.get::<Decimal, _>("amount").to_string(),
        currency: row.get("currency"),
        transaction_type: row.get("transaction_type"),
        source_type: row.get("source_type"),
        source_id: row.get("source_id"),
        created_at: row.get("created_at"),
    }
}
