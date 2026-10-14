// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Bill8:Ledger
// SPDX-License-Identifier: Apache-2.0

//! Transfer handlers — internal movements between connected balances,
//! and fee-model configuration. Both post through the double-entry
//! engine; a transfer or fee never bypasses it.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use bill8::ledger::contracts::Transfer;
use rust_decimal::Decimal;
use sqlx::Row;
use uuid::Uuid;

use crate::auth;
use crate::handlers::{emit_event, err_db, set_tenant_context, AppState};

/// Request body for `POST /v1/transfers`.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateTransferRequest {
    pub from_balance_id: Uuid,
    pub to_balance_id: Uuid,
    pub amount: String,
    pub currency: String,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

/// Fee-model request body.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct UpsertFeeRequest {
    pub transaction_type: String,
    pub fixed_amount: String,
    pub bps: i32,
    pub currency: String,
}

/// `POST /v1/transfers` — move funds between connected balances.
#[utoipa::path(
    post,
    path = "/v1/transfers",
    request_body = CreateTransferRequest,
    responses(
        (status = 200, description = "Transfer created", body = Transfer),
        (status = 403, description = "Missing bill8:ledger:transfers:write"),
        (status = 422, description = "Insufficient funds — nothing written"),
    ),
    security(("bill8:ledger:transfers:write" = [])),
    tag = "ledger",
)]
pub async fn create_transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateTransferRequest>,
) -> Result<Json<Transfer>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("bill8:ledger:transfers:write"),
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

    // Delegate the movement to the double-entry engine (row-locked legs).
    crate::handlers::transactions::post_transaction_in_tx(
        &mut conn,
        a.env_id(),
        bill8::ledger::contracts::PostTransactionRequest {
            debit_balance_id: req.from_balance_id,
            credit_balance_id: req.to_balance_id,
            amount: req.amount.clone(),
            currency: req.currency.clone(),
            transaction_type: "transfer".into(),
            source_type: Some("transfer".into()),
            source_id: None,
            idempotency_key: req.idempotency_key.clone(),
        },
    )
    .await?;

    let id = Uuid::new_v4();
    let row = sqlx::query(
        r#"INSERT INTO ledger.transfers (id, env_id, from_balance_id, to_balance_id, amount, currency, idempotency_key)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING id, from_balance_id, to_balance_id, amount, currency, created_at"#,
    )
    .bind(id)
    .bind(a.env_id())
    .bind(req.from_balance_id)
    .bind(req.to_balance_id)
    .bind(amount)
    .bind(&req.currency)
    .bind(req.idempotency_key.as_deref())
    .fetch_one(&mut *conn)
    .await
    .map_err(err_db)?;

    sqlx::query(
        r#"INSERT INTO ledger.outbox (env_id, event_type, payload)
           VALUES ($1, 'bill8:ledger:transfer:created', $2)"#,
    )
    .bind(a.env_id())
    .bind(serde_json::json!({ "transfer_id": id, "amount": req.amount }))
    .execute(&mut *conn)
    .await
    .map_err(err_db)?;

    conn.commit().await.map_err(err_db)?;

    let transfer = Transfer {
        id: row.get("id"),
        from_balance_id: row.get("from_balance_id"),
        to_balance_id: row.get("to_balance_id"),
        amount: row.get::<Decimal, _>("amount").to_string(),
        currency: row.get("currency"),
        created_at: row.get("created_at"),
    };
    Ok(Json(transfer))
}

/// `GET /v1/transfers` — list transfers.
#[utoipa::path(
    get,
    path = "/v1/transfers",
    responses(
        (status = 200, description = "List of transfers", body = [Transfer]),
        (status = 403, description = "Missing bill8:ledger:transfers:read"),
    ),
    security(("bill8:ledger:transfers:read" = [])),
    tag = "ledger",
)]
pub async fn list_transfers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Transfer>>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("bill8:ledger:transfers:read"),
    )
    .await?;

    let mut conn = state.pool.acquire().await.map_err(err_db)?;
    set_tenant_context(&mut conn, a.env_id())
        .await
        .map_err(err_db)?;

    let rows = sqlx::query(
        "SELECT id, from_balance_id, to_balance_id, amount, currency, created_at FROM ledger.transfers ORDER BY created_at DESC LIMIT 200",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(err_db)?;

    Ok(Json(
        rows.iter()
            .map(|r| Transfer {
                id: r.get("id"),
                from_balance_id: r.get("from_balance_id"),
                to_balance_id: r.get("to_balance_id"),
                amount: r.get::<Decimal, _>("amount").to_string(),
                currency: r.get("currency"),
                created_at: r.get("created_at"),
            })
            .collect(),
    ))
}

/// `POST /v1/fees` — upsert the fee model for a transaction type.
#[utoipa::path(
    post,
    path = "/v1/fees",
    request_body = UpsertFeeRequest,
    responses(
        (status = 200, description = "Fee model saved"),
        (status = 403, description = "Missing bill8:ledger:fees:write"),
    ),
    security(("bill8:ledger:fees:write" = [])),
    tag = "ledger",
)]
pub async fn upsert_fee(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpsertFeeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("bill8:ledger:fees:write"),
    )
    .await?;

    let fixed: Decimal = req.fixed_amount.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "fixed_amount must be a decimal string".into(),
        )
    })?;

    let mut conn = state.pool.begin().await.map_err(err_db)?;
    set_tenant_context(&mut conn, a.env_id())
        .await
        .map_err(err_db)?;

    sqlx::query(
        r#"INSERT INTO ledger.fees (env_id, transaction_type, fixed_amount, bps, currency)
           VALUES ($1, $2, $3, $4, $5)
           ON CONFLICT (env_id, transaction_type, currency)
           DO UPDATE SET fixed_amount = EXCLUDED.fixed_amount, bps = EXCLUDED.bps, updated_at = now()"#,
    )
    .bind(a.env_id())
    .bind(&req.transaction_type)
    .bind(fixed)
    .bind(req.bps)
    .bind(&req.currency)
    .execute(&mut *conn)
    .await
    .map_err(err_db)?;

    sqlx::query(
        r#"INSERT INTO ledger.outbox (env_id, event_type, payload)
           VALUES ($1, 'bill8:ledger:fee:updated', $2)"#,
    )
    .bind(a.env_id())
    .bind(serde_json::json!({ "transaction_type": req.transaction_type, "bps": req.bps }))
    .execute(&mut *conn)
    .await
    .map_err(err_db)?;

    conn.commit().await.map_err(err_db)?;
    emit_event(
        &state.pool,
        a.env_id(),
        "bill8:ledger:fee:updated",
        serde_json::json!({ "transaction_type": req.transaction_type }),
    )
    .await;
    Ok(Json(serde_json::json!({ "status": "saved" })))
}
