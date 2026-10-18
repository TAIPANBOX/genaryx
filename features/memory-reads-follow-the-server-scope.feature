Feature: The memory panel reads through a server scoped to the agent it shows

  Engram's MCP server now takes its scope from its own identity: a server
  started with an agent serves that agent, a server started without one
  serves the whole file, and a call that names any other agent is refused.
  The console used to start one server with no agent and name the agent on
  every call. Against the new server those calls fail, so the console has to
  choose the server, not the call.

  @decided 2026-10-18: the whole-file view starts one server with no agent and
  names none on any call. A per-agent view starts a server for that agent and
  names none on its calls. A read never sends a provenance, and a read that
  names an agent its server is not scoped to is refused before it is sent.

  # @test:a_whole_file_server_starts_with_no_agent
  # @test:a_whole_file_read_carries_no_agent_and_no_provenance
  Scenario: The whole-file view starts its server with no agent and names none
    Given the memory panel shows the whole file
    When the panel starts its server and reads stats and recall
    Then the server is started with no agent and the reads carry no agent and no provenance

  # @test:a_per_agent_server_starts_with_its_agent
  Scenario: A per-agent view starts a server for that agent
    Given the memory panel shows one agent
    When the panel starts the server for that agent
    Then the server is started with that agent as its identity

  # @test:a_read_never_names_an_agent_to_the_server
  Scenario: A read on a scoped server names no agent to it
    Given a server scoped to one agent
    When the panel reads stats and recall for that agent
    Then neither read names an agent on the wire

  # @test:a_read_naming_another_agent_than_the_servers_is_refused_before_the_call
  Scenario: A read for another agent is refused before anything is sent
    Given a server scoped to one agent, or a whole-file server
    When a read names an agent that server is not scoped to
    Then the read is refused and nothing is sent to the server

  # @test:a_scope_is_spawned_once_and_then_reused
  Scenario: A scoped server is reused for later reads of its agent
    Given a per-agent server already started
    When the panel reads the same agent again
    Then no second server is started

  # @test:the_pool_never_holds_more_than_its_bound
  # @test:the_least_recently_used_scope_is_the_one_evicted
  Scenario: The number of scoped servers is bounded
    Given the bound on scoped servers is two
    When a third agent is read
    Then only two scoped servers are held, and the least recently used one is the one closed

  # @test:an_idle_scope_is_closed_on_the_next_use
  # @test:a_recently_used_scope_is_kept_when_another_goes_idle
  Scenario: A scoped server idle for too long is closed
    Given a scoped server that has not been read for longer than the idle limit
    When another agent is read after that
    Then the idle server is closed, and a recently used one stays

  # @test:a_failed_start_is_not_held_and_is_tried_again
  Scenario: A server that failed to start is not kept
    Given a scoped server whose start fails
    When the same agent is read again
    Then nothing is held for the failed start and the start is tried again

  # @test:a_scope_must_be_one_printable_word
  Scenario: An agent name that is not one printable word never reaches the command line
    Given an agent name with a space, a control character, a leading dash or no name
    When the panel reads with that name
    Then the name is refused and no server is started for it

  # @test:an_empty_agent_is_the_whole_file_view
  Scenario: An empty agent name is the whole-file view
    Given the panel asks for an empty agent name
    When the read scope is worked out
    Then the read is the whole-file view

  # @test:a_bad_agent_is_refused_before_any_server_starts
  Scenario: A bad agent name is refused as a memory error before any server starts
    Given an agent name that is not one printable word
    When the read scope is worked out
    Then the memory error says the name is refused, and no server starts
