// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Bill8:Ledger
// SPDX-License-Identifier: Apache-2.0

//! Service self-registration data for Bill8:Ledger.
//! Output via `--print-registration`. The vertical owns this data; the
//! platform (`vertical enroll`) provisions the roles, scope grants, and
//! policy seeds from it. See `services/ledger/DESIGN.md` §4/§5.

use sentin8::registration::{
    AdminScope, AdminUi, AdminUiPage, ErrorCode, M2mGrant, PgRole, PolicySetDecl, Registration,
    SqlMigration,
};
use utoipa::OpenApi;

const PG_ROLES: &[PgRole] = &[PgRole {
    scope: "bill8:ledger:read,bill8:ledger:write",
    role: "bill8_ledger_service",
    bypass_rls: false,
    database: "bill8",
    cross_schema_grants: &[],
}];

/// Workload scope grants - one entry per workload this service registers.
/// The ledger holds no cross-service reads of its own: payments/billing/
/// merchants call INTO the ledger via the `bill8-{x}-ledger` workloads
/// declared on THEIR registrations (see payments/registration.rs).
const M2M_GRANTS: &[M2mGrant] = &[
    M2mGrant {
        workload: "bill8:ledger",
        scopes: &[
            "sentin8:bus:produce",
            "sentin8:bus:consume",
            "bill8:ledger:admin",
            "sentin8:stat:measure",
        ],
    },
    M2mGrant {
        workload: "ledger",
        scopes: &[
            "sentin8:bus:produce",
            "sentin8:bus:consume",
            "bill8:ledger:admin",
            "sentin8:stat:measure",
        ],
    },
    M2mGrant {
        workload: "ledger-audit",
        scopes: &["sentin8:sys:audit:write"],
    },
    M2mGrant {
        workload: "bill8:ledger-policy",
        scopes: &["sentin8:bus:consume", "sentin8:stat:measure"],
    },
    M2mGrant {
        workload: "ledger-policy",
        scopes: &["sentin8:stat:measure"],
    },
    M2mGrant {
        workload: "bill8-ledger-tls",
        scopes: &["sentin8:sys:instances:read"],
    },
];

const ADMIN_SCOPES: &[AdminScope] = &[
    AdminScope::Typed {
        role: "bill8:ledger:admin",
        scope: "bill8:ledger:accounts:read",
    },
    AdminScope::Typed {
        role: "bill8:ledger:admin",
        scope: "bill8:ledger:accounts:write",
    },
    AdminScope::Typed {
        role: "bill8:ledger:admin",
        scope: "bill8:ledger:balances:read",
    },
    AdminScope::Typed {
        role: "bill8:ledger:admin",
        scope: "bill8:ledger:balance-transactions:read",
    },
    AdminScope::Typed {
        role: "bill8:ledger:admin",
        scope: "bill8:ledger:balance-transactions:write",
    },
    AdminScope::Typed {
        role: "bill8:ledger:admin",
        scope: "bill8:ledger:payouts:read",
    },
    AdminScope::Typed {
        role: "bill8:ledger:admin",
        scope: "bill8:ledger:payouts:write",
    },
    AdminScope::Typed {
        role: "bill8:ledger:admin",
        scope: "bill8:ledger:transfers:read",
    },
    AdminScope::Typed {
        role: "bill8:ledger:admin",
        scope: "bill8:ledger:transfers:write",
    },
    AdminScope::Typed {
        role: "bill8:ledger:admin",
        scope: "bill8:ledger:fees:read",
    },
    AdminScope::Typed {
        role: "bill8:ledger:admin",
        scope: "bill8:ledger:fees:write",
    },
    AdminScope::Typed {
        role: "bill8:ledger:admin",
        scope: "bill8:ledger:policy:read",
    },
    AdminScope::Typed {
        role: "bill8:ledger:admin",
        scope: "bill8:ledger:policy:write",
    },
];

