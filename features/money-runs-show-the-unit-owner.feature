Feature: Money runs carry the unit owner

  TokenFuse Cloud's `RunAgg` (`GET /v1/runs`) and its `OwnerAgg` (`GET
  /v1/owners`) both carry the human at the root of a run's delegation chain.
  Before this, the console's own `RunAgg` mirror had no `owner` field at all,
  so a Cloud that sent one had it silently dropped for every run listed.

  @decided 2026-09-26: `owner` is read as a bare string with a default,
  mirroring how `unit` was already handled: an older Cloud that never sends
  the field renders as "not reported", never a blank cell that reads as "no
  owner".

  # @test:run_agg_deserializes_with_last_seen_millis_rename
  Scenario: An older Cloud with no owner field still parses
    Given the Cloud's GET /v1/runs response carries no owner field at all
    When the console parses the response
    Then the run's owner defaults to empty rather than failing to parse

  # @test:run_agg_carries_a_resolved_owner_when_the_cloud_sends_one
  Scenario: A Cloud that resolves an owner has it carried through
    Given the Cloud's GET /v1/runs response names an owner for a run
    When the console parses the response
    Then the run's owner is exactly what the Cloud sent
