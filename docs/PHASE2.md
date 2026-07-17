# Phase 2 — policies + hands (Ф2)

Source: `itrat-console/09` Ф2 + `07 §4.3` + `08 §Policy`. Builds on Phase 1
(Cloud connector, CommandBroker, Money panels, `taipan up`). Branch
`phase-2-policies`.

**Exit gate (09 §Ф2).** The full human-in-the-loop cycle: an agent action
holds on a Wardryx policy -> the console's **Approvals Inbox** shows it -> the
operator grants it with a local hardware confirmation (Touch ID) and a
cost-bound `approval_token` is minted -> the agent proceeds (a re-`decide` with
that token resolves to `allow`) -> every step is in the `commands_journal` and
on the bus. Verified by the mockryx `approval-required` + `wardryx-denied-tool`
fire drills against a live `taipan up` stack with the console in the loop, plus
a unit on the token boundary (cost-bound / TTL / single-use).

## Grounded Wardryx contract (read from the wardryx Go source 2026-07-17)

Authoritative wire shapes live in the connector's own doc comments
(`crates/connectors/src/wardryx.rs`, every DTO cites its wardryx `file:line`).
Summary the panels need:

- **Auth**: `Authorization: Bearer <token>` where `<token>` is the BARE token
  from `WARDRYX_KEYS="token:org[:role],..."` (indexed by `parts[0]`; sending the
  full spec 401s — the same #20 bug). Admin role required for
  `/v1/approvals/{id}/decide` and all `/v1/policies` routes.
- **`GET /v1/approvals`** — a BARE JSON array; each item
  `{approval_id, agent_id, run_id, requested_at, decided_at?, decided_by?,
  decision?, pending}` + an untyped `context` map holding `org`, `model`,
  `est_cost_usd`, `attestation_method`, `on_behalf_of`, `reason`,
  `policy_version`, `tool_names`. No `expires_at` on a stored approval.
- **`POST /v1/approvals/{id}/decide`** (admin) — `{decision:grant|deny,
  decided_by}` -> `{approval_id, decision, approval_token?}` (token only on
  grant). 404 / 409 already-decided / 500 no-secret.
- **`approval_token`** = `base64url_nopad(claims).hex(HMAC)`;
  claims `{agent_id, run_id, tools, max_cost_usd, exp, nonce}`; TTL 10 min;
  single-use optional (`WARDRYX_APPROVAL_SINGLE_USE`). The console DECODES it
  for display (`ApprovalTokenClaims`), never verifies (no secret).
- **`GET /v1/policies`** (admin) — bare array of FLATTENED
  `{id, target, deny_tool, allow_domains, require_human_above_usd,
  deny_above_usd, max_steps, deny_if_unattested, updated_at}`; set-level
  `policy_version` (sha256 of the normalized set, first 12 hex).
- **Bus events** (`source:"wardryx"`, schema v0.2): `policy_allow` (info),
  `policy_deny` (high), `approval_requested` (medium, `data.approval_id`),
  `approval_granted` (info), `approval_denied` (high), `approval_timeout`
  (high), `policy_updated` (high). These arrive on the same NDJSON bus the Bus
  Explorer already tails — the decision stream is a FILTER over them, not a new
  REST read.

Two `taipan` gaps block the full-stack e2e (tracked as task #29, fixed in Wave
4, not needed before then): `taipan up --with wardryx` sets no
`WARDRYX_APPROVAL_SECRET` (grant fail-closes 500) and starts wardryx with no
`-policy` (zero policies -> no holds/denies ever).

## Waves

1. **DONE** (commit `58ef1a2`): Wardryx connector — `WardryxClient`
   (approvals/policies/decide), `ApprovalTokenClaims` display decoder,
   `WardryxError`, live hold->grant->token->allow e2e
   (`crates/connectors/{src/wardryx.rs, tests/wardryx_test.rs}`).
2. **Policy panel, BOTH shells** (two parallel tracks from the data contract
   below): Track A (Tauri/Web) `apps/desktop/src-tauri/src/policy/` +
   React panel; Track B (SwiftUI) `crates/ffi/src/wardryx/` (`WardryxHandle`) +
   SwiftUI panel + Touch ID grant.
3. Actionable notifications (`approval_requested` Approve/Deny) + Posture-lite +
   break-glass ceremony + fail-closed privileged path.
4. e2e acceptance (exit gate: mockryx + console approval cycle) + `taipan` gaps
   (#29) + the token-boundary unit.

## Wave 2 data contract + UX (08 §Policy; PARITY across both shells)

A new sidebar item **Policy**, one panel with three sections. Each shell mirrors
its own Phase-1 Money panel (Tauri: `src-tauri/src/money/*` + `MoneyView.tsx`;
SwiftUI: `crates/ffi/src/cloud/*` + `MoneyView.swift`), reusing every existing
convention (empty/failure states like `MoneyEmptyState`, the
`command::record` journal, the `Conformer`-checked bus line, the fail-closed
"always journal the attempt" rule from `finish_mutation`).

1. **Decision stream** — a live, filtered view of the shared bus where
   `source == "wardryx"` (`policy_allow/deny/hold`, `approval_*`,
   `policy_updated`). Reuses the existing event pipeline (the same live tail the
   Bus Explorer renders) — NOT a new REST read. Row: `ts`, a type badge
   (allow=info / deny=high / hold=medium), `agent_id`, `data.reason`,
   `data.tool_names`.
2. **Approvals Inbox** — `WardryxClient::list_approvals()`. The queue of holds
   (`pending == true`) with full context pulled from `context`: who
   (`agent_id` + the `on_behalf_of` chain), what (`tool_names`), how much
   (`est_cost_usd`), why (`reason`), when (`requested_at`), `policy_version`.
   Each row has **Grant** and **Deny**:
   - A privileged mutation. SwiftUI gates it behind a local hardware
     confirmation (LocalAuthentication / Touch ID) BEFORE the call; Tauri uses
     an explicit confirm ceremony (the hardware gate is a Wave-3 upgrade there).
     It calls `decide_approval(id, Grant|Deny, decided_by=<operator principal>)`
     then journals a `console_command` via `command::record` —
     `action` `console.grant_approval` / `console.deny_approval`,
     `decision` `"allow"` (the sanctioned human-in-the-loop path; a `break_glass`
     override of a DENY is separate Wave-3 work), `target` = `approval_id`,
     `verify_result` e.g. `granted ceiling_usd:50.00 ttl_s:600` / `denied`.
     Always journal the attempt, even on a failed/rejected call.
   - On **Grant**, decode the returned `approval_token`
     (`ApprovalTokenClaims::decode`) and show the operator exactly what they
     authorized: agent/run, tools, cost ceiling (`cost_ceiling_usd`), expiry
     countdown (`ttl_remaining`). Caption single-use awareness: if the server
     runs `WARDRYX_APPROVAL_SINGLE_USE` the token is one-shot.
   - Decided approvals move to a history list (`decision`, `decided_by`,
     `decided_at`).
3. **Policy view** — `WardryxClient::list_policies()`. Each policy: `id`,
   `target`, `deny_tool`, `allow_domains`, `require_human_above_usd`,
   `deny_above_usd`, `max_steps`, `deny_if_unattested`, plus the set-level
   `policy_version`. Read-only in MVP (the guarded PUT/DELETE editor is v1).

An environment with no wardryx service resolves to a clean "no policy plane"
empty state (mirror `MoneyEmptyState` / `CloudError::NoEnvironment`), never an
error.

## Parity checklist (Wave 2; both shells before the wave is done)

- [ ] Decision stream renders `wardryx.*` bus events (allow/deny/hold + approval_*)
- [ ] Approvals Inbox lists holds with full context (who/what/cost/why/chain)
- [ ] Grant mints and shows a DECODED cost-bound token (ceiling + TTL + single-use note)
- [ ] Grant/Deny journals a conforming `console_command` (action `console.grant_approval`/`console.deny_approval`)
- [ ] Policy view shows policies + set-level `policy_version`
- [ ] No-wardryx environment renders a clean empty state, not an error

## Wave 3 — actionable notifications + Posture-lite (both shells, PARITY)

Two per-shell tracks (Track A `apps/desktop`, Track B `apps/macos`); each shell
does BOTH features, from existing signals (no `crates/core`/`crates/connectors`
change; add a minimal ffi accessor only if strictly unavoidable). Break-glass +
the fail-closed privileged-path precheck are a SEPARATE, security-focused pass
(task #30), not this wave.

### Actionable notifications
When an `approval_requested` event arrives on the live bus (the same feed the
Decision Stream filters), raise a NATIVE notification -
"Approval needed - `<agent_id>` (`data.reason`)":
- SwiftUI: `UNUserNotificationCenter` + `UNNotificationAction` (request
  authorization once on launch); actions **Review / Approve / Deny**.
- Tauri: `tauri-plugin-notification` (add it if absent); actions where the
  platform supports them, otherwise a tap that focuses the Policy panel.
- SECURITY (non-negotiable): an Approve/Deny action must NOT silently execute
  the privileged mutation. It DEEP-LINKS into the Approvals Inbox focused on
  that `approval_id`, where the operator completes the existing
  Touch-ID / confirm-gated grant/deny from Wave 2. The notification alerts and
  routes; the hardware/confirm gate stays on the actual decision, never bypassed.
- De-dupe: at most one notification per `approval_id` (never re-raised on a
  list refresh).
- Mute: per agent / per run / per environment (an in-memory mute set is fine for
  v0); a muted key raises nothing.

### Posture-lite
A new sidebar item **Posture** - a read-only list of stack-sanity findings
computed from already-observable signals (the resolved env source, the live bus
events, `list_policies()`); each finding = {severity, title, why it matters, how
to fix (a concrete command / env var)}. v0 zonds (identical set, both shells):
1. **devkey in use** (high) - the environment authenticates via a devkey /
   `ALLOW_DEVKEY` fallback (org resolved to `default`, or the bearer is literally
   `devkey`). Fix: mint real keys (`taipan up` mints them; or set real
   `TOKENFUSE_CLOUD_KEYS` / `WARDRYX_KEYS`).
2. **Governance fail-open: no policies** (high) - wardryx is reachable but
   `list_policies()` is empty, so every action is allowed. Fix: PUT policies (or
   `taipan up --with wardryx` with a seeded `-policy`).
3. **Schema mix v0.1 + v0.2** (info) - the bus carries both envelope versions
   (tokenfuse/qryx emit v0.1, wardryx/verdryx/mockryx v0.2). Fix: the
   tokenfuse-core v0.2 PR (workstream C). Informational, not a defect.
4. **Bus stale** (medium) - no events observed recently, or the events source is
   empty. Fix: check the feeder / the descriptor's events paths.

The "missing `WARDRYX_APPROVAL_SECRET`" zond is surfaced reactively by the
Approvals Inbox when a grant returns `NoApprovalSecret`; a proactive probe is
deferred.

### Parity checklist (Wave 3)
- [ ] `approval_requested` raises a native, de-duped, mutable notification, both shells
- [ ] notification Approve/Deny ROUTE to the Touch-ID/confirm-gated decision (never silent-execute), both shells
- [ ] Posture panel shows the 4 v0 zonds with why + how-to-fix, both shells

## Wave 3B — break-glass + fail-closed privileged path (task #30, security-focused)

Deferred to its own carefully-reviewed pass: a `console_command` `decision`
of `break_glass` (operator override of a Wardryx `deny`) with a heightened
ceremony (mandatory typed reason + hardware confirm) and loud journaling; and
the fail-closed privileged-path precheck (a privileged mutation consults Wardryx
`/v1/decide` first - `deny` blocks pending an explicit break-glass, `hold`
requires an approval, a missing approval secret refuses rather than proceeds).
Pairs with the `taipan` gateway->wardryx wiring in #29.
