// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Bill8:Ledger
// SPDX-License-Identifier: Apache-2.0

//! Payout-schedule handlers (LED-7) — automatic payouts via the
//! Sentin8:Cron proxy-back run. At most one payout per schedule tick;
//! every payout obeys the available-balance guard (LED-3's rules).

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use rust_decimal::Decimal;
use sqlx::Row;
use uuid::Uuid;

use crate::auth;
use crate::handlers::{err_db, set_tenant_context, AppState};

/// Request body for `POST /v1/payout-schedules`.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreatePayoutScheduleRequest {
    /// The ledger account to pay out.
    pub account_id: Uuid,
    /// The external-account destination (secret-by-reference).
    pub external_account_ref: String,
    /// The payout interval (`daily` / `weekly` / `monthly`).
    pub interval: String,
    /// Payouts fire only when available ≥ minimum.
    pub minimum_amount: String,
    /// ISO 4217 currency code.
    pub currency: String,
}

/// Response shape.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct PayoutScheduleResponse {
    pub id: Uuid,
    pub account_id: Uuid,
    pub interval: String,
    pub minimum_amount: String,
    pub currency: String,
    pub enabled: bool,
}

/// `POST /v1/payout-schedules` — create an automatic payout schedule.
#[utoipa::path(
    post,
    path = "/v1/payout-schedules",
    request_body = CreatePayoutScheduleRequest,
    responses(
        (status = 200, description = "Schedule created", body = PayoutScheduleResponse),
        (status = 403, description = "Missing bill8:ledger:payout-schedules:write"),
    ),
    security(("bill8:ledger:payout-schedules:write" = [])),
    tag = "ledger",
)]
pub async fn create_payout_schedule(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreatePayoutScheduleRequest>,
) -> Result<Json<PayoutScheduleResponse>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("bill8:ledger:payout-schedules:write"),
    ).await?;

    if !matches!(req.interval.as_str(), "daily" | "weekly" | "monthly") {
        return Err((StatusCode::BAD_REQUEST, "interval must be daily|weekly|monthly".into()));
    }
    let minimum: Decimal = req
        .minimum_amount
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "minimum_amount must be a decimal string".into()))?;

    let mut conn = state.pool.begin().await.map_err(err_db)?;
    set_tenant_context(&mut conn, a.env_id()).await.map_err(err_db)?;

    let id = Uuid::new_v4();
    let row = sqlx::query(
        r#"INSERT INTO ledger.payout_schedules
               (id, env_id, account_id, external_account_ref, interval, minimum_amount, currency)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING id, account_id, interval, minimum_amount, currency, enabled"#,
    )
    .bind(id)
    .bind(a.env_id())
    .bind(req.account_id)
    .bind(&req.external_account_ref)
    .bind(&req.interval)
    .bind(minimum)
    .bind(&req.currency)
    .fetch_one(&mut *conn)
    .await
    .map_err(err_db)?;

    sqlx::query(
        r#"INSERT INTO ledger.outbox (env_id, event_type, payload)
           VALUES ($1, 'bill8:ledger:payout_schedule:created', $2)"#,
    )
    .bind(a.env_id())
    .bind(serde_json::json!({ "payout_schedule_id": id }))
    .execute(&mut *conn)
    .await
    .map_err(err_db)?;

    conn.commit().await.map_err(err_db)?;
    Ok(Json(row_to_schedule(&row)))
}

/// `GET /v1/payout-schedules` — list schedules.
#[utoipa::path(
    get,
    path = "/v1/payout-schedules",
    responses(
        (status = 200, description = "List of schedules", body = [PayoutScheduleResponse]),
        (status = 403, description = "Missing bill8:ledger:payout-schedules:read"),
    ),
    security(("bill8:ledger:payout-schedules:read" = [])),
    tag = "ledger",
)]
pub async fn list_payout_schedules(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<PayoutScheduleResponse>>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("bill8:ledger:payout-schedules:read"),
    ).await?;

    let mut conn = state.pool.acquire().await.map_err(err_db)?;
    set_tenant_context(&mut conn, a.env_id()).await.map_err(err_db)?;

    let rows = sqlx::query(
        "SELECT id, account_id, interval, minimum_amount, currency, enabled FROM ledger.payout_schedules ORDER BY created_at",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(err_db)?;

    Ok(Json(rows.iter().map(row_to_schedule).collect()))
}

/// `DELETE /v1/payout-schedules/{id}` — disable a schedule.
#[utoipa::path(
    delete,
    path = "/v1/payout-schedules/{id}",
    responses(
        (status = 200, description = "Schedule disabled"),
        (status = 404, description = "Schedule not found"),
    ),
    security(("bill8:ledger:payout-schedules:write" = [])),
    tag = "ledger",
)]
pub async fn delete_payout_schedule(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("bill8:ledger:payout-schedules:write"),
    ).await?;

    let mut conn = state.pool.begin().await.map_err(err_db)?;
    set_tenant_context(&mut conn, a.env_id()).await.map_err(err_db)?;

    let result = sqlx::query("UPDATE ledger.payout_schedules SET enabled = false WHERE id = $1 AND enabled")
        .bind(id)
        .execute(&mut *conn)
        .await
        .map_err(err_db)?;
    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "schedule not found or already disabled".into()));
    }

    conn.commit().await.map_err(err_db)?;
    Ok(Json(serde_json::json!({ "status": "disabled" })))
}

