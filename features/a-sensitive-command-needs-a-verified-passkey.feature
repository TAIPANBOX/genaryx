Feature: A sensitive command needs a passkey that verified the person

  A passkey can answer a challenge with the person only present (a tap), or
  with the person verified (a fingerprint, a face, a device PIN). The
  per-action ceremony exists so that a human confirms this one action, and a
  bare tap does not show that.

  @decided 2026-10-17: a per-action assertion for a sensitive command, for
  enrolling a passkey and for removing one, is accepted only when the
  authenticator verified the person. The challenge asks for that verification.

  # @test:a_ceremony_refuses_a_presence_only_assertion
  Scenario: A tap without verification does not run a sensitive command
    Given an admin with one enrolled passkey
    When the admin confirms a kill with an assertion that verified nobody
    Then the console refuses it with 403 and the command does not run

  # @test:a_ceremony_accepts_a_verified_assertion
  Scenario: A verified assertion still runs the command
    Given an admin with one enrolled passkey
    When the admin confirms a kill with an assertion that verified the person
    Then the command is dispatched

  # @test:the_action_start_asks_the_authenticator_to_verify_the_user
  Scenario: The challenge asks the authenticator to verify the person
    Given an admin with one enrolled passkey
    When the console starts a ceremony for a sensitive command
    Then the challenge carries a user verification requirement of required

  # @test:an_assertion_without_user_verification_is_refused
  Scenario: The verification core refuses an assertion that proves presence only
    Given an enrolled passkey
    When an otherwise valid assertion arrives with the user-verified flag clear
    Then the verification fails as user not verified
