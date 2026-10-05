Feature: The Money panel shows every run budget the Cloud holds

  Measured 2026-10-05 on the forge lab: the runs table read a run's budget
  only from GET /v1/alerts plus the budgets set in the current console
  session. /v1/alerts lists a run only once it is near or over its limit, so
  mig-flint (a 4500 uUSD budget, 3210 spent, under its alert threshold)
  showed no budget at all, and the table printed "no cap" for it: a real
  budget shown as an absence.

  @decided 2026-10-05: run budgets in the Money panel come from the Cloud's
  GET /v1/budgets, the same read Felyx's budgets tool uses, and the panel
  tells "this run has no budget" apart from "the console could not read the
  budgets".

  # @test:a_run_with_a_cloud_budget_and_no_alert_shows_its_budget
  Scenario: A run under its budget still shows its budget
    Given the Cloud holds a budget of 4500 uUSD for mig-flint, which has spent 3210
    And no alert exists for mig-flint because it is under its threshold
    When the operator opens the Money panel's runs table
    Then mig-flint shows a budget of 0.0045 USD
    And every other budgeted run shows its own budget, alerted or not

  # @test:a_run_with_no_cloud_budget_reads_as_no_budget
  Scenario: A run with no budget says so only when the budgets were read
    Given the Cloud's budget map answered and holds nothing for p1-beryl2
    When the operator opens the runs table
    Then p1-beryl2 shows no budget
    And every row says its budget was read, so "no cap" is a true statement

  # @test:a_cloud_that_cannot_answer_budgets_says_so_rather_than_no_budget
  # @test:a_cloud_that_answers_neither_budgets_nor_alerts_still_lists_its_runs
  Scenario: A Cloud that cannot answer for budgets is never shown as having none
    Given the Cloud's GET /v1/budgets answers 404, 500 or 403
    When the operator opens the runs table
    Then every run is still listed
    And every row says its budget could not be read, so the table shows "cap unknown", never "no cap"
    And a budget GET /v1/alerts still lists is shown as the real figure it is

  # @test:the_cloud_budget_map_wins_over_the_alert_figure
  Scenario: The budget map is the source, not the alerts derived from it
    Given the budget map and the alerts list disagree about a run's budget
    When the operator opens the runs table
    Then the budget map's figure is the one shown

  # @test:hostile_budget_bodies_never_cost_the_table_or_invent_a_budget
  Scenario: A budget map the console cannot parse costs nothing and invents nothing
    Given GET /v1/budgets answers 200 with a body that is not a run-to-micros map
    When the operator opens the runs table
    Then every run is still listed
    And no row shows a budget the body did not contain
    And every row says its budget could not be read
