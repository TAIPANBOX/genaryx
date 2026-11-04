Feature: A candidate policy is run against the decisions already taken, and the proof says both halves

  Editing a policy is a blind act. wardryx replay takes the decisions that
  happened and puts each recorded question back to the policy engine, first
  against the version it was decided under, then against a candidate. The
  console runs that CLI and turns its report into a proof for one incident.

  @decided 2026-11-04: a proof reports both halves, whether the candidate would
  have stopped this incident and how many other decisions it would have
  changed, and wardryx's own named outcomes are shown and never folded into a
  count. A proof that could not be made, because an input is missing, the run
  timed out or the report does not hold together, is an error that says so and
  never a proof that nothing changes. The report is text a producer's ids can
  forge, so its counts are checked against its rows. Nothing is applied; a
  person decides, through the existing ceremony.

  # @test:a_report_is_read_as_wardryx_writes_it
  # @test:the_text_of_a_report_is_data
  Scenario: A report is read as wardryx writes it, and its text stays data
    Given the report wardryx documents for a replay against a candidate
    When the console reads it
    Then it holds the counts, the changed decisions, their verdicts, runs and reasons
    And every agent, run and reason in it is carried as data

  # @test:the_incident_is_stopped_only_by_turning_its_own_allow_into_a_deny_or_a_hold
  # @test:a_proof_reports_both_halves_for_an_incident_in_a_real_store
  Scenario: The proof says whether the incident would have stopped and what else would change
    Given a candidate that tightens the incident's run, tightens one other run, loosens another and reclassifies a third
    When the proof is made for the incident
    Then the incident is stopped, by its own allow turning into a deny
    And the other changes are counted as tightened, loosened and reclassified, and listed
    And a candidate that only loosens the incident's own decision stops nothing

  # @test:the_named_outcomes_are_carried_apart_and_never_folded
  Scenario: What could not be replayed is shown by name, never folded into a count
    Given a replay with approval rows, decisions not archived, unreadable decisions and a diverged one
    When the proof is made
    Then each of wardryx's seven outcomes is its own number in the proof

  # @test:a_diverged_replay_proves_nothing
  # @test:a_diverged_replay_exits_non_zero_and_is_still_reported_as_untrustworthy
  Scenario: A replay that disagrees with the record is reported and not relied on
    Given a replay in which a decision diverged from the record, and wardryx exits non-zero
    When the proof is made
    Then the proof is marked untrustworthy and says nothing about whether the incident would stop

  # @test:without_a_candidate_only_replayability_is_known
  # @test:an_empty_history_is_read_as_an_empty_report
  Scenario: With no candidate, or no history, only what can be known is said
    Given a replay asked no candidate question, and a history with no decisions
    When the console reads each
    Then the first says only how much of the history can be replayed
    And the second is an empty report, not an error

  # @test:a_report_that_contradicts_itself_is_refused
  # @test:text_that_is_not_a_report_is_refused_and_a_200_seed_sweep_never_panics
  Scenario: A report that forges a row or does not add up is refused
    Given a run id that prints a row nobody replayed, outcomes that do not sum, and text that is no report
    When the console reads each
    Then each is refused, and a 200-seed sweep of hostile text never panics or reads an inconsistent report

  # @test:the_proof_runs_wardryx_replay_with_exactly_its_arguments_and_removes_the_candidate
  # @test:the_child_gets_no_environment_but_a_path
  # @test:a_replay_that_hangs_is_killed_at_its_deadline
  Scenario: The CLI runs bounded, with no shell and nothing of the console's environment
    Given a candidate policy and a stand-in wardryx that records how it was run
    When the proof is made
    Then wardryx gets exactly the replay arguments, the files the console chose, and an environment of PATH alone
    And it reads the operator's text from a private file that is removed when the run ends
    And a replay that hangs is killed at its deadline

  # @test:an_unconfigured_proof_names_what_is_missing_and_is_never_a_proof
  # @test:a_candidate_that_cannot_be_given_is_refused_before_anything_runs
  # @test:an_incident_with_no_run_or_no_row_is_refused
  # @test:a_replay_that_fails_without_a_report_says_so
  # @test:a_missing_binary_is_a_spawn_error
  Scenario: A proof that could not be made says why and is never a proof of nothing
    Given no archive, an empty or oversized candidate, an incident with no run, a failing replay and a missing binary
    When the proof is asked for
    Then each is an error that names what was missing, and wardryx is not started for the ones refused first

  # @test:proving_a_policy_is_a_viewer_read
  Scenario: Proving a policy is a read any operator may make
    Given the command table of the web shell
    When it is asked about the policy proof command
    Then it needs the viewer role and no passkey ceremony

  # @test:slots_hand_out_exactly_their_limit
  # @test:a_released_slot_is_taken_again
  # @test:a_limit_of_zero_refuses_everything
  # @test:two_hundred_threads_never_hold_more_than_the_limit
  # @test:a_busy_slot_is_an_answer_not_a_queue
  Scenario: Only a few proofs run at once, and a full house is an answer, not a queue
    Given every proof slot in the console process is taken
    When another operator asks for a proof
    Then the answer is a busy error naming the limit, at once, and no child process is started
    And when a slot is given back the next ask is served
    And many threads taking and giving slots never hold more than the limit and leak none
