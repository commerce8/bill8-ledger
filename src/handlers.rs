// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Bill8:Ledger
// SPDX-License-Identifier: Apache-2.0

//! Ledger HTTP handlers — the double-entry money engine.
//!
//! **Same-transaction discipline** (ledger DESIGN.md §2): every mutation
//! runs BEGIN → `set_tenant_context(env)` → validation → row-locked
//! balance mutation → append transaction → outbox insert → COMMIT, all
//! on ONE connection. The outbox row is written in the same transaction
//! as the domain change (outbox-first, architecture-principles §6.7).

pub mod accounts;
pub mod balances;
pub mod fees;
pub mod payout_schedules;
pub mod payouts;
pub mod transactions;
pub mod transfers;

use std::sync::Arc;

use crate::config::LedgerConfig;
use crate::policy::Engine;

/// Shared application state bound into the shell.
#[derive(Clone)]
pub struct AppState {
    /// Service configuration.
    pub config: Arc<LedgerConfig>,
    /// Capped PostgreSQL pool (platform `pg_pool::connect`, max 3).
    pub pool: sqlx::PgPool,
    /// Cert-presenting Bus client.
    pub bus_client: sentin8::bus::client::BusClient,
    /// Shield policy engine — step 5 is the ONLY gate.
    pub policy: Arc<Engine>,
}

/// Set the transaction-local RLS tenant context on a connection.
///
/// MUST be `.await`ed and called on the SAME connection as the subsequent
/// data query — `sentin8_set_tenant_context` uses `set_config(..., true)`
/// which is transaction-local.
pub async fn set_tenant_context(
    conn: &mut sqlx::PgConnection,
    env_id: uuid::Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT sentin8_set_tenant_context($1, NULL, NULL, NULL, false, false, false, NULL, 'ledger')")
        .bind(env_id)
        .execute(conn)
        .await
        .map(|_| ())
}

/// Standard DB error → HTTP 500.
pub fn err_db(e: sqlx::Error) -> (axum::http::StatusCode, String) {
    (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

/// Emit a domain event to the service's LOCAL outbox (DBX producer path,
/// architecture-principles §6.7). Inserts a `ledger.outbox` row that the
/// shell's post-commit relay drains to Bus `POST /v1/produce`
/// (idempotency_key = row id). Best-effort: errors are logged, not
/// propagated — the in-transaction variant lives on the same `conn`
/// for atomic paths.
pub async fn emit_event(
    pool: &sqlx::PgPool,
    env_id: uuid::Uuid,
    event_type: &str,
    payload: serde_json::Value,
) {
    if let Err(e) =
        sentin8::bus::outbox::insert_outbox_row_pool(pool, env_id, "ledger", event_type, &payload)
            .await
    {
        tracing::warn!(error = %e, event_type, "bill8:ledger: failed to emit outbox event");
    }
}

#[utoipa::path(
    get,
    path = "/v1/health",
    responses(
        (status = 200, description = "Liveness (version, build)", body = serde_json::Value),
    ),
    security(),
    tag = "meta",
)]
/// Health probe payload.
pub async fn health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "service": "ledger",
        "version": sentin8::build_info::id(), "build": sentin8::build_info::build_hash() }))
}
