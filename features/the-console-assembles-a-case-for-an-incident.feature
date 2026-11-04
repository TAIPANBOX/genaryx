Feature: The console assembles one case for an incident, with no model, and says what it could not reach

  Detection, correlation, replay, signing and sealing each live in their own
  service, and nothing joins them for the person who has to decide what to do
  about an incident. Slice 1 of the incident-to-proven-fix plan adds the join:
  a case file, assembled from reads the console already makes, deterministically
  and with no model in the path. Later slices let Felyx read the case, propose
  a remedy and prove it; none of that is here.

  @decided 2026-11-04: assembly is deterministic and names what is missing. A
  plane that did not answer is a line in the case, never an absence, because a
  missing line reads as "nothing to report" and that is the silent-zero fault
  this console already refuses elsewhere (invariants 4 and 8). Each fact in a
  case carries the plane that gave it, that plane's own id for the row, and a
  digest of the row. An identity refusal is filed under the key that made the
  call (invariant 19), so no question about an agent is asked under the agent
  the refusal merely claimed. Text that came from events, logs and memory stays
  marked as data in the case, and nothing in a case is an instruction.

  # @test:every_fact_carries_its_plane_its_row_and_a_digest
  # @test:a_case_names_every_question_it_asks_in_a_fixed_order
  Scenario: Every fact says which plane gave it, which row it is, and what the row hashed to
    Given every plane answered with a row for the incident
    When the console assembles the case
    Then each fact names its plane and the id that plane gave the row
    And each fact carries the sha256 digest of the row's own content
    And the case holds one line per question, in a fixed order

  # @test:a_plane_that_did_not_answer_is_a_line_in_the_case
  # @test:a_case_with_no_answers_at_all_still_has_a_line_for_every_section
  Scenario: A plane that did not answer is a line in the case, not an absence
    Given the identity plane could not be reached
    And the policy plane was never asked
    When the console assembles the case
    Then the case has a line for the identity question saying the plane did not answer
    And the case has a line for the policy question saying why it was not asked
    And no question is missing from the case, even when nothing answered at all

  # @test:the_subject_agents_identity_and_blast_radius_come_from_idryx
  # @test:wardryx_gives_the_policy_status_and_the_approvals_of_the_run
  # @test:an_agent_idryx_does_not_know_is_an_empty_answer_not_a_failure
  Scenario: Idryx and Wardryx give the subject's identity and blast radius, the policy status and the run's approvals
    Given idryx holds the subject with an admin permission and an identity that acts through it
    And wardryx runs a policy version and holds two approvals of this run and one of another
    When the console assembles the case
    Then the identity section holds the record and the alerts on this subject only
    And the blast radius counts the permissions and names who acts through the subject
    And the policy section names the version and the decisions section the two approvals of this run
    And an agent idryx does not know is an empty answer, not a failure

  # @test:a_plane_that_answered_with_no_rows_is_empty_and_not_unavailable
  Scenario: A plane that answered with nothing is told apart from one that did not answer
    Given the memory plane answered and holds no prior ruling for this kind of incident
    When the console assembles the case
    Then the case line for prior rulings says the plane answered and had nothing
    And that line is not the one a plane that failed to answer gets

  # @test:a_digest_is_the_sha256_of_the_row_and_changes_with_it
  # @test:the_same_inputs_assemble_the_same_case_and_the_same_case_digest
  Scenario: The digests are real, and the same inputs give the same case
    Given a row and the same row with one field changed
    When the console assembles a case from each
    Then the two facts have different digests
    And the digest of the first is the sha256 of its content as written
    And assembling the first twice gives byte-identical cases with the same case digest

  # @test:an_identity_refusal_case_is_filed_under_its_key
  # @test:a_refusal_asks_no_plane_about_the_claimed_agent
  # @test:an_identity_refusal_incident_is_filed_under_its_key_end_to_end
  # @test:an_identity_refusal_asks_idryx_nothing_at_all
  Scenario: An identity refusal is filed under the key and no agent question is asked about the agent it claimed
    Given the key forge-imposter was refused while claiming to be flint
    When the console works out what to ask for the incident
    Then the incident is filed under key:forge-imposter
    And the claim is kept as a claim, naming flint
    And the identity and blast radius questions are not asked, and the case says why
    And nothing in the case is looked up under flint

  # @test:text_inside_an_event_is_carried_as_data_and_never_as_the_label
  # @test:an_event_that_tells_the_reader_to_obey_it_is_still_only_a_row
  Scenario: Words written by an event or a memory never become the console's own words
    Given an event whose own text tells the reader to ignore its instructions
    When the console assembles the case
    Then the text appears only inside the row of its fact
    And the fact is marked as data
    And the label of every fact and line is the console's fixed wording

  # @test:the_case_for_a_bus_incident_reads_its_event_the_run_timeline_and_similar_past_incidents
  # @test:an_event_is_read_back_by_its_id
  # @test:similar_events_are_the_earlier_events_of_the_same_type_newest_first
  Scenario: The console's own store gives the event, the run's timeline in time order, and similar past incidents
    Given the bus holds an event with a run, earlier events of the same type, and the rest of that run
    When the operator asks for the case of the event
    Then the case holds the event row
    And the run's events in the order they happened, whichever source wrote them first
    And the earlier events of the same type, newest first, never the event itself

  # @test:a_case_with_no_reachable_plane_names_each_one
  # @test:a_run_the_cloud_knows_gives_the_case_its_spend_row
  Scenario: With every other plane unreachable the case still comes back and names each one
    Given the console's store holds the incident and no other plane is reachable
    When the operator asks for the case
    Then the case holds what the store gave
    And one line per unreachable plane says it did not answer

  # @test:an_incident_id_names_an_event_row_or_is_refused
  # @test:an_incident_that_is_not_an_event_row_is_refused_by_name
  # @test:an_unknown_event_row_is_refused_not_emptied
  Scenario: An incident the case cannot start from is refused, and the refusal says which kind
    Given an incident id from money, identity or posture, or an id the store does not hold
    When the operator asks for its case
    Then the answer is a refusal naming the source or the missing row
    And it is never an empty case

  # @test:reading_a_case_is_a_viewer_read
  Scenario: Reading a case is a read any operator may make
    Given the command table of the web shell
    When it is asked about the case command
    Then the case command needs the viewer role
    And it needs no passkey ceremony

  # @test:the_case_view_lists_what_was_gathered_and_what_is_missing
  # @test:the_case_view_says_in_words_why_a_plane_is_missing
  Scenario: Incident 360 shows the case, and what is missing from it, in words
    Given a case with facts from three planes, one plane that did not answer and one that was not asked
    When the operator opens the case in Incident 360
    Then each answered question shows how many facts it holds and from which plane
    And the plane that did not answer is listed with the console's own sentence for it
    And the question that was not asked says why

  # @test:the_case_view_prints_event_text_as_data
  Scenario: The case view prints text from events as data
    Given a case whose event row holds markup and an instruction
    When Incident 360 renders the case
    Then the text is escaped and shown inside the data frame of its fact
    And it is not used for any sentence the console says itself

  # @test:the_demo_assembles_a_case_for_an_identity_refusal_on_invented_rows
  # @test:the_demo_case_includes_a_plane_that_did_not_answer
  # @test:the_demo_refuses_an_incident_with_no_event_row_by_name
  Scenario: The demo shows a case on invented data, including a plane that did not answer
    Given the demo build with no box behind it
    When the operator asks for the case of the demo identity refusal
    Then the case is filed under the demo key and not under the claimed agent
    And one plane in it did not answer
    And every row in it comes from the demo transport's own rows
