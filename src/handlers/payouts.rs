// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Bill8:Ledger
// SPDX-License-Identifier: Apache-2.0

//! Payout handlers — withdrawals to external accounts.
//!
//! State machine (ledger DESIGN.md §3): `pending → paid | failed`;
//! terminal states never regress. A payout never exceeds the available
//! balance at post time (row-lock serialized).

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use bill8::ledger::contracts::{CreatePayoutRequest, Payout};
use rust_decimal::Decimal;
use sqlx::Row;
use uuid::Uuid;

use crate::auth;
use crate::handlers::transactions::{fetch_balance_for_update, map_domain};
use crate::handlers::{err_db, set_tenant_context, AppState};
use crate::types::LedgerError;

/// `POST /v1/payouts` — pay out available balance to an external account.
#[utoipa::path(
    post,
    path = "/v1/payouts",
    request_body = CreatePayoutRequest,
    responses(
        (status = 200, description = "Payout created (pending)", body = Payout),
        (status = 403, description = "Missing bill8:ledger:payouts:write"),
        (status = 422, description = "Insufficient available balance — nothing written"),
    ),
    security(("bill8:ledger:payouts:write" = [])),
    tag = "ledger",
)]
pub async fn create_payout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreatePayoutRequest>,
) -> Result<Json<Payout>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("bill8:ledger:payouts:write"),
    ).await?;

    let amount: Decimal = req
        .amount
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "amount must be a decimal string".into()))?;
    if amount <= Decimal::ZERO {
        return Err((StatusCode::BAD_REQUEST, "amount must be positive".into()));
    }

    let mut conn = state.pool.begin().await.map_err(err_db)?;
    set_tenant_context(&mut conn, a.env_id()).await.map_err(err_db)?;

    // Idempotency: a retried payout returns the original.
    if let Some(key) = req.idempotency_key.as_deref() {
        if let Ok(Some(row)) = sqlx::query(
            "SELECT id FROM ledger.payouts WHERE env_id = $1 AND idempotency_key = $2",
        )
        .bind(a.env_id())
        .bind(key)
        .fetch_optional(&mut *conn)
        .await
        {
            let id: Uuid = row.get("id");
            conn.commit().await.map_err(err_db)?;
            return get_payout_row(&state, a.env_id(), id).await;
        }
    }

    // Lock the account's available balance; verify funds.
    let avail = sqlx::query(
        "SELECT id FROM ledger.balances WHERE account_id = $1 AND name = 'available' AND currency = $2",
    )
    .bind(req.account_id)
    .bind(&req.currency)
    .fetch_optional(&mut *conn)
    .await
    .map_err(err_db)?
    .map(|r| r.get::<Uuid, _>("id"))
    .ok_or_else(|| (StatusCode::NOT_FOUND, "available balance not found".into()))?;

    let locked = fetch_balance_for_update(&mut conn, avail, a.env_id()).await?;
    if locked.amount < amount {
        return Err(map_domain(LedgerError::InsufficientFunds {
            available: locked.amount.to_string(),
            requested: req.amount.clone(),
        }));
    }

    // Payout record + holding movement (available → pending).
    let payout_id = Uuid::new_v4();
    let tx_id = Uuid::new_v4();
    let pending = sqlx::query(
        "SELECT id FROM ledger.balances WHERE account_id = $1 AND name = 'pending' AND currency = $2",
    )
    .bind(req.account_id)
    .bind(&req.currency)
    .fetch_optional(&mut *conn)
    .await
    .map_err(err_db)?
    .map(|r| r.get::<Uuid, _>("id"))
    .ok_or_else(|| (StatusCode::NOT_FOUND, "pending balance not found".into()))?;

    sqlx::query(
        r#"INSERT INTO ledger.balance_transactions
               (id, env_id, debit_balance_id, credit_balance_id, amount, currency,
                transaction_type, source_type, source_id, idempotency_key)
           VALUES ($1, $2, $3, $4, $5, $6, 'payout', 'payout', $7, $8)"#,
    )
    .bind(tx_id)
    .bind(a.env_id())
    .bind(avail)
    .bind(pending)
    .bind(amount)
    .bind(&req.currency)
    .bind(payout_id)
    .bind(req.idempotency_key.as_deref())
    .execute(&mut *conn)
    .await
    .map_err(err_db)?;

    sqlx::query("UPDATE ledger.balances SET amount = amount - $1, updated_at = now() WHERE id = $2")
        .bind(amount)
        .bind(avail)
        .execute(&mut *conn)
        .await
        .map_err(err_db)?;
    sqlx::query("UPDATE ledger.balances SET amount = amount + $1, updated_at = now() WHERE id = $2")
        .bind(amount)
        .bind(pending)
        .execute(&mut *conn)
        .await
        .map_err(err_db)?;

    let row = sqlx::query(
        r#"INSERT INTO ledger.payouts
               (id, env_id, account_id, external_account_ref, amount, currency,
                status, balance_transaction_id, idempotency_key)
           VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7, $8)
           RETURNING id, env_id, account_id, external_account_ref, amount, currency, status, created_at"#,
    )
    .bind(payout_id)
    .bind(a.env_id())
    .bind(req.account_id)
    .bind(&req.external_account_ref)
    .bind(amount)
    .bind(&req.currency)
    .bind(tx_id)
    .bind(req.idempotency_key.as_deref())
    .fetch_one(&mut *conn)
    .await
    .map_err(err_db)?;

    sqlx::query(
        r#"INSERT INTO ledger.outbox (env_id, event_type, payload)
           VALUES ($1, 'bill8:ledger:payout:created', $2)"#,
    )
    .bind(a.env_id())
    .bind(serde_json::json!({ "payout_id": payout_id, "amount": req.amount, "currency": req.currency }))
    .execute(&mut *conn)
    .await
    .map_err(err_db)?;

    conn.commit().await.map_err(err_db)?;
    Ok(Json(row_to_payout(&row)))
}

