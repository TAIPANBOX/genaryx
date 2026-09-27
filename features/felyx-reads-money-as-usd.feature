Feature: Every money value Felyx's tools hand to the model is decimal USD

  Measured 2026-09-27, console v1.1.17, on the forge lab: asked "Which planes
  can you see, and is each one healthy?", Felyx answered "Run
  `genaryx-copilot` exceeded its $50 budget by 27% ($63.64 spent)". The
  Cloud's own figures were `budget_micros: 50000` (five cents) and
  `spent_microusd: 63640` (about six and a third cents), read straight off a
  connector DTO and handed to the model as bare integers. In the SAME
  session, a different question about the same run's own alert got the
  conversion right ("$0.0547 on a $0.05 budget"): the model can do this
  arithmetic, it is just not required to, and a figure that is only
  sometimes converted correctly is a defect even on the calls where it comes
  out right.

  @decided 2026-09-27: every copilot tool whose result carries a money value
  (`money_summary`, `list_runs`, `list_agents`, `savings`, `alerts`,
  `savings_breakdown`, `cost_per_action`) passes its result through a shared
  `dollarize` step before returning it to the model, so a micro-USD integer
  reaches the model only as a decimal amount under a key that says "usd".
  The `genaryx_connectors` wire DTOs themselves are untouched (they stay
  byte-exact mirrors of the Cloud's own `store.rs` shapes); the conversion
  happens once, in the tool layer, on the JSON built for the model.

  # @test:money_summary_dollarizes_its_spend_field
  # @test:money_summary_reaches_the_model_as_a_decimal_usd_field
  Scenario: A headline spend question
    Given the Cloud reports spent_microusd as a bare integer
    When money_summary's result reaches the model
    Then it carries spent_usd as a decimal dollar amount
    And it carries no spent_microusd field at all

  # @test:alerts_dollarizes_both_the_spent_and_the_ambiguously_named_budget_field
  # @test:alerts_reaches_the_model_with_both_spent_and_the_ambiguously_named_budget_as_usd
  Scenario: A near-cap or over-cap run, the exact 2026-09-27 numbers
    Given an alert reports spent_microusd 63640 and budget_micros 50000
    When alerts' result reaches the model
    Then it carries spent_usd 0.06364 and budget_usd 0.05
    And budget_micros, which names no unit in its own key, is never handed
      to the model bare

  # @test:list_runs_dollarizes_every_row_and_the_wrapping_total
  # @test:list_runs_reaches_the_model_with_every_row_and_its_total_as_usd
  Scenario: The top-spenders list, a row and its wrapping total together
    Given list_runs sorts and sums several runs' spent_microusd
    When its result reaches the model
    Then every run's own spend and the wrapping total_spent are both decimal
      USD, not the bare integers the sort and sum were computed from

  # @test:savings_breakdown_dollarizes_its_money_fields
  # @test:savings_reaches_the_model_with_every_amount_as_usd
  # @test:list_agents_reaches_the_model_with_spend_as_usd
  Scenario: Savings, agent spend, and a reason-keyed breakdown
    Given savings and savings_breakdown report several microUSD totals, one
      of them (by_reason_microusd) keyed by block reason rather than by a
      sibling field name
    When their results reach the model
    Then every total, including every value inside the reason-keyed
      breakdown, is decimal USD under a renamed key

  # @test:cost_per_action_dollarizes_its_money_fields_and_keeps_a_null_rate_null
  Scenario: A cost rate that is not known, not zero
    Given cost_per_action reports cost_per_tool_call_microusd as null when
      the rate is undefined
    When its result reaches the model
    Then the renamed cost_per_tool_call_usd field is still null, never a
      computed 0.0 a model could read as a real answer

  # @test:dollarize_never_misreads_an_unrelated_micros_field_as_money
  Scenario: A field that merely ends the same way is not money
    Given a hypothetical future field named with a "_micros" suffix that is
      not a dollar amount (a duration, say)
    When the shared conversion runs
    Then that field is left untouched, because the conversion works from an
      allow-list of known money keys, not a bare suffix match
