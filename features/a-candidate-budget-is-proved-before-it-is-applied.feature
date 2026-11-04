Feature: A candidate budget is replayed over the calls already made, and the proof says both halves

  Setting a budget after an incident is a guess about the next one. tokenfuse
  backtest replays a candidate budget or step limit over the calls the gateway
  recorded and says what it would have blocked and saved. The console runs that
  CLI and turns its output into a proof for one incident.

  @decided 2026-11-04: a proof reports both halves, whether the candidate would
  have stopped this incident and what else it would have blocked, and a proof
  that could not be made, because an input is missing, the run is not in the
  trace, a child timed out or the output does not hold together, is an error
  that says so and never a proof that nothing changes. The CLI prints totals
  over the whole trace and nothing per run, so the incident's own calls are
  copied into a trace of their own and the same CLI replays that. Nothing is
  applied; a person decides, through the existing ceremony.

  # @test:amounts_are_whole_micro_dollars_or_refused
  # @test:a_candidate_names_what_it_limits_and_hands_over_only_what_it_can
  # @test:a_candidate_that_sets_no_limit_or_a_bad_amount_is_refused_before_anything_runs
  Scenario: A candidate is a set of limits in whole micro-dollars, or it is refused
    Given a candidate with no limit, a negative or fractional-micro amount, an exponent, a zero limit and a step limit of zero
    When the proof is asked for
    Then each is refused with the reason, and no child process is started

  # @test:an_amount_is_handed_to_the_cli_as_text_it_reads_back_exactly
  # @test:a_candidate_is_handed_over_as_the_amount_the_cli_will_read
  Scenario: The CLI is handed the amount that was asked for, not one micro-dollar less
    Given the CLI reads a flag as a float times a million, cut to a whole number
    When a budget of 0.000249 dollars is proved
    Then the text handed over reads back as 249 micro-dollars, and not 248

  # @test:the_proof_runs_four_children_in_order_over_the_right_directories_and_removes_the_copy
  Scenario: The incident's run is copied out and replayed alone, then the whole trace is replayed
    Given a run in the gateway's trace
    When the proof is made
    Then the run is copied into a private directory, counted, replayed there, and the whole trace is replayed
    And every child has an environment of a path and the directory it was pointed at, and no shell
    And the copy is gone when the proof ends

  # @test:the_incident_is_stopped_only_when_its_own_run_has_a_call_blocked
  # @test:the_rest_of_the_trace_is_the_whole_trace_less_the_incident
  # @test:a_budget_proof_reports_both_halves_for_an_incident_in_a_real_store
  # @test:a_candidate_that_blocks_nothing_in_the_run_but_something_elsewhere_stops_nothing
  Scenario: The proof says whether the incident would have stopped and what else would be blocked
    Given a candidate that blocks eighteen of the incident's thirty-five calls and fifteen runs in all
    When the proof is made for the incident
    Then the incident is stopped, with what it would have spent and saved
    And the rest of the trace is the whole trace less the incident
    And a candidate that blocks calls elsewhere and none of the incident's own stops nothing

  # @test:a_run_with_calls_at_the_same_step_marks_the_split_approximate
  # @test:calls_at_the_same_step_make_the_split_approximate
  Scenario: Calls at the same step make the split between the incident and the rest approximate
    Given a run whose calls share steps, which the whole-trace replay loads in an order the CLI does not promise
    When the proof is made
    Then the numbers are given and the split is marked as approximate

  # @test:the_report_is_read_as_backtest_prints_it
  # @test:a_backtest_report_for_another_candidate_is_refused
  # @test:a_backtest_report_that_contradicts_itself_is_refused
  # @test:text_that_is_not_a_backtest_report_is_refused_and_a_200_seed_sweep_never_panics
  Scenario: A report is read as the CLI prints it and refused when it is not for this candidate or does not add up
    Given the report the CLI prints, one that ran another candidate, ones whose counts or projection contradict themselves, and text that is no report
    When the console reads each
    Then the first holds the counts and the amounts in micro-dollars
    And each of the others is refused, and a 200-seed sweep of hostile text never panics

  # @test:reports_that_disagree_with_each_other_are_refused
  # @test:reports_that_cannot_describe_the_same_trace_are_refused
  Scenario: Reports that cannot be about the same trace are refused
    Given a copy and a count that disagree, a replay of another number of calls, and a whole trace smaller than the run
    When the proof is made
    Then it is refused, and no number from those reports is believed

  # @test:a_run_the_trace_does_not_hold_is_not_a_proof_of_nothing
  # @test:an_empty_trace_is_no_calls_and_not_a_report
  Scenario: A run the trace does not hold is an answer, not a proof that nothing changes
    Given an incident whose run has no calls in the gateway's trace
    When the proof is made
    Then the answer is that the trace does not hold the run, and nothing else is run

  # @test:a_run_id_is_put_in_a_query_only_when_it_is_a_plain_name
  # @test:a_run_id_that_could_break_the_query_never_reaches_it
  Scenario: A run id that could break out of the query never reaches it
    Given a run id with a quote, a space or a semicolon in it
    When the proof is made
    Then it is refused by name and no child is started

  # @test:a_table_is_read_as_tokenfuse_sql_prints_it
  # @test:a_child_that_exits_zero_with_an_error_of_its_own_is_not_a_proof
  Scenario: A CLI that exits zero with an error of its own is not a proof
    Given tokenfuse prints an error and exits zero, as it does
    When the proof is made
    Then the answer is the child's own failure, and the output that is no table is refused

  # @test:an_input_that_is_missing_is_named_and_nothing_runs
  Scenario: A missing input is named
    Given a box with no trace directory, and an incident with no run
    When the proof is asked for
    Then each is an answer that says what is missing, and no child is started

  # @test:all_the_children_of_one_proof_share_one_deadline
  # @test:a_budget_proof_meets_a_busy_slot_as_an_answer_not_a_queue
  Scenario: A proof is bounded in time and in how many run at once
    Given children that each take a second and a proof that has two, and a slot shared with every other proof
    When the proof is made
    Then it times out at its one deadline, and a proof asked for while every slot is taken is answered busy at once

  # @test:proving_a_budget_is_a_viewer_read
  Scenario: Proving a budget is a read
    Given a viewer
    When they ask for a budget proof
    Then it is allowed, with no ceremony, because it applies nothing
