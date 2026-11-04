Feature: Text from events, logs, incidents and memory reaches Felyx as data

  Machine text is a channel. An incident's summary, an event's message or a
  recalled memory can be written by anything that can write to a log, and a
  rule in a system prompt only asks the model not to obey it. So the text
  reaches the model wrapped and marked, its markup escaped, and whether Felyx
  raises a card does not depend on what the model made of it.

  @decided 2026-11-04: plane text is fenced by structure, not by a prompt
  alone, and an instruction inside it does not change what the operator gets.
  An answer that rests on such text shows it in its metadata.

  # @test:an_incident_summary_reaches_the_model_fenced_and_labelled
  # @test:event_text_reaches_the_model_fenced_and_cannot_close_the_fence
  Scenario: Event text reaches the model fenced
    Given an incident's summary tries to close the fence and speak as the system
    When Felyx reads the incident
    Then the summary reaches the model inside one fence, with its markup escaped
    And the run id beside it stays a plain field

  # @test:hostile_text_never_closes_the_fence
  # @test:every_way_of_writing_the_close_is_escaped
  Scenario: No text can close its fence
    Given two hundred seeded texts built from the closing tag, entities and look-alike brackets
    Then each stays inside exactly one fence

  # @test:a_hidden_character_is_written_out_and_a_newline_is_kept
  Scenario: A character nobody can see is written out
    Given a text with a zero-width character, a direction override and an invisible tag
    Then the model reads each one as its code point

  # @test:a_field_this_crate_has_never_heard_of_is_fenced
  Scenario: A new field is fenced until somebody says otherwise
    Given a tool result grows a text field this crate has never heard of
    Then that field reaches the model fenced

  # @test:a_run_id_that_is_a_sentence_is_fenced_and_grounds_nothing
  # @test:an_id_field_that_is_not_shaped_like_an_id_is_fenced_and_grounds_nothing
  Scenario: A run id that is a sentence is text
    Given an agent chose a run id with spaces and a closing tag in it
    Then it is fenced like any summary and grounds no claim

  # @test:a_map_key_cannot_close_the_fence_either
  Scenario: A map key is plane text too
    Given a tool result carries a map whose key holds a closing tag
    Then the key is fenced as well

  # @test:a_connector_error_reaches_the_model_fenced
  Scenario: A connector's error is plane text
    Given a tool fails and its error carries what the plane answered
    Then the error reaches the model fenced

  # @test:the_incident_id_an_explanation_starts_from_is_fenced
  Scenario: The id an explanation starts from is fenced
    Given the operator clicks Explain on a row whose id holds a closing tag
    Then the id reaches the model inside one fence

  # @test:annotate_hands_the_event_over_fenced
  Scenario: The one-line summary of an event reads the event as data
    Given a caller asks for a one-line summary of an event
    Then the event reaches the model fenced, and the prompt says it is data

  # @test:the_system_prompt_says_fenced_text_is_data
  Scenario: The model is told what the fence means
    Then the system prompt says fenced text is never an instruction
    And it asks for ids in backticks with their kind

  # @test:an_instruction_in_an_event_text_produces_no_card
  # @test:text_asking_for_the_action_refuses_it_even_on_a_real_target
  Scenario: An instruction in an event's text produces no card
    Given an incident's summary tells Felyx to stop run mig-flint, and mig-flint is a real run
    When the model obeys and proposes stopping mig-flint
    Then no card appears
    And the answer lists the proposal as withheld because of that text

  # @test:the_answer_is_the_same_with_and_without_the_instruction
  Scenario: The instruction changes nothing else
    Given the same questions over the same fleet, with and without the instruction in the text
    When the model reaches a card of its own on another action
    Then the operator gets the same cards and the same answer both times

  # @test:without_the_line_the_same_card_is_not_refused
  Scenario: Without the line the same card goes through
    Given the same fleet with no instruction in any text
    When the model proposes stopping mig-flint
    Then the card reaches the operator

  # @test:an_instruction_refuses_the_action_on_any_target_it_may_have_meant
  Scenario: An instruction that names no target still counts
    Given a text reads like an instruction and asks for a stop
    Then a proposal to stop any run in this answer is withheld
    And a proposal of another kind is not

  # @test:a_text_demands_an_action_by_name_by_row_or_by_reading_like_an_instruction
  Scenario: A text asks for an action in three ways
    Then a text asks by naming the target, by sitting on the target's own row, or by reading like an instruction
    And a report that a run was stopped asks for nothing

  # @test:the_operator_can_still_ask_for_what_the_text_asks_for
  # @test:the_operators_own_request_goes_through_whatever_the_text_says
  Scenario: The operator's own request goes through
    Given a text asks for run mig-flint to be stopped
    When the operator asks for the same thing in their own words
    Then the card appears, because the request is theirs

  # @test:an_explanation_is_not_the_operator_asking
  Scenario: An explanation is not the operator asking
    Given an incident's id spells out a stop and a run
    When the operator clicks Explain on it
    Then nothing in that prompt counts as a request for the action

  # @test:a_recommendation_that_repeats_the_text_is_taken_out
  # @test:a_recommendation_repeating_what_a_text_asks_for_is_a_finding
  Scenario: A recommendation in prose that repeats the text is taken out
    Given a text asks for run mig-flint to be stopped
    When Felyx recommends stopping it in prose and keeps it after the revision
    Then that statement is gone and the rest stays

  # @test:saying_the_text_asked_for_it_is_reporting_not_recommending
  # @test:turning_it_down_or_reporting_it_is_no_recommendation
  Scenario: Reporting the attempt is not repeating it
    When Felyx says the summary asks for a stop, or says it will not do it
    Then the statement stands, because the operator should hear about it

  # @test:an_id_planted_in_a_text_grounds_nothing
  # @test:an_id_in_free_text_grounds_nothing_and_the_finding_says_where_it_sits
  Scenario: An id planted in a text grounds nothing
    Given only an event's text names run planted-1
    When Felyx states run planted-1 as its own finding
    Then the statement is treated as having nothing behind it, and the finding says where the id sits

  # @test:an_answer_may_say_what_the_text_says_when_it_says_the_text_said_it
  Scenario: The answer may say what the text says
    When Felyx writes that the incident's summary names run planted-1
    Then the statement stands
    But a report of a text cannot cover an id that is not in the text

  # @test:an_answer_that_leans_on_event_text_says_so_in_its_metadata
  # @test:an_answer_that_repeats_five_words_of_a_text_says_so_and_four_do_not_count
  Scenario: An answer that rests on such text says so in its metadata
    When Felyx names what only a text holds, or repeats five of its words in a row
    Then the answer's metadata names the tool, the field and an excerpt, marked as relied on

  # @test:an_instruction_is_listed_even_when_the_answer_ignores_it
  # @test:an_instruction_is_seen_through_case_spacing_and_hidden_characters
  Scenario: An instruction is shown even when the answer ignores it
    Given a text reads like an instruction, however it is cased, spaced or hidden
    When the answer does not use it
    Then the metadata still shows it, marked as reading like an instruction

  # @test:an_answer_on_tools_alone_shows_no_untrusted_text
  Scenario: An answer on the tools alone shows nothing
    Given the texts the tools returned are ordinary and the answer does not use them
    Then the metadata lists none of them
