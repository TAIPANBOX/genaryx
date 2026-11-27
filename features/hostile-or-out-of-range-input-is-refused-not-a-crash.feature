Feature: Input from outside the console is refused by name, never a crash or a wrong number

  Three readers take bytes the console does not control: the WireGuard UAPI
  daemon's answer, the tokenfuse savings report and the box's session
  endpoint. Each one either reads what it was given correctly or refuses it
  with a reason.

  @decided 2026-11-27: a defect found by the coverage wave is fixed test first;
  a key or an amount that cannot be read is an error that says so, and a box
  that cannot be reached is no session, not an unhandled rejection.

  # @test:non_ascii_keys_from_the_daemon_are_refused_not_a_panic
  Scenario: A key with a multi-byte character from the daemon is refused, not a panic
    Given the UAPI daemon answers with a key holding a multi-byte character at any position and any length
    When the console reads the interface state
    Then the key is refused as a bad key and the rest of the answer is still read

  # @test:amounts_past_the_i64_microdollar_range_are_refused_by_name_not_wrapped
  Scenario: A dollar amount past what the console can hold is refused by name
    Given a savings report whose amount is beyond about 9.2 trillion dollars either way
    When the console reads the report
    Then it fails with "USD amount out of range" naming the line, and both ends of the range that fit are read exactly

  # @test:a_box_that_cannot_be_reached_leaves_no_session_and_no_unhandled_rejection
  Scenario: A box that cannot be reached leaves no session badge and no unhandled error
    Given the web console cannot reach the box for its session
    When the header asks who is signed in
    Then there is no session and no unhandled promise rejection
