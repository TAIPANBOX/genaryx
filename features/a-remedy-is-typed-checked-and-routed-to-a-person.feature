Feature: A remedy is a typed value that is checked, proved where a proof exists, and routed to a person

  An incident ends in something an operator does: change a policy, set a limit,
  stop an agent, revoke a delegation. Until now that was a kind and a bag of
  parameters, and nothing said which proof applies, whether it fits the incident,
  or how it would be applied. A remedy is now one of four typed values, each
  refused when it is not well formed, each knowing its proof and its route.

  @decided 2026-11-04: a remedy with no proof can still be applied, and the case
  says it was unproven; nothing is applied without a person, so a remedy is
  routed to the existing ceremony command or to the place outside the console
  where it is done, and never executed here; an identity refusal is filed under
  its key, so it is offered no remedy that would charge the agent it claimed.
  A Mockryx scenario is a fire drill against the operator's own gateway, so one
  is named and never started.

  # @test:an_event_type_is_classified_by_the_table_and_any_other_is_unclassified
  # @test:every_type_in_the_table_is_there_once_and_names_a_scenario_that_ships
  # @test:an_event_type_the_table_does_not_hold_is_offered_nothing_and_never_guessed
  Scenario: An incident's class comes from one table and an unknown type is not guessed
    Given the event types that mean something to a remedy, and types that are near them or new
    When the console classifies each
    Then a listed type has its class and the shipped scenario that rehearses it, if there is one
    And any other type, a near miss included, is unclassified and offered nothing

  # @test:each_class_is_offered_the_kinds_that_fit_it
  # @test:a_spend_incident_is_classified_offered_a_budget_and_a_drill
  Scenario: Each class is offered the kinds of remedy that fit it, with the drill that rehearses it
    Given a spend incident in a real store
    When a remedy is checked against it
    Then it is offered a budget limit and a stop, and the runaway-budget scenario is named
    And the scenario is reported present, absent or unknown according to the box's scenario directory
    And a policy candidate is reported as not fitting

  # @test:an_identity_incident_is_offered_no_remedy_and_says_why
  # @test:an_identity_refusal_is_offered_nothing_and_a_stop_of_its_claimed_agent_does_not_fit
  Scenario: An identity refusal is offered nothing that would charge the agent it claimed
    Given an identity refusal, filed under the key that made the call
    When a stop or a revocation of the claimed agent is checked against it
    Then it does not fit, the reason is given, and no scenario is named

  # @test:a_remedy_is_checked_before_anything_is_proved_or_routed
  # @test:a_remedy_reads_from_the_shape_the_command_takes_and_nothing_more
  # @test:a_remedy_that_is_not_well_formed_is_refused_before_the_store_is_read
  Scenario: A remedy that is not well formed is refused before anything runs
    Given an empty policy, a budget with no limit, a step limit of zero, a subject that is no agent and a reason inside a remedy
    When the remedy is checked
    Then each is refused, and the store is not read

  # @test:a_policy_candidate_goes_to_the_policy_proof_and_a_budget_to_the_budget_proof
  Scenario: A policy candidate and a budget limit are each proved by their own proof
    Given an incident, a policy candidate and a budget limit, and no archive or trace configured
    When each is checked
    Then the policy candidate asks for the policy archive and the budget limit asks for the trace directory

  # @test:a_stop_is_unproven_and_says_so_and_routes_to_the_ceremony
  Scenario: A stop has no proof, says so, and is still routed
    Given a stop of an agent
    When it is checked
    Then the proof is the statement that no recorded history replays an act, and no slot is taken
    And it is routed to agent_stop with its subject, the reason left for the person

  # @test:a_stop_or_a_revocation_routes_to_its_ceremony_command_with_its_subject
  # @test:a_policy_or_a_budget_has_no_console_command_and_says_where_it_is_done
  # @test:every_remedy_route_is_a_real_admin_command_that_needs_the_ceremony
  Scenario: A remedy is routed to a ceremony command or to where it is done outside the console
    Given the four remedies
    When their routes are read
    Then a stop and a revocation name an admin command that needs the ceremony
    And a policy and a limit say that no console command does it and where it is done

  # @test:an_incident_with_no_event_row_is_refused_by_name
  Scenario: An incident with no event row is refused by name
    Given an incident id from another plane, one the store does not hold and one that is no id
    When a remedy is checked against it
    Then each is refused by name and nothing is proved

  # @test:checking_a_remedy_is_a_viewer_read
  Scenario: Checking a remedy is a read
    Given a viewer
    When they check a remedy
    Then it is allowed, with no ceremony, because it applies nothing
