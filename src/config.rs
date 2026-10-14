// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Bill8:Ledger
// SPDX-License-Identifier: Apache-2.0

//! Ledger configuration loaded from environment variables.
//! See `services/ledger/DESIGN.md` §5 (cross-cutting concerns).

use std::env;

use sentin8::auth::client::{Client as AuthClient, ClientOptions};

/// Ledger configuration.
#[derive(Clone)]
pub struct LedgerConfig {
    /// Service port (assigned by Sentin8:Captain).
    pub port: u16,
    /// PostgreSQL DSN (role `bill8_ledger_service`, db `bill8`).
    pub database_url: String,
    /// Auth issuer base URL.
    pub sentin8_auth_issuer: String,
    /// Auth JWKS URL.
    pub sentin8_auth_jwks_url: String,
    /// Bus base URL (outbox relay target).
    pub sentin8_bus_url: String,
    /// Stream family this service produces to.
    pub stream_family: String,
    /// Shared Auth client (discovery + JWKS caches reused across
    /// requests; cloning is cheap — caches live behind an `Arc`).
    pub sentin8_auth_client: AuthClient,
    /// Certificate-presenting transport for the service's own workload
    /// identity (CERTFREE-4V — no M2M token exists). Set at startup by
    /// `server::build_state`; `None` until then.
    pub m2m_http: Option<reqwest::Client>,
}

impl std::fmt::Debug for LedgerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LedgerConfig")
            .field("port", &self.port)
            .field("database_url", &"<redacted>")
            .field("sentin8_auth_issuer", &self.sentin8_auth_issuer)
            .field("stream_family", &self.stream_family)
            .field("m2m_http", &self.m2m_http.is_some())
            .finish_non_exhaustive()
    }
}

impl LedgerConfig {
    /// Load configuration from environment variables.
    ///
    /// # Panics
    /// Panics when `BILL8_LEDGER_PORT` is unset (the captain always
    /// sets it) or not a valid `u16`.
    pub fn from_env() -> Self {
        let cfg = sentin8::config::Sentin8Config::from_env();
        Self {
            port: env::var("BILL8_LEDGER_PORT")
                .expect("BILL8_LEDGER_PORT must be set by captain")
                .parse()
                .expect("BILL8_LEDGER_PORT must be a valid u16"),
            database_url: env::var("BILL8_LEDGER_DATABASE_URL")
                .or_else(|_| env::var("DATABASE_URL"))
                .unwrap_or_else(|_| sentin8::config::Sentin8Config::default_database_url().to_string()),
            sentin8_auth_issuer: env::var("SENTIN8_AUTH_ISSUER")
                .unwrap_or_else(|_| cfg.sentin8_auth_issuer()),
            sentin8_auth_jwks_url: env::var("SENTIN8_AUTH_JWKS_URL")
                .unwrap_or_else(|_| cfg.sentin8_auth_jwks_url()),
            sentin8_bus_url: env::var("SENTIN8_BUS_URL")
                .unwrap_or_else(|_| cfg.sentin8_bus_url()),
            stream_family: "ledger".into(),
            sentin8_auth_client: AuthClient::new(ClientOptions {
                base_url: env::var("SENTIN8_AUTH_ISSUER")
                    .unwrap_or_else(|_| cfg.sentin8_auth_issuer()),
                ..Default::default()
            }),
            m2m_http: None,
        }
    }
}