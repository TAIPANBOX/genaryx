Feature: The console throttles password sign-in

  The local operator password is the break-glass factor, so it is the
  factor a guessing run goes after. Until now a sign-in could be attempted
  without limit from any address, and nothing slowed the guesses down.

  @decided 2026-10-17: password sign-in is throttled per account and per
  client address. Once a key has failed too often inside a window it is held
  back until the window passes, and the answer says how long to wait. A
  successful sign-in clears its keys. The memory for these keys is bounded.

  # @test:the_sign_in_is_throttled_per_account_after_repeated_failures
  Scenario: Repeated wrong passwords for one account hold back the account
    Given an operator account with a known password
    When ten wrong passwords for it arrive, each from a different address
    Then the right password is refused with 429 and a Retry-After in seconds

  # @test:the_sign_in_is_throttled_per_client_address
  Scenario: Repeated attempts from one address hold back that address only
    Given an operator account with a known password
    When thirty failed attempts for different names arrive from one address
    Then the right password from that address is refused with 429
    And the right password from another address still signs in

  # @test:a_key_is_refused_after_its_limit_and_admitted_after_the_window
  Scenario: A held key is admitted again once its window has passed
    Given a key that has failed up to its limit inside one window
    When a check arrives ten seconds into the window
    Then it is refused with the fifty seconds left in the window
    And a check at the end of the window is admitted

  # @test:a_success_clears_the_key
  Scenario: A successful sign-in clears the failures of its keys
    Given a key with one failure recorded
    When the sign-in succeeds and one more failure is recorded
    Then the key is still admitted

  # @test:one_key_does_not_throttle_another
  Scenario: One key does not hold back another
    Given a key that has reached its limit
    When a check arrives for a different key
    Then the different key is admitted

  # @test:memory_stays_bounded_under_a_flood_of_distinct_keys
  Scenario: A flood of distinct names does not grow the table without bound
    Given a throttle that holds at most eight keys
    When ten thousand distinct keys each fail once
    Then the throttle holds no more than eight keys

  # @test:a_hostile_username_is_keyed_in_bounded_space
  Scenario: A very long username costs a bounded key
    Given a username of one mebibyte
    When the throttle derives the key for it
    Then the key is at most 256 bytes and differs from the address key of the same text
