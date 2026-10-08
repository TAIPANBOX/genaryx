Feature: An identity refusal is filed under the key that made the call, never the agent it claimed

  TokenFuse refuses a call when the credential presented may not speak as the
  agent the caller named, and it writes an identity_mismatch event whose
  envelope agent_id is the CLAIMED agent and whose data.key_id is the
  credential that actually called. Measured on a home-lab run on 2026-10-07
  (the same run behind TokenFuse invariant 81): a caller holding the key
  forge-imposter claimed to be agent://taipanbox.dev/routers/flint and was
  refused, which is right. This console then read the envelope like every
  other event, so flint was counted as stopped, the attempt sat on flint's own
  stop list and profile, the incident centre named flint as the subject of a
  high-severity incident, and Felyx's cost per action counted the refused
  calls among flint's own.

  @decided 2026-10-08: the console files an identity refusal under the key
  that made the call, as key:<key_id>, the same rule TokenFuse applies to its
  FOCUS export, and never under the agent it claimed. A refusal that carries
  no key is filed under key:(none), which names nobody, rather than under the
  one agent it is known not to be. The incident says both halves: which agent
  was claimed and which key made the call.

  # @test:an_identity_refusal_is_counted_under_the_key_not_the_claimed_agent
  Scenario: The Statistics panel counts the attempt against the key, not the agent it claimed
    Given flint was stopped once by its own policy
    And the key forge-imposter tried twice to call as flint and was refused both times
    When the operator opens the Statistics panel
    Then flint shows one stop, its own
    And a row for key:forge-imposter shows the two refused attempts

  # @test:an_identity_refusal_is_not_on_the_claimed_agents_stop_list
  Scenario: The claimed agent's own stop list does not carry somebody else's attempt
    Given the key forge-imposter was refused while claiming to be flint
    When the operator opens flint's Agent 360 card
    Then flint's stop list holds only flint's own stops
    And the attempt is listed under key:forge-imposter instead

  # @test:an_identity_refusal_is_grouped_under_its_key_in_the_aggregate
  # @test:an_identity_refusal_is_filed_the_same_way_in_sql_and_in_rust
  Scenario: The store files the attempt under the key in every count it makes
    Given refusals claiming flint arrive with a key, with an empty key, with no key, and with keys shaped like SQL or like an agent id
    When the console counts the bus and builds flint's profile
    Then none of them is counted under flint
    And each one is filed under key:<its key>, or under key:(none) when it carried no readable key

  # @test:an_identity_refusal_names_the_key_and_the_claimed_agent
  # @test:identity_refusals_group_by_key_not_by_the_claimed_agent
  Scenario: The incident names the claim and the key, and is not raised against the claimed agent
    Given the key forge-imposter was refused while claiming to be flint
    When the operator opens the incident centre
    Then the incident's subject is key:forge-imposter
    And its detail says it claimed flint with key forge-imposter
    And two different keys claiming flint are two incidents, one per key

  # @test:an_identity_refusal_with_no_key_names_no_agent
  # @test:an_identity_refusal_with_no_readable_key_names_no_agent
  Scenario: A refusal with no key names nobody
    Given a refusal claiming flint arrives from a gateway running without client keys
    When the console files it
    Then it is filed under key:(none)
    And the incident says it claimed flint with no key

  # @test:an_identity_refusal_is_filed_under_its_key_in_cost_per_action
  Scenario: Felyx's cost per action does not count refused calls among the claimed agent's own
    Given flint made two calls of its own and the key forge-imposter made three refused calls claiming flint
    When Felyx reads cost per action by agent
    Then flint shows its two calls and its own spend
    And key:forge-imposter shows the three refused calls

  # @test:demo_bus_shows_an_identity_refusal_under_the_key_not_the_claimed_agent
  # @test:demo_statistics_count_the_refusal_under_the_key
  # @test:demo_claimed_agents_feed_carries_the_refusal_as_a_claim_by_the_key
  Scenario: The public demo shows an identity refusal filed under the key
    Given a visitor opens the published demo
    When they open the Incidents tab or the Statistics panel
    Then an identity refusal by the key forge-imposter claiming budget-forecaster is listed under key:forge-imposter
    And the incident says it claimed budget-forecaster with key forge-imposter
    And budget-forecaster's own feed reads it as claimed by key forge-imposter, refused

  # @test:an_identity_refusal_on_the_claimed_agents_feed_reads_as_a_claim_by_the_key
  # @test:every_other_event_on_the_feed_is_the_agents_own_and_reads_as_its_type
  Scenario: The claimed agent's own event feed shows the attempt as a claim, not as its activity
    Given the key forge-imposter was refused while claiming to be flint
    When the operator opens flint's Agent 360 card
    Then the event feed reads "claimed by key forge-imposter, refused" for that line
    And it is marked as not flint's own activity

  # @test:an_identity_refusal_counts_as_the_keys_activity_in_the_graph
  # @test:an_identity_refusal_counts_as_the_keys_activity_from_the_store
  Scenario: The graph counts the attempt as the key's activity
    Given flint acted once and the key forge-imposter was refused once while claiming flint
    When the console builds the delegation graph, live or from the store
    Then flint's event count is one, its own
    And the key forge-imposter carries the refused attempt

  # @test:an_identity_refusal_is_not_a_call_of_the_model_it_named
  Scenario: A refused call is not a call of the model it named
    Given a model served two calls and was named by three calls the gateway refused for identity
    When Felyx reads cost per action by model
    Then the model shows two calls
    And a model named only by refused calls does not appear at all

  # @test:a_runless_incident_says_there_is_no_run_to_read_instead_of_reading_forever
  # @test:an_incident_with_a_run_id_still_reads_its_run
  Scenario: An incident with no run id says there is no run, instead of reading forever
    Given a bus incident whose event carries no run id
    When the operator opens it in Incident 360
    Then "Where it was stopped" and "What led to it" both say there is no run id on this event, so there is no run to read
    And an incident with a run id still reads its run
