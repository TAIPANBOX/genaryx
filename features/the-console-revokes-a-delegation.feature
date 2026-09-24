Feature: The console revokes a delegation

  An operator who sees a compromised agent in the console needs to cut its
  delegated authority off from there, as a deliberate, attributed act: the
  same class as killing a run. Before this, the only way to revoke was curl
  with the key by hand.

  @claude 2026-09-24: a decision taken under delegated authority, open to
  reversal. delegation_revoke is admin-only and WebAuthn-ceremony-gated, the
  same posture as money_kill_run and remote_operator_wg_revoke, because
  revoking takes something away mid-incident and a stolen session must not
  be able to pull that switch alone.

  # @test:delegation_revoke_with_a_valid_assertion_runs
  Scenario: An admin with a passkey revokes an agent and vouchryx receives it
    Given an admin has an enrolled passkey
    When they confirm a delegation_revoke for a compromised agent with a fresh assertion
    Then the role gate and the webauthn gate both pass
    And the revocation reaches vouchryx over POST /v1/revoke with the operator's own reason

  # @test:a_viewer_is_refused_by_the_role_gate_on_delegation_revoke
  # @test:an_approver_is_refused_by_the_role_gate_on_delegation_revoke
  Scenario: A viewer or an approver cannot revoke a delegation
    Given a signed-in viewer, or a signed-in approver
    When they call delegation_revoke
    Then the console refuses with role admin required
    And vouchryx is never called

  # @test:vouchryx_503_is_reported_as_not_durable_never_as_success
  Scenario: A revocation vouchryx could not persist is shown as such, never as success
    Given vouchryx answers 503 because it could not durably record the revocation
    When the console reports the outcome
    Then it is shown as not durable, never as a success
    And the journal records vouchryx's real 503 status, never a fabricated 200

  # @test:the_key_never_appears_in_the_journaled_line_or_the_returned_error
  # @test:the_key_is_read_out_of_revoke_key_in_exactly_one_place
  Scenario: The revoke key is never shown
    Given the console holds vouchryx's bearer revoke key
    When any outcome is reported, logged, or journaled, success or refusal alike
    Then the key's bytes appear nowhere but the one Authorization header the console sends to vouchryx
