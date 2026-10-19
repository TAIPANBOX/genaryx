Feature: A passkey is required by default, and the software-signed fallback is a named choice

  A console with nobody enrolled used to run a sensitive command on the session
  alone and journal it software-signed. That made the per-action ceremony
  optional by default, which is the opposite of what the invariant says.

  @decided 2026-10-19: the passkey ceremony is required by default. The
  software-signed fallback, which runs a sensitive command on the session alone
  when no passkey is enrolled, is off unless the operator sets an explicit
  switch, and the console announces that switch loudly at startup and on every
  use of the fallback.

  # @test:by_default_a_sensitive_command_with_nobody_enrolled_is_refused
  Scenario: A console with nobody enrolled refuses every sensitive command by default
    Given a console whose fallback switch is not set
    And an admin with no enrolled passkey
    When the admin asks for any command that carries the ceremony
    Then the console refuses it with 403 and says how to enrol a passkey

  # @test:by_default_the_probe_says_the_passkey_policy_is_on
  Scenario: The passkey probe reports the requirement as on by default
    Given a console whose fallback switch is not set
    When a signed-in caller asks the passkey probe for the policy
    Then the probe reports that the policy requires a passkey

  # @test:by_default_the_first_passkey_enrols_on_the_password_and_then_the_ceremony_runs
  Scenario: The first passkey is enrolled with the operator password, then the ceremony runs
    Given a console whose fallback switch is not set
    And an admin with no enrolled passkey
    When the admin enrols a first passkey with the operator password
    Then a sensitive command asks for the passkey assertion instead of being refused

  # @test:the_passkey_is_required_when_the_fallback_switch_is_unset_or_blank
  Scenario: An unset or blank fallback switch keeps the passkey required
    Given the fallback switch is unset or blank
    Then the passkey is required

  # @test:the_fallback_switch_drops_the_requirement_only_for_the_words_an_operator_would_type
  Scenario: The fallback switch turns the software-signed fallback on for a true word
    Given the fallback switch is set to a true word such as 1, true, yes or on
    Then the passkey is not required and the fallback is allowed

  # @test:a_typo_or_a_value_nobody_meant_keeps_the_passkey_required
  Scenario: A typo in the fallback switch keeps the passkey required
    Given the fallback switch is set to a value nobody meant, such as 0, off or ON!
    Then the passkey is still required

  # @test:the_fallback_switch_has_its_own_name_in_the_environment
  Scenario: The fallback switch has its own variable name
    Given an operator who wants the software-signed fallback
    When the operator looks up the variable that turns it on
    Then the name is GENARYX_WEB_ALLOW_SOFTWARE_CEREMONY
