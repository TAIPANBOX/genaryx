Feature: Felyx reads the case file the console assembled, instead of running a tool loop to gather it

  Felyx used to explain an incident by calling tools until it believed it had
  enough. The console now assembles one case file for an incident, from its own
  store and from every plane that has something about it, and each statement in
  it names its plane, its row and a digest. Felyx reads that, and only that.

  @decided 2026-11-04: reading a case is not a tool loop. The model is offered
  no tool, and a tool it asks for anyway is refused and never run. The case
  reaches it fenced by position: the console's own words in the case stay plain,
  every string inside a plane's row is data whatever the plane named the field,
  and an id found only in such text grounds nothing. A chain that cites a run,
  an approval or an incident the case does not hold is withheld whole. A plane
  the case says did not answer is a gap the answer must name, never a thing the
  answer describes. Nothing is proposed and nothing is applied; a person decides.

  # @test:felyx_reads_the_case_it_is_given_and_asks_for_no_tool
  # @test:a_model_that_asks_for_a_tool_while_reading_a_case_gets_none
  Scenario: The model is given the case and no tool, and a tool it asks for is refused
    Given a case file for an identity refusal, with every plane that answered
    When Felyx explains the incident
    Then the model is given the case with its lines, its row ids and its digests
    And it is offered no tool
    And no plane is asked anything
    And a tool it asks for anyway is not run, and it hears that it has none

  # @test:a_run_the_case_does_not_hold_is_not_cited_and_the_chain_is_withheld
  # @test:a_case_with_no_incident_id_gives_no_chain
  Scenario: A chain through something the case does not hold is withheld whole
    Given a draft that names a run the case does not hold
    When Felyx has had one chance to correct it and still names it
    Then the operator gets no root-cause chain, and the finding names the run
    And a case with no readable incident id gives no chain either

  # @test:plane_text_in_a_case_row_reaches_the_model_fenced_and_grounds_nothing
  Scenario: Text a plane wrote reaches the model as data, and an id found only there grounds nothing
    Given an event row whose own text tries to close the fence and plants a run id
    And a plane row that carries a reason field of its own
    When Felyx explains the incident
    Then the model reads that text inside a fence it cannot close
    And the console's own reasons for the case's lines are plain
    And a draft that states the planted run as a fact is withheld

  # @test:the_key_a_refusal_is_filed_under_is_citable_without_its_prefix
  Scenario: The key an identity refusal is filed under can be cited by its name
    Given an identity refusal filed under key:forge-imposter
    When Felyx names the key as forge-imposter
    Then the answer stands, because the case holds that key

  # @test:explain_case_on_a_disabled_service_is_no_provider
  # @test:a_refusal_describes_itself_in_words
  # @test:felyx_reading_a_case_is_a_viewer_read
  Scenario: With no provider, or no event row, the answer is a sentence, and reading is a viewer read
    Given a console with no copilot provider configured
    When an operator asks Felyx to read a case
    Then the answer says there is no provider, and never a fabricated chain
    And an incident with no event row is refused in words before the model is called
    And the command is a viewer read with no ceremony

  # @test:the_case_view_offers_felyx_and_prints_its_answer_as_text
  # @test:a_source_with_no_event_row_does_not_offer_felyx
  # @test:with_no_box_to_ask_felyx_says_so_and_invents_no_answer
  Scenario: Incident 360 offers Felyx on a case and prints what it says as text
    Given the Case section of an incident with an event row
    When the operator opens it
    Then there is a button to ask Felyx to read the case
    And the answer is printed as escaped text, with what it could not support listed and its limits said once
    And a money, identity or posture incident does not offer Felyx
    And with no box behind the console there is no answer to show

  # @test:the_demo_felyx_reading_cites_only_ids_the_demo_case_holds
  # @test:the_demo_refuses_to_read_an_incident_with_no_event_row
  Scenario: The demo shows Felyx reading a demo case, citing only ids the case holds
    Given the demo build with no box behind it
    When the operator asks Felyx to read the demo identity refusal
    Then the reading names the key, says what is missing from the case, and proposes nothing
    And every id in it is one the demo case holds
    And an incident with no event row is refused in the real command's words
