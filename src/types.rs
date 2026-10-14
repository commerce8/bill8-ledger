// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Bill8:Ledger
// SPDX-License-Identifier: Apache-2.0

//! Ledger domain types — request/response shapes for the ledger API.
//! Wire DTOs live in `bill8_ledger_contracts`; these are the internal
//! row mappings and validation errors. See `services/ledger/DESIGN.md` §3.

use serde::Deserialize;

/// Domain errors on the money path — every variant maps to an HTTP status
/// and (critically) to a `no rows written` guarantee.
#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    /// The posting batch does not net to zero — rejected pre-write.
    #[error("batch does not net to zero (debit {debit} != credit {credit})")]
    Imbalanced { debit: String, credit: String },
    /// A referenced balance was not found.
    #[error("balance {0} not found")]
    BalanceNotFound(uuid::Uuid),
    /// A balance transacts only in its own currency — rejected pre-write.
    #[error(
        "currency mismatch: balance {balance} is {balance_currency}, posting is {posting_currency}"
    )]
    CurrencyMismatch {
        balance: uuid::Uuid,
        balance_currency: String,
        posting_currency: String,
    },
    /// Insufficient available balance — rejected pre-write.
    #[error("insufficient funds: available {available}, requested {requested}")]
    InsufficientFunds {
        available: String,
        requested: String,
    },
    /// A terminal-state transition was attempted (never regresses).
    #[error("illegal state transition: {from} -> {to}")]
    IllegalTransition { from: String, to: String },
    /// A database error (transaction rolled back atomically).
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// Query params for balance reads.
#[derive(Debug, Default, Deserialize)]
pub struct ListBalancesQuery {
    /// Filter by owning account.
    pub account_id: Option<uuid::Uuid>,
    /// Filter by balance name.
    pub name: Option<String>,
}

/// Request body for `POST /v1/accounts`.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateAccountRequest {
    /// Account family (`customer` / `platform` / `connected`).
    pub account_type: String,
    /// Display name.
    pub display_name: String,
}
