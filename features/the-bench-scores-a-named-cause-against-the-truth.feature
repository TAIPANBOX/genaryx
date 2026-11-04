Feature: The bench scores a named cause, the ids an answer cites and the remedy against the truth

  The pieces of the incident-to-proven-fix loop are each checked on their own. The
  bench asks whether the loop, taken whole, reaches the right answer on an
  incident whose cause is known. It is modelled on the costcrew bench: incidents
  with a known cause, a packet that hides it, and three questions asked of what
  the analyst says.

  @decided 2026-11-04: the three questions are whether the named cause was right,
  whether every cited id exists, and whether the remedy that follows from the
  named cause stops the replay of the incident's recorded history. A cause is
  judged on what the answer NAMES: an answer that names two causes names none, and
  the true cause's words appearing somewhere in a long answer prove nothing. A
  remedy the console cannot prove is neither a pass nor a miss. CI runs scripted
  models; the same cases can be put to a real model through an endpoint named by
  the environment.

  # @test:the_packets_hide_the_cause_and_every_cause_has_a_case
  # @test:a_packet_gives_the_cause_away_only_by_its_words
  Scenario: The packet hides the cause
    Given incidents with a known cause, and the words that would give each cause away
    When the packet an analyst reads is checked
    Then it holds none of the words of any cause, and every cause has an incident

  # @test:a_cause_is_named_when_one_cause_is_named_in_a_sentence_and_no_other
  # @test:a_200_seed_sweep_of_text_never_panics_and_never_names_two_ways
  Scenario: A cause is judged on what the answer names
    Given answers that name a cause, name two, name none, or spread a cause's words over two sentences
    When the bench reads which cause each names
    Then one cause named is that cause, two named is none, and the words of a cause in two sentences are not the cause

  # @test:an_oracle_that_reads_the_case_scores_every_question_it_is_asked
  Scenario: An analyst that reads the case scores every question
    Given a scripted model that names the true cause from the case alone, for each known incident
    When the real Felyx loop explains each case
    Then the cause is right, every cited id exists, and the remedy that follows replays as stopping the incident
    And an incident whose cause calls for no console remedy has none counted

  # @test:a_rival_cause_misses_the_cause_and_the_remedy_but_not_the_ids
  Scenario: Naming another cause misses the cause and the remedy
    Given a scripted model that names a different cause, citing only what the case holds
    When the loop explains each case
    Then the cause is wrong, the ids exist, and the remedy that follows from the wrong cause does not stop the incident

  # @test:an_answer_that_cites_a_run_nobody_holds_misses_the_ids_and_is_withheld
  # @test:the_first_two_questions_are_judged_on_the_cause_named_and_on_what_was_left_unsupported
  Scenario: Citing a run nobody holds misses the ids
    Given a scripted model that cites a run the case does not hold
    When the loop explains each case
    Then the chain is withheld, no cause is named, the ids do not exist, and no remedy is replayed

  # @test:an_answer_that_names_nothing_misses_the_cause_and_has_no_remedy_to_replay
  # @test:an_answer_that_names_two_causes_names_none
  Scenario: Naming nothing, or two causes, names no cause
    Given a scripted model that says the case does not say, and one that names two causes
    When the loop explains each case
    Then no cause is named and no remedy is replayed

  # @test:the_recorded_replays_say_what_the_bench_says_they_do
  # @test:the_remedy_a_cause_calls_for_is_the_one_that_follows_from_it
  Scenario: The remedy replayed is the one the named cause calls for
    Given the recorded output of the two engines for each incident and each kind of remedy
    When the real parsers and judges read them
    Then a budget stops the spend loop, a policy stops the denied call, and neither stops a refused key

  # @test:a_remedy_that_does_not_apply_is_counted_as_neither_a_pass_nor_a_miss
  # @test:the_report_names_every_case_and_every_question
  Scenario: The report counts what applies and names every case
    Given scores in which some remedies do not apply
    When they are tallied and rendered
    Then a remedy that does not apply is counted as neither a pass nor a miss, and each case and question is named
