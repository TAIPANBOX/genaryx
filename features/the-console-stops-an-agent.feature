Feature: The console stops an agent everywhere with one action

  Stopping one agent used to take three services and no shared handle: kill
  each run, revoke its delegation at vouchryx, remove its key and restart the
  gateway. An operator watching a compromised agent needs one action.

  @decided 2026-10-18: an operator stops an agent with one console action. It
  bans the agent at its issuer, so every token it holds dies at every door and
  no new one is issued until the ban is lifted, then kills the agent's open
  runs, and records every step in the console's own chain. Lifting the ban is
  the same ceremony. A step that fails is shown and recorded, and an action
  with a failed step is never reported as a stop or a lift.

  # @test:stopping_an_agent_bans_it_and_kills_each_open_run
  Scenario: Stopping an agent bans it and kills each of its open runs
    Given an agent with two open runs, one run already killed, and runs of other agents
    When an admin stops it
    Then vouchryx is asked once for a ban
    And the Cloud kills exactly the agent's two open runs
    And the console reports the agent stopped

  # @test:the_stop_asks_vouchryx_for_a_ban_not_a_revocation
  Scenario: The stop asks for a ban, not only a revocation
    Given vouchryx revokes an agent's tokens on POST /v1/revoke and bans it only when asked
    When an admin stops the agent
    Then the request names the agent as subject, no token, the operator as actor, the reason, and ban true

  # @test:every_step_of_a_stop_is_journaled_in_order_ceremony_first
  # @test:a_confirmed_ceremony_is_the_first_line_and_signs_every_step
  # @test:agent_stop_with_a_valid_assertion_reaches_the_command_and_records_the_ceremony
  Scenario: Every step of a stop is recorded in the console's own chain, the ceremony first
    Given an admin confirmed the stop with a passkey
    When the stop runs
    Then the chain holds the ceremony, the ban, the run listing, one line per run killed, and the verdict, in that order
    And every line carries the passkey that confirmed it
    And a stop with no passkey enrolled says so on its first line rather than claiming a ceremony

  # @test:a_revocation_without_a_confirmed_ban_is_never_a_stop
  Scenario: A revocation without a confirmed ban is never a stop
    Given a vouchryx that answers revoked true and does not say banned
    When an admin stops an agent
    Then the ban step is recorded as revoked but not banned
    And the console does not report the agent stopped

  # @test:a_refused_ban_is_shown_and_recorded_and_the_runs_are_still_killed
  Scenario: A ban vouchryx refuses is shown and recorded, and the runs are still killed
    Given vouchryx refuses the ban, cannot keep it, answers with something that is not vouchryx, cannot be reached, or is not configured
    When an admin stops an agent
    Then the ban step is recorded with vouchryx's real status and code
    And the agent's open runs are still killed
    And the console does not report the agent stopped

  # @test:a_run_the_cloud_will_not_kill_makes_the_stop_incomplete
  Scenario: A run the Cloud will not kill makes the stop incomplete
    Given the Cloud refuses one kill, or answers with another run id
    When an admin stops the agent
    Then that kill is recorded as failed and the verdict names how many kills failed
    And the console does not report the agent stopped

  # @test:runs_that_cannot_be_listed_make_the_stop_incomplete
  Scenario: Runs that cannot be listed make the stop incomplete
    Given the Cloud cannot list runs, or the console has no Cloud connection
    When an admin stops an agent
    Then the open runs are unknown, never zero
    And the console does not report the agent stopped

  # @test:only_the_agents_own_open_runs_are_killed
  Scenario: Only the agent's own open runs are killed
    Given an agent whose id is the start of another agent's id, and runs of a third agent
    When an admin stops the first agent
    Then no run of the other two agents is killed, and no run already killed is killed again

  # @test:a_ban_vouchryx_cannot_keep_is_a_stop_that_says_so
  Scenario: A ban vouchryx cannot keep across a restart is a stop that says so
    Given a vouchryx with no revocation store, which answers banned true and durable false
    When an admin stops an agent
    Then the agent is reported stopped with a warning that a vouchryx restart lifts the ban

  # @test:lifting_an_agent_asks_vouchryx_to_lift_the_ban_and_records_it
  Scenario: Lifting an agent asks vouchryx to lift the ban and records it
    Given a banned agent
    When an admin lifts the ban
    Then vouchryx receives POST /v1/revoke/lift with the agent, the operator and the reason
    And the chain holds the ceremony, the lift and the verdict

  # @test:a_lift_vouchryx_refuses_is_never_reported_as_lifted
  # @test:agent_lift_with_a_valid_assertion_reaches_the_command_and_records_the_ceremony
  Scenario: A lift vouchryx refuses is never reported as lifted
    Given vouchryx refuses the lift, cannot keep it, or answers without confirming it
    When an admin lifts the ban
    Then the lift step is recorded with vouchryx's real status
    And the console does not report the ban lifted

  # @test:agent_stop_with_a_passkey_and_no_assertion_is_428
  # @test:agent_lift_with_a_passkey_and_no_assertion_is_428
  # @test:stopping_and_lifting_an_agent_are_admin_rows_with_the_ceremony
  Scenario: Stop and lift need a fresh passkey assertion
    Given an admin with an enrolled passkey
    When they stop or lift an agent without a fresh assertion
    Then the console asks for the assertion and calls nobody

  # @test:a_viewer_or_approver_is_refused_by_the_role_gate_on_agent_stop_and_lift
  Scenario: A viewer or an approver cannot stop or lift an agent
    Given a signed-in viewer, or a signed-in approver
    When they stop or lift an agent
    Then the console refuses with role admin required

  # @test:a_stop_naming_no_agent_reaches_nobody
  # @test:an_agent_id_follows_the_estate_grammar
  # @test:two_hundred_seeds_of_hostile_stop_arguments_never_half_validate
  Scenario: A stop that names no agent, or gives no reason, reaches nobody
    Given a subject that is not an agent id, or an empty or overlong reason
    When an admin stops or lifts it
    Then the console refuses before vouchryx or the Cloud is called, and records nothing

  # @test:the_revoke_key_never_appears_in_a_stop_journal_or_answer
  Scenario: The revoke key is never shown
    Given the console holds vouchryx's bearer revoke key
    When a stop is refused by vouchryx
    Then the key appears neither in the chain nor in the answer

  # @test:a_complete_stop_reads_stopped_and_lists_every_step
  # @test:a_stop_with_a_failed_step_never_reads_as_stopped
  # @test:a_step_the_journal_did_not_take_is_said
  # @test:a_ban_a_restart_forgets_is_a_warning
  # @test:a_lift_vouchryx_refused_never_reads_as_lifted
  Scenario: The operator reads the outcome step by step
    Given a stop or a lift has run
    When the console shows its outcome
    Then a complete action reads as done and lists every step
    And an action with a failed step reads as not done, with the failed step and its reason
    And a step the chain did not take, and a ban a restart would forget, are said
