Feature: A refused client key and a PII finding read as what they are

  A gateway with client keys refused a call that presented no key or an
  unknown one with a 401 and wrote nothing an operator could see, and its PII
  check in shadow mode recorded a finding only on the answer, never where a
  dashboard could read it. TokenFuse now writes both: a refused key as
  identity_mismatch with the cause and, for an unknown key, a fingerprint
  rather than the key; a PII finding as a count per run and per kind in the
  Cloud, and as a dlp_shadow event. Both are optional: an older gateway writes
  neither, and the console does not wait for them.

  @decided 2026-10-14: PII shadow findings and refused-key events appear in
  the console once TokenFuse emits them, on the incident timeline the console
  already reads, as optional types.

  # @test:a_refused_client_key_reads_as_a_refused_key_not_an_identity_mismatch
  Scenario: A call refused for having no client key reads as a refused key
    Given the gateway refused two calls that claimed a capped agent and presented no client key
    When the operator opens the incident centre
    Then one incident reads "client key refused: none presented"
    And its detail says which agent was claimed and how many more refusals the rate limit held back
    And it is not filed under the claimed agent

  # @test:two_unknown_keys_are_two_incidents_and_a_missing_key_a_third
  Scenario: Different unknown keys are different incidents
    Given the gateway refused two different unknown keys and one call with no key
    When the operator opens the incident centre
    Then there are three incidents, each unknown key named only by its fingerprint

  # @test:a_pii_finding_in_shadow_reaches_the_anomalies_tab_with_its_kinds
  Scenario: A PII finding in shadow reaches the Anomalies tab with its kinds
    Given the gateway forwarded a call in which it found one card number and two email addresses
    When the operator opens the Anomalies tab
    Then a row reads "personal data found (shadow)" with the kinds, their counts and the unit
    And the fire card on the Overview does not carry it, since it is a medium finding

  # @test:a_run_that_carried_personal_data_carries_a_pii_pill
  # @test:the_calls_a_run_carried_personal_data_in_reach_the_runs_table_when_reported
  Scenario: A run that carried personal data says so on the runs board
    Given the Cloud reports three calls on a run in which personal data was found
    When the operator opens the runs table
    Then the run carries a "pii ×3" pill, and a Cloud that does not report it shows none

  # @test:the_overview_says_how_many_calls_carried_personal_data_or_nothing_when_unreported
  # @test:the_overview_carries_pii_counts_only_when_the_cloud_reports_them
  # @test:pii_findings_are_read_when_the_cloud_reports_them_and_unknown_when_not
  Scenario: The Overview says how many calls carried personal data
    Given the Cloud reports four calls with personal data, three with an email and one with a card
    When the operator opens the Overview
    Then the model calls tile says "4 with personal data: email 3, card 1"
    And a Cloud that does not report it says nothing about personal data
