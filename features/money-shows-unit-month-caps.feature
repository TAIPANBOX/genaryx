Feature: The Money panel shows each business unit against its month cap

  A gateway caps what a business unit may spend in a UTC calendar month and
  refuses the unit's calls with unit_budget_exceeded once the cap is reached.
  The Cloud holds the central caps every gateway polls and the month's spend
  per unit, and the console showed neither: only its assistant read them, and
  the watch dock said units carry no budget at all. Measured on the forge lab
  on 2026-10-14, a unit capped at USD 0.00005 was refused before any provider
  was called.

  @decided 2026-10-14: the dashboards show what the product does today, with
  a section listing each unit, its cap, its spend this month and its state.
  The cap can be changed from the console because the Cloud already offers a
  signed admin route for it; the change is the same class of act as a run
  budget change.

  # @test:the_units_section_joins_month_spend_with_each_units_cap
  # @test:the_unit_caps_section_shows_each_units_cap_spend_and_state
  Scenario: Each unit shows its cap, its month spend and where it stands
    Given the Cloud holds a USD 0.00005 cap for unit capped, which has spent USD 0.000061 this month
    And a unit with no central cap and a unit under its cap
    When the operator opens the Money panel's unit month caps
    Then capped reads over cap with its spend and cap
    And the unit without a cap reads "no central cap"
    And the over-cap units lead the section

  # @test:a_unit_near_its_cap_reads_near
  Scenario: A unit at 80 percent of its cap reads near cap
    Given a unit has spent 90 percent of its central cap this month
    When the operator opens the unit month caps
    Then it reads near cap

  # @test:a_cloud_without_unit_budgets_shows_caps_unknown_never_no_cap
  # @test:caps_the_console_could_not_read_read_cap_unknown_never_none
  Scenario: Caps the console could not read are unknown, never none
    Given the Cloud's cap map answers 404, 500 or 403
    When the operator opens the unit month caps
    Then every unit reads "cap unknown", never "no central cap"
    And the section says which read did not answer

  # @test:a_cloud_without_units_still_lists_its_caps_with_spend_unknown
  Scenario: Caps are listed even when the month spend cannot be read
    Given the Cloud has no unit spend route and holds four central caps
    When the operator opens the unit month caps
    Then all four units are listed with their caps and "spend unknown"

  # @test:a_cloud_that_answers_neither_is_an_error_not_an_empty_section
  Scenario: A Cloud that answers neither read is an error, not an empty section
    Given both unit reads answer 500
    When the operator opens the unit month caps
    Then the section says the caps could not be read

  # @test:hostile_unit_cap_bodies_never_cost_the_section_or_invent_a_cap
  Scenario: A cap map the console cannot parse invents nothing
    Given the cap map answers with a body that is not a unit-to-micros map
    When the operator opens the unit month caps
    Then every unit reads "cap unknown" and no unit shows a cap the body did not hold

  # @test:a_zero_cap_says_it_refuses_every_call
  Scenario: A zero cap says what it does
    Given a unit's central cap is zero
    When the operator opens the unit month caps
    Then the cap reads "$0.00 · refuses every call"

  # @test:setting_a_unit_cap_posts_a_signed_budget_to_the_cloud
  Scenario: Setting a unit cap posts a signed change to the Cloud
    Given an admin with a paired console device
    When they set unit w2x's month cap to USD 0.007168 with a reason
    Then the Cloud receives a device-signed POST /v1/units/w2x/budget with that amount
    And the attempt is journaled with what the Cloud answered

  # @test:money_set_unit_budget_without_an_assertion_is_428
  # @test:a_viewer_or_approver_is_refused_by_the_role_gate_on_money_set_unit_budget
  # @test:setting_a_unit_cap_is_admin_and_reading_caps_is_a_read
  Scenario: Only an admin, with a fresh passkey assertion, can set a unit cap
    Given a viewer, an approver, and an admin with an enrolled passkey
    When each sets a unit cap without a passkey assertion
    Then the viewer and the approver are refused by role
    And the admin is asked for a passkey assertion for this one change

  # @test:a_unit_cap_change_that_cannot_be_a_cap_never_reaches_the_cloud
  # @test:a_unit_cap_change_fails_closed_without_a_device_or_with_a_unit_that_is_no_segment
  Scenario: A cap change that cannot be a cap never reaches the Cloud
    Given an empty reason, an empty unit, the unassigned bucket, a negative or a non-number amount
    When the operator sets the cap
    Then the console refuses it before anything is signed or sent
