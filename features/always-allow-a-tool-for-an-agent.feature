Feature: Always allow a tool for an agent, from the approvals inbox

  A call held because nothing grants the tool asks a person every time, and a
  person who answers yes to the same agent and the same tool again and again is
  doing a job a policy does. Until now that policy was a command on the box.

  @decided 2026-11-07: a held approval offers to always allow its tool for its
  agent, behind the same passkey ceremony as a grant, writing the grant_tool
  policy through wardryx in the shape stack-single's agent-allow writes it. An
  agent id with no trust domain has the action off, with a clear line. A
  one-time grant stays the default button. Once the same agent and tool have
  been approved three times, the inbox offers it in a row of its own.

  # @test:the_row_keeps_grant_and_deny_and_adds_always_allow_before_them
  # @test:the_button_is_quieter_than_grant_so_grant_stays_the_default
  # @test:a_floor_hold_offers_always_allow_for_its_tool
  Scenario: A held call offers Always allow, and Grant stays the default
    Given a tool call held because nothing grants it
    When the operator reads the row
    Then Grant and Deny are where they were
    And Always allow sits before them, quieter than Grant

  # @test:always_allow_goes_through_the_same_ceremony_as_a_grant
  # @test:a_standing_grant_with_a_passkey_and_no_assertion_is_428
  # @test:a_standing_grant_with_a_valid_assertion_passes_the_gate
  # @test:a_standing_grant_is_an_admin_row_with_the_ceremony
  Scenario: Always allow asks for the same passkey as a grant
    Given an operator with a passkey
    When they press Always allow with no assertion
    Then the console asks for one, and with a good one the command runs

  # @test:an_assertion_for_one_hold_does_not_open_another
  # @test:the_browser_names_only_the_hold_and_never_the_agent_or_the_tool
  # @test:a_hold_wardryx_does_not_have_reaches_no_policy
  Scenario: The passkey confirms one hold, and the policy is read from wardryx's record of it
    Given an assertion made for one hold
    When it is presented for another
    Then it is refused
    And the browser never says which agent or which tool

  # @test:an_approver_is_refused_by_the_role_gate_on_a_standing_grant
  # @test:an_approver_sees_the_button_off_and_why
  # @test:nobody_but_an_admin_is_offered_it
  Scenario: Only an admin can make a grant outlive the call
    Given an approver, who may answer one call
    When they try to always allow it
    Then the button is off and says why, and the command refuses it too

  # @test:a_grant_writes_the_policy_agent_allow_writes
  # @test:a_new_grant_has_the_shape_agent_allow_writes
  # @test:the_id_is_the_id_agent_allow_writes
  # @test:two_agents_whose_slugs_meet_keep_two_ids
  Scenario: The policy is the one agent-allow writes
    Given a held call of one agent and one tool
    When the operator always allows it
    Then wardryx is sent grant_tool, require_identity_source and deny_if_chain_unproven
    And the policy id is grant-<slug>-<hash>, the same id agent-allow gives that agent

  # @test:a_grant_adds_to_the_policy_the_agent_already_holds
  # @test:a_policy_the_agent_holds_is_added_to
  # @test:every_tool_stays_every_tool
  # @test:a_grant_with_no_source_requirement_stays_without_one
  # @test:a_policy_wardryx_will_not_read_writes_nothing_over_it
  Scenario: A grant adds to what the agent holds and takes nothing away
    Given an agent that already holds a tool through agent-allow
    When the operator always allows another
    Then the agent holds both, under the one policy
    But a policy wardryx will not read is not written over

  # @test:an_id_with_no_trust_domain_is_off_and_the_line_says_why
  # @test:a_row_with_no_trust_domain_has_the_button_off_and_the_line_and_grant_still_works
  # @test:an_agent_with_no_trust_domain_reaches_no_policy_and_is_journaled
  # @test:an_id_with_no_domain_is_not_planned_and_the_line_says_why
  # @test:an_id_with_a_trust_domain_is_recognised
  # @test:an_id_with_no_trust_domain_is_refused
  Scenario: An agent id with no trust domain has the action off, with a clear line
    Given a held call whose agent id names no trust domain
    When the operator reads the row
    Then Always allow is off and a line says why and what to do
    And a one-time grant still works
    And the command refuses it too, before anything reaches wardryx

  # @test:a_glob_an_agent_wrote_is_not_a_trust_domain
  # @test:a_glob_in_an_id_a_caller_wrote_is_not_a_grant_to_the_domain
  Scenario: A glob in an agent id is not a grant to the domain
    Given an agent id is whatever the caller wrote
    When a caller writes a star into its id
    Then one click on its hold grants nothing to the domain

  # @test:a_hold_of_any_other_kind_offers_nothing
  # @test:a_hold_on_spend_shows_no_such_button
  # @test:a_hold_of_any_other_kind_is_not_planned
  # @test:a_hold_of_another_kind_reaches_no_policy
  Scenario: A hold no grant lifts offers nothing
    Given a hold on spend, or on a signal
    When the operator reads the row
    Then there is no Always allow, because a grant would not end it

  # @test:a_hold_with_no_one_tool_is_off
  # @test:a_hold_of_several_tools_is_not_planned
  Scenario: A hold with no one tool has nothing to always allow
    Given a hold that lists two tools and no one call
    When the operator reads the row
    Then Always allow is off and says why

  # @test:a_hold_that_came_with_a_static_key_is_not_turned_into_a_refusal
  # @test:a_hold_that_came_with_a_static_key_is_granted_one
  # @test:a_static_key_is_named_before_it_is_granted
  # @test:a_static_key_is_said_on_the_row
  Scenario: A grant does not turn the call it was asked on into a refusal
    Given a held call that came with a static key
    When the operator always allows it
    Then the grant admits a static key, as agent-allow --static-key does
    And the row said so before the button was pressed

  # @test:always_allow_in_the_console_writes_a_policy_and_the_row_says_this_call_still_waits
  # @test:a_row_the_agent_already_holds_says_this_call_still_waits
  # @test:an_agent_that_already_holds_the_tool_is_told_so
  Scenario: The call on the row still waits for a person
    Given an operator who always allowed the tool of a held call
    When the row comes back
    Then it says the agent's next call passes and this one still waits for an answer

  # @test:a_grant_is_on_the_consoles_own_chain_with_its_agent_and_tool
  # @test:a_wardryx_that_will_not_say_what_it_holds_writes_nothing_and_is_journaled
  Scenario: A standing grant is on the console's own record
    Given an operator who always allowed a tool
    When the journal is read
    Then it names the agent, the tool and the hold, under console.grant_tool

  # @test:a_floor_hold_plans_the_tool_of_its_call
  # @test:a_denied_hold_is_no_basis_and_a_granted_one_is
  Scenario: The offer can be taken on a hold a person already granted
    Given the latest of three granted holds
    When the operator takes the offer
    Then the command plans the agent and the tool from that hold
    But a hold a person denied is no basis for it

  # @test:three_grants_of_one_tool_offer_it
  # @test:the_third_grant_of_a_tool_puts_a_row_in_the_inbox_offering_it
  Scenario: Three grants of one tool to one agent put a row in the inbox
    Given a person granted one agent one tool three times
    When the inbox renders
    Then a row offers to always allow it

  # @test:two_grants_offer_nothing
  # @test:the_second_grant_puts_no_row_in_the_inbox
  # @test:a_denial_a_pending_hold_or_a_cost_hold_does_not_count
  # @test:two_agents_or_two_tools_do_not_add_up
  # @test:an_agent_with_no_trust_domain_is_not_offered
  Scenario: Nothing else counts toward the three
    Given two grants, a denial, a hold on spend, or another agent or tool
    When the inbox renders
    Then there is no row

  # @test:a_tool_the_agent_already_holds_is_not_offered_again
  Scenario: A tool the agent holds is not offered again
    Given a stored policy that grants the agent the tool, or every tool
    When the inbox renders
    Then there is no row
