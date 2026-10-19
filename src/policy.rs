// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Bill8:Ledger
// SPDX-License-Identifier: Apache-2.0

//! Shield policy integration for Bill8:Ledger - a thin re-export of
//! the shared bridge (`sentin8::shield::runtime`). Boot wiring is
//! shell-owned (policy design §4.2b); this module exposes the step-5
//! gate handlers call. See `services/ledger/DESIGN.md` §5.

#[allow(unused_imports)]
pub use sentin8::shield::runtime::{
    check_policy, engine_for, policy_bus_client, request_ctx, spawn_hot_reload,
};
pub use sentin8::shield::PolicyEngine as Engine;

/// Step 5 for the ledger: verify (steps 1-4) already ran; evaluate + 403
/// on enforced deny. Handlers call this instead of any JWT scope check.
pub use sentin8::shield::runtime::require_policy as require;
