// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Bill8:Ledger
// SPDX-License-Identifier: Apache-2.0

//! Bill8:Ledger binary — boots on the platform `sentin8::shell`.
//! See `services/ledger/DESIGN.md` for the detailed design.

// arenas fragment under the many-worker tokio runtime and never return
#[allow(dead_code)]
mod auth;
#[allow(dead_code)]
mod config;
#[allow(dead_code)]
mod handlers;
#[allow(dead_code)]
mod openapi;
mod policy;
mod registration;
#[allow(dead_code)]
mod server;
#[allow(dead_code)]
mod types;

use utoipa::OpenApi;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    sentin8::shell::serve(
        sentin8::shell::Sentin8Shell::new("bill8:ledger", "BILL8_LEDGER")
            .with_registration(registration::print)
            .with_state(server::build_state)
            .with_own_health()
            .with_router(|router, _state| server::routes(router))
            .with_openapi(openapi::LedgerOpenApi::openapi()),
    )
    .await
}
