Feature: The console says which of three ways a hold was closed

  Wardryx closes a hold in three ways and no others: a person decides it (grant
  or deny), wardryx closes it as expired once it has waited its lifetime and
  nobody decided, or the held agent withdraws it. The console knew two outcomes
  and painted everything that was not a grant as a refusal, so a hold nobody
  answered, or one its agent took back, read as a person saying no.

  @decided 2026-11-14: an expired hold reads "Expired, nobody decided in time"
  and a withdrawn one "Withdrawn by the agent", both in a neutral tone that is
  neither the refusal red nor the grant green, wherever a decision is shown: the
  inbox history, the agent card, and the event feed with how long the hold waited.
  A decision value this build does not know reads "Closed (value)" and the page
  keeps rendering. What a person can send stays grant or deny, and the console
  offers nobody, an administrator included, a way to cancel or close a hold.

  # @test:an_expired_hold_reads_as_expired_and_nobody_decided_in_time
  # @test:the_history_row_shows_an_expired_hold_neutral_with_the_system_as_decider
  Scenario: A hold nobody decided in time reads as expired, not as a refusal
    Given a hold that wardryx closed as expired after its lifetime
    When the operator opens the approvals history
    Then the row reads "Expired, nobody decided in time"
    And it is neither the refusal red nor the grant green
    And it names wardryx as the one that closed it

  # @test:a_withdrawn_hold_reads_as_withdrawn_by_the_agent
  # @test:the_history_row_shows_a_withdrawn_hold_neutral_with_the_agent_as_decider
  Scenario: A hold its agent took back reads as withdrawn by the agent
    Given a hold that its own agent withdrew
    When the operator opens the approvals history
    Then the row reads "Withdrawn by the agent"
    And it is neither the refusal red nor the grant green

  # @test:expired_and_withdrawn_are_neither_the_refusal_red_nor_the_grant_green
  # @test:a_grant_and_a_deny_read_as_before
  Scenario: A grant stays green and a person's refusal stays red
    Given holds that a person granted and denied
    When the operator reads them next to expired and withdrawn ones
    Then only the grant is green and only the denial is red

  # @test:the_badge_on_the_agent_card_and_the_one_in_the_inbox_are_one_component
  Scenario: The agent card says the same thing as the inbox
    Given an expired hold on an agent
    When the operator opens the agent card
    Then its approvals list uses the same reading as the inbox history

  # @test:an_unknown_decision_reads_as_closed_with_its_value_and_is_neutral
  # @test:a_closed_hold_with_no_decision_is_neutral_and_never_a_refusal
  # @test:the_history_row_closes_an_unknown_value_as_closed_and_keeps_rendering
  # @test:two_hundred_seeds_of_hostile_decision_values_never_throw_and_never_turn_red_or_green
  Scenario: A decision this build has never heard of reads as closed and breaks nothing
    Given a closed hold whose decision value is one this console does not know
    When the operator opens the approvals history
    Then the row reads "Closed" with that value in brackets
    And it is neutral and the other rows are unchanged

  # @test:an_approval_expired_event_says_how_long_it_waited_and_the_limit
  # @test:an_approval_withdrawn_event_says_the_agent_withdrew_it_and_how_long_it_waited
  # @test:the_expanded_event_row_and_the_detail_card_show_the_closure_line
  Scenario: The event feed says how long a closed hold waited
    Given an approval_expired or an approval_withdrawn event on the bus
    When the operator expands the row or opens its detail card
    Then it says expired or withdrawn and how long the hold waited, with the limit for an expiry

  # @test:other_events_have_no_closure_line
  # @test:a_closure_line_survives_missing_hostile_and_negative_numbers
  Scenario: A missing or hostile number never breaks the line and other events gain none
    Given an approval_expired event whose numbers are absent, negative, not finite or not numbers
    When the operator reads it
    Then it still says expired and prints none of those numbers
    And an event of any other type gets no such line

  # @test:the_mail_link_for_an_expired_or_withdrawn_hold_opens_the_policy_panel
  Scenario: An alert mail about a closed hold opens the policy panel
    Given a mail link for an approval_expired or an approval_withdrawn
    When the operator follows it
    Then the policy panel opens

  # @test:the_decide_action_takes_only_grant_and_deny
  # @test:no_control_in_the_inbox_cancels_or_closes_a_hold
  Scenario: A person can still only grant or deny, and nobody can cancel a hold
    Given the approvals inbox in every role
    When the operator looks for a way to close a hold by hand
    Then there is none, and the decision a person sends is grant or deny only
