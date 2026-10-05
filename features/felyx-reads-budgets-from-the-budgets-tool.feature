Feature: Felyx states a budget only when a tool returned it

  Measured 2026-10-05 on the forge lab, console v1.1.19, Felyx on Haiku 4.5
  through the stack's own gateway. Asked about the home router agents, Felyx
  said p1-beryl2 "exceeded budget alert (spent $0.004019 vs $0.001 cap)"
  although no budget exists on any beryl2 run (the $0.001 cap was
  p1-brume's), and asked which runs have a budget it called mig-flint "no
  budget" although the Cloud's GET /v1/budgets held 4500 uUSD for it, spent
  3210. Felyx had no budgets tool, so it read budgets off `alerts`, which
  lists a run only once it is near or over its limit.

  @decided 2026-10-05: the copilot gets a read-only `budgets` tool (run
  budgets from GET /v1/budgets joined with each run's spend, unit budgets
  from GET /v1/unit-budgets with month-to-date spend), keeps its no-signer,
  propose-only shape, and its prompt and its answer check stop it from
  stating a budget fact that no tool returned.

  # @test:a_run_with_a_budget_and_no_alert_is_listed_with_its_budget_and_spend
  Scenario: A run under its budget is still a run with a budget
    Given the Cloud holds a budget of 4500 uUSD for mig-flint, which has spent 3210
    And no alert exists for mig-flint because it is under its threshold
    When Felyx reads budgets
    Then mig-flint is listed with budget 0.0045 USD, spent 0.00321 USD, not at or over it

  # @test:a_cap_set_on_one_run_is_never_reported_on_another
  Scenario: A cap belongs to the run it was set on
    Given p1-brume has a 1000 uUSD budget and p1-beryl2 has none
    When Felyx reads budgets
    Then p1-brume is listed with its own budget and is over it
    And p1-beryl2 is listed among the runs without a budget, never with p1-brume's cap

  # @test:the_tool_says_an_absent_run_has_no_control_plane_budget_and_what_it_cannot_see
  # @test:a_budget_with_no_spend_record_says_spent_is_unknown_not_zero
  Scenario: The tool says what an absence means and what it cannot see
    Given the budgets tool returns its rows
    Then it says a run not listed has no budget on the control plane
    And it names the gateway-local ceilings it cannot see
    And a budget whose run has no spend record shows spend as unknown, not zero

  # @test:unit_budgets_carry_their_month_to_date_spend
  # @test:an_older_cloud_without_unit_budgets_says_so_rather_than_showing_none
  Scenario: Unit budgets, where the Cloud exposes them
    Given the Cloud holds a monthly budget for a unit
    When Felyx reads budgets
    Then the unit is listed with its budget and its month-to-date spend
    And a Cloud without GET /v1/unit-budgets is reported as unable to say, never as having none

  # @test:the_budgets_tool_only_reads
  # @test:copilot_does_not_depend_on_the_signer
  Scenario: The copilot still only reads and proposes
    When Felyx reads budgets
    Then every request it makes to the Cloud is a GET
    And the tool is not a propose tool, and the copilot crate still holds no signer

  # @test:a_budget_stated_for_a_run_no_tool_gave_one_is_sent_back_for_revision
  # @test:a_budgeted_run_called_unbudgeted_is_sent_back_for_revision
  Scenario: A budget statement with no tool behind it is sent back once
    Given Felyx's draft says p1-beryl2 exceeded a $0.001 cap, or that mig-flint has no budget
    And this answer's tool results say otherwise
    When the draft is checked against those results
    Then Felyx is asked to revise it, naming the unsupported run and only that run
    And the revised answer is the one the operator sees

  # @test:budget_talk_without_reading_budgets_is_sent_back_to_read_them
  # @test:the_system_prompt_says_budgets_come_from_the_budgets_tool_not_alerts
  Scenario: Budgets are read from the budgets tool, not inferred from alerts
    Given Felyx called only `alerts` and its draft states a run's budget
    When the draft is checked
    Then Felyx is asked to call `budgets` first, and its instructions say the same

  # @test:a_claim_that_survives_revision_reaches_the_operator_marked_unsupported
  Scenario: A statement that survives revision is marked, not hidden
    Given the revised draft still states a budget no tool returned
    Then the operator sees the answer with a note naming that statement as unsupported

  # @test:a_correct_budget_answer_costs_no_extra_call
  # @test:saying_felyx_cannot_change_a_budget_is_not_a_budget_claim
  Scenario: A correct answer is not second-guessed
    Given an answer whose every budget statement matches the tool results
    And an answer that only says Felyx cannot change a budget
    Then no revision is requested and no extra model call is made