const ERROR_CODES: &[ErrorCode] = &[
    ErrorCode {
        enum_name: "LedgerBalanceNotFound",
        error_code: "ledger_balance_not_found",
        status_code: 404,
    },
    ErrorCode {
        enum_name: "LedgerImbalanced",
        error_code: "ledger_imbalanced",
        status_code: 422,
    },
    ErrorCode {
        enum_name: "LedgerInsufficientFunds",
        error_code: "ledger_insufficient_funds",
        status_code: 422,
    },
    ErrorCode {
        enum_name: "LedgerIllegalTransition",
        error_code: "ledger_illegal_transition",
        status_code: 422,
    },
];

const ADMIN_UI_PAGES: &[AdminUiPage] = &[
    AdminUiPage {
        route: "overview",
        label: "Overview",
        ui_endpoint: "/v1/admin/ui/overview",
        data_endpoint: "/v1/admin/overview",
        icon: "0xe1b1",
    },
    AdminUiPage {
        route: "accounts",
        label: "Accounts",
        ui_endpoint: "/v1/admin/ui/accounts",
        data_endpoint: "/v1/admin/accounts",
        icon: "0xe040",
    },
    AdminUiPage {
        route: "balances",
        label: "Balances",
        ui_endpoint: "/v1/admin/ui/balances",
        data_endpoint: "/v1/admin/balances",
        icon: "0xe083",
    },
    AdminUiPage {
        route: "transactions",
        label: "Transactions",
        ui_endpoint: "/v1/admin/ui/transactions",
        data_endpoint: "/v1/admin/transactions",
        icon: "0xe53e",
    },
    AdminUiPage {
        route: "payouts",
        label: "Payouts",
        ui_endpoint: "/v1/admin/ui/payouts",
        data_endpoint: "/v1/admin/payouts",
        icon: "0xe8af",
    },
    AdminUiPage {
        route: "fees",
        label: "Fee Model",
        ui_endpoint: "/v1/admin/ui/fees",
        data_endpoint: "/v1/admin/fees",
        icon: "0xe263",
    },
];

/// The scope-mirror Shield text - action-granular permits over the LIVE
/// effective scope set, mirroring the exact step-5 gates this service's
/// handlers evaluate. Seeded by `vertical enroll` as `mode='enforced'`.
pub const SCOPE_MIRROR_POLICIES: &str = "permit(principal, action == Action::\"bill8:ledger:accounts:read\", resource) when { principal.scopes.contains(\"bill8:ledger:accounts:read\") };\npermit(principal, action == Action::\"bill8:ledger:accounts:read\", resource) when { principal.scopes.contains(\"sentin8:sys:admin\") };\npermit(principal, action == Action::\"bill8:ledger:accounts:read\", resource) when { principal.scopes.contains(\"sentin8:sys:org:admin\") };\npermit(principal, action == Action::\"bill8:ledger:accounts:read\", resource) when { principal.scopes.contains(\"sentin8:sys:env:admin\") };\npermit(principal, action == Action::\"bill8:ledger:accounts:write\", resource) when { principal.scopes.contains(\"bill8:ledger:accounts:write\") };\npermit(principal, action == Action::\"bill8:ledger:accounts:write\", resource) when { principal.scopes.contains(\"sentin8:sys:admin\") };\npermit(principal, action == Action::\"bill8:ledger:accounts:write\", resource) when { principal.scopes.contains(\"sentin8:sys:org:admin\") };\npermit(principal, action == Action::\"bill8:ledger:accounts:write\", resource) when { principal.scopes.contains(\"sentin8:sys:env:admin\") };\npermit(principal, action == Action::\"bill8:ledger:balances:read\", resource) when { principal.scopes.contains(\"bill8:ledger:balances:read\") };\npermit(principal, action == Action::\"bill8:ledger:balances:read\", resource) when { principal.scopes.contains(\"sentin8:sys:admin\") };\npermit(principal, action == Action::\"bill8:ledger:balances:read\", resource) when { principal.scopes.contains(\"sentin8:sys:org:admin\") };\npermit(principal, action == Action::\"bill8:ledger:balances:read\", resource) when { principal.scopes.contains(\"sentin8:sys:env:admin\") };\npermit(principal, action == Action::\"bill8:ledger:balance-transactions:read\", resource) when { principal.scopes.contains(\"bill8:ledger:balance-transactions:read\") };\npermit(principal, action == Action::\"bill8:ledger:balance-transactions:read\", resource) when { principal.scopes.contains(\"sentin8:sys:admin\") };\npermit(principal, action == Action::\"bill8:ledger:balance-transactions:write\", resource) when { principal.scopes.contains(\"bill8:ledger:balance-transactions:write\") };\npermit(principal, action == Action::\"bill8:ledger:balance-transactions:write\", resource) when { principal.scopes.contains(\"sentin8:sys:admin\") };\npermit(principal, action == Action::\"bill8:ledger:balance-transactions:write\", resource) when { principal.scopes.contains(\"sentin8:sys:org:admin\") };\npermit(principal, action == Action::\"bill8:ledger:balance-transactions:write\", resource) when { principal.scopes.contains(\"sentin8:sys:env:admin\") };\npermit(principal, action == Action::\"bill8:ledger:payouts:read\", resource) when { principal.scopes.contains(\"bill8:ledger:payouts:read\") };\npermit(principal, action == Action::\"bill8:ledger:payouts:read\", resource) when { principal.scopes.contains(\"sentin8:sys:admin\") };\npermit(principal, action == Action::\"bill8:ledger:payouts:write\", resource) when { principal.scopes.contains(\"bill8:ledger:payouts:write\") };\npermit(principal, action == Action::\"bill8:ledger:payouts:write\", resource) when { principal.scopes.contains(\"sentin8:sys:admin\") };\npermit(principal, action == Action::\"bill8:ledger:payouts:write\", resource) when { principal.scopes.contains(\"sentin8:sys:org:admin\") };\npermit(principal, action == Action::\"bill8:ledger:payouts:write\", resource) when { principal.scopes.contains(\"sentin8:sys:env:admin\") };\npermit(principal, action == Action::\"bill8:ledger:transfers:read\", resource) when { principal.scopes.contains(\"bill8:ledger:transfers:read\") };\npermit(principal, action == Action::\"bill8:ledger:transfers:write\", resource) when { principal.scopes.contains(\"bill8:ledger:transfers:write\") };\npermit(principal, action == Action::\"bill8:ledger:fees:read\", resource) when { principal.scopes.contains(\"bill8:ledger:fees:read\") };\npermit(principal, action == Action::\"bill8:ledger:fees:write\", resource) when { principal.scopes.contains(\"bill8:ledger:fees:write\") };\npermit(principal, action == Action::\"bill8:ledger:fees:write\", resource) when { principal.scopes.contains(\"sentin8:sys:admin\") };\n";