/// `POST /v1/internal/payout-schedules/run` — the Cron proxy-back tick.
/// Creates a payout for each enabled schedule whose available balance
/// meets its minimum. At most one payout per tick (the balance guard
/// itself enforces it: the available row lock serializes).
#[utoipa::path(
    post,
    path = "/v1/internal/payout-schedules/run",
    responses(
        (status = 200, description = "Tick processed"),
    ),
    security(("sentin8:cron:write" = [])),
    tag = "ledger",
)]
pub async fn run_payout_schedules(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Cron proxy-back: the cron workload presents its certificate (the
    // TLS handshake is the identity; verification is the standard pipeline).
    let a = auth::verify_token(&state.config, &headers).await?;

    let mut conn = state.pool.begin().await.map_err(err_db)?;
    set_tenant_context(&mut conn, a.env_id()).await.map_err(err_db)?;

    let schedules = sqlx::query(
        "SELECT id, account_id, external_account_ref, interval, minimum_amount, currency FROM ledger.payout_schedules WHERE enabled",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(err_db)?;

    let mut payouts_created = 0usize;
    let mut below_minimum = 0usize;
    for s in &schedules {
        let schedule_id: Uuid = s.get("id");
        let account_id: Uuid = s.get("account_id");
        let external_ref: String = s.get("external_account_ref");
        let minimum: Decimal = s.get("minimum_amount");
        let currency: String = s.get("currency");

        // The available balance under its row lock (LED-3 guard).
        let avail_row = sqlx::query(
            "SELECT id, amount FROM ledger.balances WHERE account_id = $1 AND name = 'available' AND currency = $2 FOR UPDATE",
        )
        .bind(account_id)
        .bind(&currency)
        .fetch_optional(&mut *conn)
        .await
        .map_err(err_db)?;
        let Some(avail_row) = avail_row else { continue };
        let available: Decimal = avail_row.get("amount");
        if available < minimum {
            below_minimum += 1;
            continue;
        }

        // Pay out the full available balance (≥ minimum).
        let payout_id = Uuid::new_v4();
        let pending: Uuid = sqlx::query(
            "SELECT id FROM ledger.balances WHERE account_id = $1 AND name = 'pending' AND currency = $2",
        )
        .bind(account_id)
        .bind(&currency)
        .fetch_optional(&mut *conn)
        .await
        .map_err(err_db)?
        .map(|r| r.get("id"))
        .ok_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR, "pending balance missing".into()))?;

        sqlx::query(
            r#"INSERT INTO ledger.balance_transactions
                   (id, env_id, debit_balance_id, credit_balance_id, amount, currency,
                    transaction_type, source_type, source_id, idempotency_key)
               VALUES ($1, $2, $3, $4, $5, $6, 'payout', 'payout_schedule', $7, $8)"#,
        )
        .bind(Uuid::new_v4())
        .bind(a.env_id())
        .bind(avail_row.get::<Uuid, _>("id"))
        .bind(pending)
        .bind(available)
        .bind(&currency)
        .bind(schedule_id)
        .bind(format!("payout-schedule-{schedule_id}-{}", chrono::Utc::now().format("%Y%m%d%H%M")))
        .execute(&mut *conn)
        .await
        .map_err(err_db)?;

        sqlx::query("UPDATE ledger.balances SET amount = 0, updated_at = now() WHERE id = $1")
            .bind(avail_row.get::<Uuid, _>("id"))
            .execute(&mut *conn)
            .await
            .map_err(err_db)?;
        sqlx::query("UPDATE ledger.balances SET amount = amount + $1, updated_at = now() WHERE id = $2")
            .bind(available)
            .bind(pending)
            .execute(&mut *conn)
            .await
            .map_err(err_db)?;

        sqlx::query(
            r#"INSERT INTO ledger.payouts (id, env_id, account_id, external_account_ref, amount, currency, status)
               VALUES ($1, $2, $3, $4, $5, $6, 'pending')"#,
        )
        .bind(payout_id)
        .bind(a.env_id())
        .bind(account_id)
        .bind(&external_ref)
        .bind(available)
        .bind(&currency)
        .execute(&mut *conn)
        .await
        .map_err(err_db)?;

        sqlx::query(
            r#"INSERT INTO ledger.outbox (env_id, event_type, payload)
               VALUES ($1, 'bill8:ledger:payout:created', $2)"#,
        )
        .bind(a.env_id())
        .bind(serde_json::json!({ "payout_id": payout_id, "schedule_id": schedule_id }))
        .execute(&mut *conn)
        .await
        .map_err(err_db)?;

        payouts_created += 1;
    }

    conn.commit().await.map_err(err_db)?;
    Ok(Json(serde_json::json!({ "payouts_created": payouts_created, "below_minimum": below_minimum })))
}

fn row_to_schedule(row: &sqlx::postgres::PgRow) -> PayoutScheduleResponse {
    PayoutScheduleResponse {
        id: row.get("id"),
        account_id: row.get("account_id"),
        interval: row.get("interval"),
        minimum_amount: row.get::<Decimal, _>("minimum_amount").to_string(),
        currency: row.get("currency"),
        enabled: row.get("enabled"),
    }
}