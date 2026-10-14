// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Bill8:Ledger
// SPDX-License-Identifier: Apache-2.0

//! Auth token verification for Bill8:Ledger (steps 1–4 of the
//! authorization pipeline). Step 5 — the Shield policy gate — is the
//! authorization decision; no JWT scope check remains in handlers.
//! See `services/ledger/DESIGN.md` §5.

use axum::http::{HeaderMap, StatusCode};

use crate::config::LedgerConfig;

pub use sentin8::auth::client::AuthContext;

/// Verify the Bearer token and return the auth context (identity +
/// signature + expiry + tenant). NO scope check — step 5
/// (`crate::policy::require`) is the authorization decision.
///
/// # Errors
/// Returns `401` with a reason when the bearer token is missing, expired,
/// or fails signature/issuer verification.
pub async fn verify_token(
    config: &LedgerConfig,
    headers: &HeaderMap,
) -> Result<AuthContext, (StatusCode, String)> {
    sentin8::auth::client::verify_bearer_token(&config.sentin8_auth_client, headers).await
}
