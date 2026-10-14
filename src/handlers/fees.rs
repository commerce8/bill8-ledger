// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Bill8:Ledger
// SPDX-License-Identifier: Apache-2.0

//! Fee-model handlers — the explicit platform-fee surface.
//!
//! Fees are DATA: the fee charged on a charge is captured as an explicit
//! balance transaction crediting the `platform_fee` balance — never
//! implicit, never folded into a charge amount. See ledger DESIGN.md §3.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use rust_decimal::Decimal;
use sqlx::Row;

use crate::auth;
use crate::handlers::{err_db, set_tenant_context, AppState};

/// Fee-model response.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct FeeResponse {
    pub id: uuid::Uuid,
    pub transaction_type: String,
    pub fixed_amount: String,
    pub bps: i32,
    pub currency: String,
}

/// `GET /v1/fees` — list the configured fee model.
#[utoipa::path(
    get,
    path = "/v1/fees",
    responses(
        (status = 200, description = "List of fee entries", body = [FeeResponse]),
        (status = 403, description = "Missing bill8:ledger:fees:read"),
    ),
    security(("bill8:ledger:fees:read" = [])),
    tag = "ledger",
)]
pub async fn list_fees(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<FeeResponse>>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("bill8:ledger:fees:read"),
    ).await?;

    let mut conn = state.pool.acquire().await.map_err(err_db)?;
    set_tenant_context(&mut conn, a.env_id()).await.map_err(err_db)?;

    let rows = sqlx::query(
        "SELECT id, transaction_type, fixed_amount, bps, currency FROM ledger.fees ORDER BY transaction_type",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(err_db)?;

    Ok(Json(
        rows.iter()
            .map(|r| FeeResponse {
                id: r.get("id"),
                transaction_type: r.get("transaction_type"),
                fixed_amount: r.get::<Decimal, _>("fixed_amount").to_string(),
                bps: r.get("bps"),
                currency: r.get("currency"),
            })
            .collect(),
    ))
}