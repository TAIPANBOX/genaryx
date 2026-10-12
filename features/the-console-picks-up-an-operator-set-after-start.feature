Feature: The console picks up an operator set after it started

  A console is often running before its operator exists: on Kubernetes the
  Deployment starts it, and only then can set-password run inside the pod.
  The console used to read whether it had an operator once, at startup, so
  set-password reported success and wrote operator.json while every sign-in
  stayed refused until somebody restarted the console (stack-k8s GOTCHAS 50,
  measured 2026-10-12 on Docker Compose with genaryx-console v1.1.34).

  @decided 2026-10-12: while the console holds no operator, a sign-in and the
  session answer read operator.json again and keep what they find, so a
  console started first picks its operator up on the next request. Once it
  holds one it does not read the file again: a changed password on a running
  console is a separate question and stays as it was.

  # @test:an_operator_set_after_start_signs_in_without_a_restart
  Scenario: set-password on a running console works at the next sign-in
    Given a console started with no operator account
    And a sign-in is refused because the box has no operator yet
    When set-password writes the account while the console keeps running
    Then the next sign-in with that password succeeds, with no restart
    And a wrong password is still refused

  # @test:the_session_route_says_configured_once_an_operator_is_set_after_start
  Scenario: The sign-in page stops saying the box has no operator
    Given a console started with no operator account
    When set-password writes the account while the console keeps running
    Then the session answer says the box is configured
    And it does not say anybody is signed in

  # @test:a_first_enrollment_takes_an_operator_password_set_after_start
  Scenario: The operator password set after start also enrolls the first passkey
    Given an admin signed in through the IdP on a console with no operator account
    When set-password writes the account while the console keeps running
    Then that password is accepted to enroll the first passkey

  # @test:a_half_written_operator_file_is_never_picked_up
  Scenario: A sign-in that reads the file mid-write is refused, not misled
    Given a console with no operator account
    When a sign-in reads operator.json before set-password has finished writing it
    Then that sign-in is refused because the box has no operator yet
    And the first sign-in after the write completes succeeds

  # @test:an_operator_loaded_at_startup_is_not_reread_from_disk
  # @test:an_operator_set_after_start_signs_in_without_a_restart
  Scenario: A console that already holds an operator keeps it until restarted
    Given a console that holds an operator account
    When set-password writes a different password while it keeps running
    Then the previous password still signs in
    And the new one is refused until the console restarts
