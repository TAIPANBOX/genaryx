Feature: Felyx points only at what a tool returned in the same answer

  An operator acts on a cited id. A root-cause chain through a run nobody can
  find sends them looking for it, and a card built on one asks them to sign for
  it. The check on budget statements read what a draft says about a budget; this
  one reads what the draft points at: run, incident, policy, agent and approval
  ids, and dollar amounts. The check is deterministic, and it holds a structured
  field of a tool result as evidence, never the free text beside it.

  @decided 2026-11-04: every id and amount in an answer must be in the tool
  results of that same answer. A claim with nothing behind it is taken out or
  plainly marked, and an explanation never gives a chain through an invented id.

  # @test:an_id_a_tool_returned_costs_no_extra_call
  # @test:every_structured_id_a_tool_returned_passes
  # @test:an_agent_is_named_by_the_last_part_of_its_id
  Scenario: An id the tools returned costs nothing
    Given the tools returned run p1-brume and its incident
    When Felyx drafts an answer citing both, with the spend the tool returned
    Then the answer goes out as drafted, with no second call to the model

  # @test:an_invented_run_id_is_sent_back_once
  # @test:an_invented_id_of_every_kind_is_a_finding_naming_it
  Scenario: An invented id is sent back once
    Given no tool returned run p1-phantom
    When Felyx drafts a statement about run p1-phantom
    Then the draft goes back once, naming p1-phantom
    And the corrected answer is the one the operator gets

  # @test:an_invented_run_id_that_stays_is_taken_out_and_named_in_a_note
  # @test:only_the_unsupported_statement_is_taken_out
  # @test:an_answer_with_nothing_left_is_the_note_alone
  Scenario: An invented id that stays is taken out
    Given Felyx drafts the same statement about run p1-phantom after the revision
    Then that statement is gone from the answer and the rest stays as written
    And a note on the answer says what was removed
    And the same finding is listed in the answer's metadata

  # @test:an_invented_policy_agent_or_approval_id_is_sent_back_and_a_real_one_is_not
  Scenario: Policy, agent and approval ids are held the same way
    Given the tools returned policy deny-shell and approval ap-77
    When Felyx drafts a statement naming policy deny-phantom, approval ap-78 and an agent no tool returned
    Then the draft goes back naming all three and none of the real ones

  # @test:an_incident_id_that_spells_budget_is_no_budget_statement
  Scenario: An incident id that spells budget is not a statement about a budget
    Given a run opened incident budget_exhausted:p1-brume
    When Felyx cites that id without having read the budgets
    Then the budget check has nothing to say about it

  # @test:a_longer_id_containing_a_real_one_is_still_invented
  Scenario: An id is matched whole
    Given the tools returned run p1-brume
    When Felyx drafts a statement about p1-brume-2
    Then p1-brume-2 counts as invented

  # @test:saying_no_tool_returned_an_id_is_the_honest_answer_and_passes
  # @test:saying_a_thing_was_not_found_is_not_claiming_it
  Scenario: Saying a thing was not found passes
    Given the operator asks about a run no tool returned
    When Felyx answers that no tool returned it
    Then the answer goes out as drafted

  # @test:an_amount_a_tool_returned_passes_rounded_and_an_invented_one_is_taken_out
  # @test:a_tool_amount_supports_itself_and_its_rounding_and_nothing_else
  Scenario: An amount must be one a tool returned, as rounded as the draft wrote it
    Given a tool returned a spend of $0.002008
    When Felyx writes $0.0020
    Then the statement stands
    When Felyx writes $0.0300, which no tool returned, and keeps it after the revision
    Then that statement is taken out and the rest stays

  # @test:rounding_to_zero_is_not_support_but_a_real_zero_is
  Scenario: Rounding down to nothing is not support
    Given a tool returned a spend of $0.002 and no zero anywhere
    When Felyx writes $0
    Then the amount has nothing behind it

  # @test:an_amount_in_the_operators_question_is_theirs_to_repeat
  Scenario: The operator's own number may be repeated
    Given the operator asks whether anything is above $50
    When Felyx answers that nothing is above $50
    Then the answer stands

  # @test:an_explanation_of_an_incident_no_tool_returned_gives_no_chain
  # @test:an_explanation_that_never_read_its_incident_gives_no_chain
  Scenario: An explanation of an incident no tool returned gives no chain
    Given no tool returned incident budget_exhausted:p1-phantom
    When the operator asks Felyx to explain it and the draft gives a chain anyway
    Then the operator gets no chain, and a sentence saying why
    And the finding names the incident

  # @test:an_explanation_citing_an_invented_run_gives_no_chain_at_all
  Scenario: A chain with an invented link is withheld whole
    Given Felyx explains a real incident and one link of the chain names an invented run
    When the link survives the revision
    Then the whole chain is withheld, because a chain with a link taken out reads as a shorter chain

  # @test:an_explanation_resting_on_what_the_tools_returned_keeps_its_chain
  Scenario: A chain the tools support goes out as drafted
    Given the tools returned the incident and every id the chain cites
    When Felyx gives the chain
    Then the operator reads it as drafted, with no second call to the model

  # @test:cards_on_an_explanation_of_no_incident_are_withheld
  Scenario: A card from an explanation of no incident is withheld
    Given Felyx raised a card while explaining an incident no tool returned
    Then the card does not reach the operator, and the answer says why

  # @test:a_card_on_a_run_a_tool_returned_reaches_the_shell
  # @test:a_proposal_on_a_run_a_tool_returned_passes
  Scenario: A card on a run a tool returned reaches the operator
    Given a tool returned run p1-brume
    When Felyx proposes stopping it
    Then the operator gets the card, to approve and sign themselves

  # @test:a_card_on_a_run_no_tool_returned_is_withheld_and_the_model_is_told
  # @test:a_target_must_have_been_returned_as_that_kind_of_thing
  Scenario: A card on a run no tool returned is withheld
    Given no tool returned run p1-phantom
    When Felyx proposes stopping it
    Then no card appears, the answer lists it as withheld with the reason
    And the model is told, so it does not report a card that is not there

  # @test:evidence_no_tool_returned_leaves_the_card_and_is_named
  # @test:evidence_no_tool_returned_is_dropped_from_the_card_and_said
  Scenario: Evidence no tool returned leaves the card
    Given a card cites one real run and one invented incident
    Then the card keeps the real one and the answer names the other

  # @test:a_rationale_naming_what_no_tool_returned_refuses_the_card
  Scenario: A rationale naming what no tool returned refuses the card
    Given a card's rationale names a run no tool returned
    Then the card is withheld

  # @test:hostile_text_never_panics_and_the_ranges_stay_whole
  Scenario: Hostile drafts never break the check
    Given two hundred seeded drafts of ids, amounts, punctuation and multibyte text
    Then the check finishes on each, and what it splits it can put back byte for byte
