// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Bill8:Ledger
// SPDX-License-Identifier: Apache-2.0

//! Ledger-account handlers — the internal accounts that hold balances.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use sqlx::Row;
use uuid::Uuid;

use crate::auth;
use crate::handlers::{emit_event, err_db, set_tenant_context, AppState};
use crate::types::CreateAccountRequest;

/// Account response shape.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct AccountResponse {
    pub id: Uuid,
    pub account_type: String,
    pub display_name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// `POST /v1/accounts` — create a ledger account with its standard
/// balances (`available`, `pending`).
#[utoipa::path(
    post,
    path = "/v1/accounts",
    request_body = CreateAccountRequest,
    responses(
        (status = 200, description = "Account created", body = AccountResponse),
        (status = 403, description = "Missing bill8:ledger:accounts:write"),
    ),
    security(("bill8:ledger:accounts:write" = [])),
    tag = "ledger",
)]
pub async fn create_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateAccountRequest>,
) -> Result<Json<AccountResponse>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("bill8:ledger:accounts:write"),
    ).await?;

    let account_type = match req.account_type.as_str() {
        "customer" | "platform" | "connected" => req.account_type.clone(),
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("invalid account_type {other}: must be customer|platform|connected"),
            ))
        }
    };

    let mut conn = state.pool.begin().await.map_err(err_db)?;
    set_tenant_context(&mut conn, a.env_id()).await.map_err(err_db)?;

    let id = Uuid::new_v4();
    let row = sqlx::query(
        r#"INSERT INTO ledger.accounts (id, env_id, account_type, display_name)
           VALUES ($1, $2, $3, $4)
           RETURNING id, account_type, display_name, created_at"#,
    )
    .bind(id)
    .bind(a.env_id())
    .bind(&account_type)
    .bind(&req.display_name)
    .fetch_one(&mut *conn)
    .await
    .map_err(err_db)?;

    // Standard balances for the account's default currency (USD).
    for name in ["available", "pending"] {
        sqlx::query(
            r#"INSERT INTO ledger.balances (env_id, account_id, name, currency, amount)
               VALUES ($1, $2, $3, 'USD', 0)
               ON CONFLICT (account_id, name, currency) DO NOTHING"#,
        )
        .bind(a.env_id())
        .bind(id)
        .bind(name)
        .execute(&mut *conn)
        .await
        .map_err(err_db)?;
    }

    // Outbox-first: the account:created event rides in the same tx.
    sqlx::query(
        r#"INSERT INTO ledger.outbox (env_id, event_type, payload)
           VALUES ($1, 'bill8:ledger:account:created', $2)"#,
    )
    .bind(a.env_id())
    .bind(serde_json::json!({ "account_id": id, "account_type": account_type }))
    .execute(&mut *conn)
    .await
    .map_err(err_db)?;

    conn.commit().await.map_err(err_db)?;

    let resp = AccountResponse {
        id: row.get("id"),
        account_type: row.get("account_type"),
        display_name: row.get("display_name"),
        created_at: row.get("created_at"),
    };
    Ok(Json(resp))
}

/// `GET /v1/accounts` — list ledger accounts.
#[utoipa::path(
    get,
    path = "/v1/accounts",
    responses(
        (status = 200, description = "List of accounts", body = [AccountResponse]),
        (status = 403, description = "Missing bill8:ledger:accounts:read"),
    ),
    security(("bill8:ledger:accounts:read" = [])),
    tag = "ledger",
)]
pub async fn list_accounts(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AccountResponse>>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("bill8:ledger:accounts:read"),
    ).await?;

    let mut conn = state.pool.acquire().await.map_err(err_db)?;
    set_tenant_context(&mut conn, a.env_id()).await.map_err(err_db)?;

    let rows = sqlx::query(
        "SELECT id, account_type, display_name, created_at FROM ledger.accounts ORDER BY created_at DESC LIMIT 500",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(err_db)?;

    Ok(Json(
        rows.iter()
            .map(|r| AccountResponse {
                id: r.get("id"),
                account_type: r.get("account_type"),
                display_name: r.get("display_name"),
                created_at: r.get("created_at"),
            })
            .collect(),
    ))
}

/// Emit the balance:updated event (used by the payout/transfer paths via
/// the pool helper — best-effort by design for non-posting reads).
pub async fn emit_balance_updated(pool: &sqlx::PgPool, env_id: Uuid, balance_id: Uuid) {
    emit_event(
        pool,
        env_id,
        "bill8:ledger:balance:updated",
        serde_json::json!({ "balance_id": balance_id }),
    )
    .await;
}