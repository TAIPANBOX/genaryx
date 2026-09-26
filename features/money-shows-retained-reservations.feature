Feature: The Money tab shows retained reservations

  The gateway (tokenfuse invariant 50) can hold a reservation open after a
  call whose outcome it never learned, deliberately neither releasing the
  money nor counting it spent. Before this, the console showed only what was
  spent, and money held for an unknown outcome had no visible home.

  @decided 2026-09-26: retained reservations are read from the gateway's own
  GET /v1/runs (the admin key the console already presents for the
  Credentials plane), shown per run where non-zero plus a fleet-wide total,
  with one plain sentence explaining why the money is held. A gateway that is
  not configured, or not reachable, says so and never renders a total as if
  it had measured zero.

  # @test:retained_runs_filters_to_non_zero_and_sums_the_whole_fleet
  Scenario: Runs with a retained reservation are listed, and the total covers the whole fleet
    Given the gateway answers GET /v1/runs with some runs retaining money and some retaining none
    When the console reads the retained-reservations report
    Then only the runs with a non-zero retained count are listed
    And the total is summed over every run the gateway answered for

  # @test:a_fleet_with_nothing_retained_reports_a_real_zero
  Scenario: A fleet with nothing retained reports a real, measured zero
    Given the gateway answers GET /v1/runs with no run retaining anything
    When the console reads the retained-reservations report
    Then the total is zero because the gateway measured zero, not because it could not answer

  # @test:retained_runs_reports_no_environment_rather_than_a_fabricated_zero
  Scenario: An unconfigured gateway says so rather than showing zero
    Given no gateway is configured for this environment
    When the console reads the retained-reservations report
    Then it reports no environment, never a fabricated zero

  # @test:retained_runs_reports_unreachable_rather_than_a_fabricated_zero
  Scenario: An unreachable gateway says so rather than showing zero
    Given the gateway is configured but not reachable
    When the console reads the retained-reservations report
    Then it reports the gateway as unreachable, never a fabricated zero
