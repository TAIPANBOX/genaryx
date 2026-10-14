Feature: The public demo names only models that are current

  The published demo answers every panel from fixtures. Its Quality, money and
  agent answers named model ids that are retired or a generation old, so a
  visitor could read them as the models the console runs on. The demo now names
  only current ids, and the fixtures keep their costs and scores plausible for
  the model each row names.

  # @test:the_demo_answers_name_no_retired_model_id
  Scenario: The demo's answers name no retired model
    Given the demo build with no box behind it
    When a visitor reads the Quality, money, agent and statistics panels
    Then no model named in any of those answers is a retired one
