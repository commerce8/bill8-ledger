// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Bill8:Ledger
// SPDX-License-Identifier: Apache-2.0

//! Axum server setup for Bill8:Ledger — state binding + routes.

use axum::routing::get;
use std::sync::Arc;

use crate::handlers;

/// Build a cert-presenting HTTP transport for `workload` (CERTFREE-4V,
/// auth DESIGN §4.3f): mint-or-load the Acme-issued pair and wrap it
/// in a reqwest client. The receiving service's TLS layer authenticates
/// the handshake and the enroll-published registry map authorizes the
/// scopes — no bearer token exists on this path.
///
/// # Errors
/// Returns the certificate-ensure or transport-build failure (the
/// caller logs and degrades).
async fn build_workload_http(
    auth_issuer: &str,
    workload: &str,
) -> anyhow::Result<reqwest::Client> {
    let cert = sentin8::auth::m2m::ensure_workload_certificate(
        auth_issuer,
        workload,
        &sentin8::auth::m2m::platform_ca_path(),
    )
    .await?;
    let mtls = sentin8::middleware::mtls::MtlsConfig::new(
        &cert.ca_cert_path,
        &cert.cert_path,
        &cert.key_path,
    );
    sentin8::middleware::mtls::build_mtls_client(&mtls)
        .map_err(|e| anyhow::anyhow!("workload '{workload}' transport: {e}"))
}

/// Bind the service state onto the shell-provided core: config, capped
/// pool, the shell-booted Shield engine (step 8), and the cert-presenting
/// Bus client. See `services/ledger/DESIGN.md` §2 (runtime architecture).
pub async fn build_state(
    core: sentin8::shell::ServiceCore,
) -> anyhow::Result<handlers::AppState> {
    let mut config = crate::config::LedgerConfig::from_env();

    // Cert-presenting Bus client (CERTFREE-4V): the `ledger` workload
    // CERTIFICATE is the identity — no DCR, no token (auth DESIGN
    // §4.3f); the mint retries while the fleet boots.
    let bus_client = loop {
        match sentin8::bus::client::BusClient::for_workload(
            &config.sentin8_bus_url,
            &config.sentin8_auth_issuer,
            "ledger",
        )
        .await
        {
            Ok(c) => break c,
            Err(e) => {
                tracing::debug!(error = %e, "ledger bus client: certificate not ready yet");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    };

    // The service's own workload transport (CERTFREE-4V): the
    // `ledger` certificate presents in the TLS handshake — the
    // receiving service's TLS layer authenticates it and the registry
    // map authorizes the scopes.
    match build_workload_http(&config.sentin8_auth_issuer, "ledger").await {
        Ok(http) => config.m2m_http = Some(http),
        Err(e) => tracing::warn!(error = %e, "ledger workload transport unavailable — degraded"),
    }

    // The Shield engine is shell-booted (step 8); copy it into state —
    // no second engine or consumer here (policy design §4.2b).
    let policy_engine = core.admin.policy.clone();

    Ok(handlers::AppState {
        config: Arc::new(config),
        pool: core.pool.clone(),
        bus_client,
        policy: policy_engine,
    })
}

/// Mount the ledger's routes onto the shell router (state-typed — the
/// shell binds `AppState` when serving).
pub fn routes(router: axum::Router<handlers::AppState>) -> axum::Router<handlers::AppState> {
    router
        .route("/v1/health", get(handlers::health))
        .route("/v1/accounts", get(handlers::accounts::list_accounts).post(handlers::accounts::create_account))
        .route("/v1/balances", get(handlers::balances::list_balances))
        .route("/v1/balances/{id}", get(handlers::balances::get_balance))
        .route(
            "/v1/balance-transactions",
            get(handlers::transactions::list_transactions).post(handlers::transactions::post_transaction),
        )
        .route(
            "/v1/payouts",
            get(handlers::payouts::list_payouts).post(handlers::payouts::create_payout),
        )
        .route("/v1/payouts/{id}", get(handlers::payouts::get_payout))
        .route(
            "/v1/transfers",
            get(handlers::transfers::list_transfers).post(handlers::transfers::create_transfer),
        )
        .route("/v1/fees", get(handlers::fees::list_fees).post(handlers::transfers::upsert_fee))
        // Payout schedules (LED-7) + the Cron proxy-back tick.
        .route(
            "/v1/payout-schedules",
            get(handlers::payout_schedules::list_payout_schedules)
                .post(handlers::payout_schedules::create_payout_schedule),
        )
        .route("/v1/payout-schedules/{id}", axum::routing::delete(handlers::payout_schedules::delete_payout_schedule))
        .route("/v1/internal/payout-schedules/run", axum::routing::post(handlers::payout_schedules::run_payout_schedules))
}