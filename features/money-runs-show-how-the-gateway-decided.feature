Feature: The Money panel shows how the gateway decided each run

  TokenFuse Cloud v1.8.0 reports, on every run it returns, the calls the
  gateway refused and the decision of the latest call, the calls charged at
  the fallback rate because the price book had no row for the model, the
  calls from a gateway that named no price basis, the calls forwarded under
  identity warn, and since v1.2.0 the site that pushed the run. The console
  dropped all of it, so a run whose every call was refused read as live, a
  call charged at the fallback rate read as an ordinary charge, and a call
  forwarded under identity warn read as honest.

  @decided 2026-10-14: the dashboards show what the product does today. The
  runs table shows refused runs, fallback pricing and identity warnings
  clearly, and a Cloud that does not report a field is never shown as zero.

  # @test:a_refused_run_reaches_the_runs_table_with_its_reason_and_count
  # @test:a_refused_run_reads_as_refused_with_its_reason_on_the_runs_board
  Scenario: A refused run reads as refused, with its reason
    Given the Cloud reports one refused call on run w2-b3-runaway, latest decision budget_exceeded
    When the operator opens the Money panel's runs table
    Then the run's status reads "refused · budget", not "live"
    And the hover names budget_exceeded and when it was taken
    And "1 refused" sits under the run's call count

  # @test:a_refused_run_is_described_with_its_reason_in_incident_360
  Scenario: Incident 360 says how the gateway decided the run
    Given an incident on a run whose latest call was refused for budget_exceeded
    When the operator opens it in Incident 360
    Then "What the whole run cost" says the latest call was refused and how many calls were

  # @test:fallback_pricing_identity_warnings_and_the_site_reach_the_runs_table
  # @test:a_run_with_fallback_priced_calls_carries_a_fallback_pill_beside_its_model
  Scenario: A run charged at the fallback rate says so
    Given the Cloud reports two calls on run w2-b1-openai charged at the fallback rate for openai/gpt-4.1-mini
    When the operator opens the runs table
    Then the run carries a "fallback ×2" pill
    And its hover says the price book has no row for the model and the spend is a gateway estimate

  # @test:a_run_whose_gateway_named_no_price_basis_says_so
  Scenario: Calls from a gateway that named no price basis are marked
    Given the Cloud reports three calls on a run from a gateway older than the price basis
    When the operator opens the runs table
    Then the run carries a "no basis ×3" pill

  # @test:a_run_forwarded_under_identity_warn_says_so
  Scenario: Calls forwarded under identity warn are marked
    Given the Cloud reports two calls on a run forwarded under identity warn for agent_id_not_allowed
    When the operator opens the runs table
    Then the run carries an "id warn ×2" pill whose hover names agent_id_not_allowed

  # @test:the_site_that_pushed_a_run_is_named_on_the_runs_board
  Scenario: The site that pushed a run is named
    Given the Cloud reports site forge-openai for a run, an empty site for another, and no site field for a third
    When the operator opens the runs table
    Then they read "site forge-openai", "no site named" and "site not reported"

  # @test:a_cloud_older_than_the_decision_fields_leaves_them_unknown_never_zero
  # @test:a_cloud_that_does_not_report_the_decision_fields_is_never_shown_as_clean
  Scenario: A Cloud older than the fields is never shown as clean
    Given a Cloud older than v1.8.0 that sends none of the decision fields
    When the operator opens the runs table
    Then no run shows a refused count, a fallback pill or a zero for them
    And the status hover says this Cloud does not report refused calls

  # @test:hostile_decision_fields_never_cost_the_table_or_invent_a_value
  # @test:a_decision_field_of_the_wrong_type_is_unknown_and_never_costs_the_row
  Scenario: A malformed field costs nothing and invents nothing
    Given a run row whose decision fields carry values of the wrong type
    When the operator opens the runs table
    Then every run is still listed
    And each malformed field is unknown, never a value the body did not hold

  # @test:the_runs_export_carries_the_decision_fields_and_null_when_not_reported
  Scenario: The runs export carries the fields and says what an empty cell means
    Given the operator exports the runs table
    Then the file appends site, blocked, last_decision, last_decision_at, fallback_calls, basis_unreported_calls, identity_warned and last_identity_reason
    And a field the Cloud did not report is an empty cell, never 0
