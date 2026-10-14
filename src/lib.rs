// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Bill8:Ledger
// SPDX-License-Identifier: Apache-2.0

//! # Bill8:Ledger — the double-entry money engine
//!
//! Bill8:Ledger owns every money position and movement in the
//! bill8 vertical: accounts, named per-currency balances, append-only
//! double-entry balance transactions, payouts, transfers, and the fee
//! model. It is the single authority that mutates balances; every other
//! service moves money by asking it to post a balance transaction.
//!
//! # Architecture
//!
//! Boot runs on the platform `sentin8::shell` (registration flag, PQ
//! provider, config, capped pool, state binding, middleware, PQ-TLS
//! listener). This crate supplies its registration data, state, and
//! routes. The authorization pipeline is the platform's 8 steps: token
//! verification (steps 1–4), the Shield policy gate (step 5 — the ONLY
//! gate), transaction-local tenant context, then the domain logic with
//! the outbox row written in the same transaction (outbox-first).
//!
//! # Quick start
//!
//! Run the service binary:
//!
//! ```no_run
//! // bill8-ledger boots via the sentin8 shell; see src/main.rs.
//! ```
//!
//! See `DESIGN.md` for the full detailed design (§2 posting sequence,
//! §3 data model, §8 invariants).

#![forbid(unsafe_code)]

pub mod auth;
pub mod config;
pub mod handlers;
pub mod openapi;
pub mod policy;
pub mod registration;
pub mod server;
pub mod types;
