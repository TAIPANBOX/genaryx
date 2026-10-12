Feature: The console closes a retained reservation with evidence

  The gateway holds a reservation open after a call whose outcome it never
  learned (tokenfuse invariant 50). Closing one is a person's act: an
  operator lists the retained reservations from the gateway and closes one,
  after a passkey assertion, by minting a short-lived token the gateway
  verifies. Nothing closes a reservation on a timer, and no administrative
  adjustment is offered.

  @decided 2026-10-11: closing is a person's act, taken with a passkey in the
  console; the console mints a token bound to one reservation id and its
  revision only after that assertion, and the gateway verifies it with a
  secret the two share. A closure is observed (the cost the evidence shows)
  or no-charge (nothing was billed).

  # @test:listing_reads_get_v1_retained_with_the_admin_key
  # @test:a_viewer_may_list_retained_reservations
  Scenario: An operator lists the retained reservations from the gateway
    Given the gateway lists retained reservations behind its admin key
    When the console reads them
    Then it sends GET /v1/retained with the gateway admin key
    And each reservation arrives with the revision a closure must name

  # @test:a_gateway_with_the_closure_route_off_says_so_when_listing
  # @test:listing_with_no_gateway_says_so_rather_than_an_empty_list
  Scenario: A gateway that cannot list says so rather than showing nothing held
    Given the gateway's closure route is off, or no gateway is ready
    When the console reads the retained reservations
    Then it names why, never an empty list

  # @test:money_close_retained_without_an_assertion_is_428
  # @test:money_close_retained_with_a_valid_assertion_reaches_the_command
  Scenario: Closing needs a fresh passkey assertion
    Given an admin has an enrolled passkey
    When they ask to close a retained reservation without an assertion
    Then the console answers 428 and mints nothing
    And with a fresh assertion for that exact closure the command runs

  # @test:a_viewer_or_approver_is_refused_by_the_role_gate_on_money_close_retained
  # @test:closing_a_retained_reservation_is_admin_and_listing_is_a_read
  Scenario: Only the role that may change a budget may close a reservation
    Given a signed-in viewer, or a signed-in approver
    When they ask to close a retained reservation
    Then the console refuses with role admin required

  # @test:the_closure_token_matches_the_gateway_test_vector
  # @test:closing_posts_the_evidence_with_a_fresh_token_bound_to_the_reservation
  Scenario: The console mints the token the gateway verifies
    Given the console and the gateway share a closure secret
    When an operator closes reservation 7 at its listed revision with an invoice line
    Then the console generates the operation id and mints a token that lives two minutes
    And it posts the evidence to POST /v1/retained/close with the admin key and the token in x-fuse-closure-token

  # @test:a_closure_is_journaled_with_the_acting_principal_and_its_result
  # @test:a_stale_revision_is_refused_and_journaled_as_the_gateways_real_409
  # @test:an_unreachable_gateway_is_journaled_with_status_zero
  Scenario: Every closure attempt is in the console's chain with who made it
    Given a signed-in operator closes a reservation
    When the gateway closes it, refuses it, or cannot be reached
    Then the console journal records the operator, the reservation, the evidence and the gateway's real status

  # @test:a_200_that_does_not_confirm_the_closure_is_never_success
  Scenario: An answer that does not confirm the closure is never shown as closed
    Given something answers 200 without saying the reservation was closed
    When the console reports the outcome
    Then it is reported as unconfirmed, never as closed

  # @test:evidence_that_does_not_fit_its_kind_is_refused_before_any_call
  Scenario: Evidence that does not fit its kind is refused before anything is signed
    Given an observed closure with no amount, a no-charge closure with an amount, or an empty evidence reference
    When the operator asks to close
    Then the console refuses it and the gateway is never called

  # @test:an_unconfigured_console_answers_not_configured_and_calls_nobody
  # @test:a_secret_shorter_than_32_bytes_refuses_to_start
  Scenario: The closure secret is configured or the console says so
    Given GENARYX_CLOSURE_SECRET_FILE is unset
    When an operator asks to close
    Then the console answers not configured and calls nobody
    And a secret file holding fewer than 32 bytes stops the console from starting

  # @test:the_secret_and_the_token_never_appear_in_the_journal_or_the_answer
  # @test:the_copilot_crate_cannot_reach_the_closure_secret_or_mint_a_token
  Scenario: The secret stays with the console, and Felyx cannot sign
    Given the console holds the closure secret
    When any closure outcome is reported or journaled
    Then neither the secret nor the token appears in it
    And the copilot has no way to reach the secret or mint a token
