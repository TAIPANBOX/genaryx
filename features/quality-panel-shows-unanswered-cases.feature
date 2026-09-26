Feature: The Quality panel shows unanswered cases beside every mean

  Verdryx (2bfd3c8) can refuse a case rather than scoring it, and records why
  in its own `unanswered` table. Before this, the Quality panel's mean score
  said nothing about that: a run that answered half its cases and a run that
  answered all of them could show the same number.

  @decided 2026-09-26: a partly-unanswered run reads "mean 0.82 over 57
  answered, 3 unanswered (label_mass_too_low: 3)", and a run with none
  answered reads "unmeasured", never a mean of 0. A `verdryx.db` written
  before the `unanswered` table existed must still open, and shows no
  unanswered column at all rather than a `0` that would look measured.

  # @test:run_summary_reports_unanswered_alongside_the_answered_mean
  Scenario: A partly-unanswered run reports its mean over answered cases plus the breakdown
    Given a run with 2 answered cases and 3 unanswered cases across two reasons
    When the console reads the run's summary
    Then the mean is over the answered cases only
    And the unanswered count and its per-reason breakdown are both reported

  # @test:a_run_with_none_answered_is_unmeasured_even_though_it_has_unanswered_rows
  Scenario: A run with none answered is unmeasured, never a mean of 0
    Given a run with zero answered cases and two unanswered cases
    When the console reads the run's summary
    Then the mean score is reported as unmeasured
    And the unanswered count is still the real count, not folded into the mean

  # @test:an_older_store_without_the_unanswered_table_still_opens
  Scenario: An older verdryx.db with no unanswered table still opens
    Given a verdryx.db written before the unanswered table existed
    When the console opens it and reads a run's summary
    Then the store opens without error
    And the summary says unanswered is not supported, never a fabricated zero

  # @test:list_baseline_summaries_carries_the_source_runs_unanswered_data
  Scenario: A baseline taken from a partly-unanswered run shows that too
    Given a saved baseline snapshotted from a run with unanswered cases
    When the console lists the saved baselines
    Then the baseline's row carries its source run's unanswered count and breakdown
