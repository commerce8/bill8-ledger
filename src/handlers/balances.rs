// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Bill8:Ledger
// SPDX-License-Identifier: Apache-2.0

//! Balance handlers — named per-currency money positions.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use bill8::ledger::contracts::Balance;
use rust_decimal::Decimal;
use sqlx::Row;
use uuid::Uuid;

use crate::auth;
use crate::handlers::{err_db, set_tenant_context, AppState};
use crate::types::ListBalancesQuery;

/// `GET /v1/balances` — list balances (optionally filtered).
#[utoipa::path(
    get,
    path = "/v1/balances",
    params(
        ("account_id" = Option<Uuid>, Query, description = "Filter by account"),
        ("name" = Option<String>, Query, description = "Filter by balance name"),
    ),
    responses(
        (status = 200, description = "List of balances", body = [Balance]),
        (status = 403, description = "Missing bill8:ledger:balances:read"),
    ),
    security(("bill8:ledger:balances:read" = [])),
    tag = "ledger",
)]
pub async fn list_balances(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListBalancesQuery>,
) -> Result<Json<Vec<Balance>>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("bill8:ledger:balances:read"),
    ).await?;

    let mut conn = state.pool.acquire().await.map_err(err_db)?;
    set_tenant_context(&mut conn, a.env_id()).await.map_err(err_db)?;

    let rows = sqlx::query(
        r#"SELECT id, account_id, name, currency, amount, updated_at
           FROM ledger.balances
           WHERE ($1::uuid IS NULL OR account_id = $1)
             AND ($2::text IS NULL OR name = $2)
           ORDER BY account_id, name LIMIT 500"#,
    )
    .bind(q.account_id)
    .bind(q.name.as_deref())
    .fetch_all(&mut *conn)
    .await
    .map_err(err_db)?;

    Ok(Json(
        rows.iter()
            .map(|r| Balance {
                id: r.get("id"),
                account_id: r.get("account_id"),
                name: r.get("name"),
                currency: r.get("currency"),
                amount: r.get::<Decimal, _>("amount").to_string(),
                updated_at: r.get("updated_at"),
            })
            .collect(),
    ))
}

/// `GET /v1/balances/{id}` — read one balance.
#[utoipa::path(
    get,
    path = "/v1/balances/{id}",
    responses(
        (status = 200, description = "The balance", body = Balance),
        (status = 404, description = "Balance not found"),
    ),
    security(("bill8:ledger:balances:read" = [])),
    tag = "ledger",
)]
pub async fn get_balance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Balance>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("bill8:ledger:balances:read"),
    ).await?;

    let mut conn = state.pool.acquire().await.map_err(err_db)?;
    set_tenant_context(&mut conn, a.env_id()).await.map_err(err_db)?;

    let row = sqlx::query(
        "SELECT id, account_id, name, currency, amount, updated_at FROM ledger.balances WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(err_db)?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "balance not found".into()))?;

    Ok(Json(Balance {
        id: row.get("id"),
        account_id: row.get("account_id"),
        name: row.get("name"),
        currency: row.get("currency"),
        amount: row.get::<Decimal, _>("amount").to_string(),
        updated_at: row.get("updated_at"),
    }))
}