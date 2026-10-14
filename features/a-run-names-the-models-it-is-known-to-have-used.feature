Feature: The console and Felyx name only the models a run is known to have used

  The Cloud keeps one model per run, the model of the last call it ingested.
  The runs board called that "the model priced this run", and Felyx's list of
  runs handed it over as the run's model. Measured on the forge lab on
  2026-10-14: a run went from Opus to Haiku, the board showed Haiku, and asked
  which model or models the run used, Felyx answered that it used Haiku. Its
  answer check passed it, because the tool result carried only the last model.

  @decided 2026-10-14: until the Cloud lists every model a run used, the
  console says "last model", shows "+N" when it knows the run switched, and
  Felyx refuses to state a single model for a run that used more than one.

  # @test:a_run_whose_models_the_cloud_does_not_report_says_last_model
  Scenario: A run whose models the Cloud does not report says last model
    Given the Cloud reports run w2-forge-switch-a2 with model claude-haiku-5-5 and no list of models
    When the operator opens the runs table
    Then the run reads "last model claude-haiku-5-5"
    And the hover says whether the run used other models is not known

  # @test:a_run_that_switched_models_shows_the_last_one_and_plus_n
  # @test:the_models_a_run_used_reach_the_runs_table_only_when_the_cloud_reports_them
  # @test:run_agg_carries_the_models_a_run_used_only_when_the_cloud_reports_them
  Scenario: A run that switched models shows the last one and how many more
    Given the Cloud reports run w2-forge-switch-a3-133712 used claude-opus-5-5 then claude-haiku-5-5
    When the operator opens the runs table
    Then the run reads "claude-haiku-5-5 +1"
    And the hover lists both models with their calls and spend

  # @test:the_runs_export_carries_every_model_and_the_switch_count
  Scenario: The runs export carries every model and the switch count
    Given the operator exports the runs table
    Then each run carries its models and model switches when the Cloud reports them, empty when not
    And the file says model is the model of the run's last call

  # @test:list_runs_names_the_model_as_the_last_model_and_passes_the_models_through
  Scenario: Felyx's list of runs names the model as the last model
    Given Felyx lists the runs
    Then each run's model reaches it as last_model, never as the run's model
    And the models a run used reach it only when the Cloud reports them

  # @test:felyx_does_not_say_a_run_used_one_model_when_the_tools_know_only_its_last
  # @test:a_single_model_claim_for_a_run_whose_models_are_unknown_is_flagged
  Scenario: Felyx does not say a run used one model when it knows only the last
    Given the operator asks which model or models run w2-forge-switch-a3-133712 used
    And the Cloud reports only the run's last model, claude-haiku-5-5
    When Felyx drafts "the run used claude-haiku-5-5"
    Then the draft is sent back once, asking to name it as the last model
    And the answer returned names the last model and says the others are not known

  # @test:felyx_names_every_model_of_a_run_that_switched_when_the_cloud_reports_them
  # @test:a_single_model_claim_for_a_run_that_used_several_is_flagged
  Scenario: A single-model claim for a run that used several is never passed
    Given the Cloud reports the run used claude-opus-5-5 and claude-haiku-5-5
    When Felyx says the run used claude-haiku-5-5, before and after its revision
    Then the answer carries a note that the tools returned two models for the run