/// `GET /v1/payouts` — list payouts.
#[utoipa::path(
    get,
    path = "/v1/payouts",
    responses(
        (status = 200, description = "List of payouts", body = [Payout]),
        (status = 403, description = "Missing bill8:ledger:payouts:read"),
    ),
    security(("bill8:ledger:payouts:read" = [])),
    tag = "ledger",
)]
pub async fn list_payouts(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Payout>>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("bill8:ledger:payouts:read"),
    ).await?;

    let mut conn = state.pool.acquire().await.map_err(err_db)?;
    set_tenant_context(&mut conn, a.env_id()).await.map_err(err_db)?;

    let rows = sqlx::query(
        r#"SELECT id, env_id, account_id, external_account_ref, amount, currency, status, created_at
           FROM ledger.payouts ORDER BY created_at DESC LIMIT 200"#,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(err_db)?;

    Ok(Json(rows.iter().map(row_to_payout).collect()))
}

/// `GET /v1/payouts/{id}` — read one payout.
#[utoipa::path(
    get,
    path = "/v1/payouts/{id}",
    responses(
        (status = 200, description = "The payout", body = Payout),
        (status = 404, description = "Payout not found"),
    ),
    security(("bill8:ledger:payouts:read" = [])),
    tag = "ledger",
)]
pub async fn get_payout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Payout>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("bill8:ledger:payouts:read"),
    ).await?;
    get_payout_row(&state, a.env_id(), id).await
}

async fn get_payout_row(
    state: &AppState,
    env_id: Uuid,
    id: Uuid,
) -> Result<Json<Payout>, (StatusCode, String)> {
    let mut conn = state.pool.acquire().await.map_err(err_db)?;
    set_tenant_context(&mut conn, env_id).await.map_err(err_db)?;
    let row = sqlx::query(
        "SELECT id, env_id, account_id, external_account_ref, amount, currency, status, created_at FROM ledger.payouts WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(err_db)?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "payout not found".into()))?;
    Ok(Json(row_to_payout(&row)))
}

fn row_to_payout(row: &sqlx::postgres::PgRow) -> Payout {
    Payout {
        id: row.get("id"),
        env_id: row.get("env_id"),
        account_id: row.get("account_id"),
        external_account_ref: row.get("external_account_ref"),
        amount: row.get::<Decimal, _>("amount").to_string(),
        currency: row.get("currency"),
        status: row.get("status"),
        created_at: row.get("created_at"),
    }
}