const POLICIES: &[PolicySetDecl] = &[PolicySetDecl {
    policy_set_name: "scope-mirror",
    description: "Action-granular scope permits for bill8:ledger over the LIVE effective scope set. Step 5 is the only gate.",
    policies: SCOPE_MIRROR_POLICIES,
    schema: None,
    mode: "enforced",
    default_enabled: true,
}];

const SQL_1_INIT: &str = include_str!("../sql/init.sql");

fn openapi_spec() -> serde_json::Value {
    serde_json::to_value(crate::openapi::LedgerOpenApi::openapi())
        .unwrap_or(serde_json::Value::Null)
}

/// Print the registration JSON to stdout (`--print-registration [--section=…]`).
pub fn print(section: Option<String>) -> anyhow::Result<()> {
    let reg = Registration {
        service_type: "bill8:ledger",
        subdomain: "bill8-ledger",
        health_path: "/v1/health",
        admin_scopes: ADMIN_SCOPES,
        error_codes: ERROR_CODES,
        consent_providers: &[],
        consent_flows: &[],
        policies: POLICIES,
        relations: &[],
        event_triggers: &[],
        schedules: &[],
        admin_ui: Some(AdminUi {
            label: "Ledger",
            icon: "0xe040",
            service_flag: "ledger",
            pages: ADMIN_UI_PAGES,
        }),
        sql_migrations: vec![SqlMigration {
            version: 1,
            name: "init",
            sql: SQL_1_INIT,
        }],
        pg_roles: PG_ROLES,
        audit_findings: None,
        service_actions: None,
        m2m_grants: Some(M2M_GRANTS),
        openapi_url: "/openapi.json",
        openapi: openapi_spec,
    };
    sentin8::registration::print_registration(section, &reg, openapi_spec)
}
