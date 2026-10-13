Feature: Felyx presents a gateway client key of its own

  A TokenFuse gateway with client keys on (TOKENFUSE_CLIENT_KEYS) refuses
  every metered call that carries no known x-fuse-key header with a 401,
  checked before anything else about the call (tokenfuse v1.11.0,
  crates/gateway/src/proxy.rs, resolve_client_key). stack-single#115 lets an
  installer turn client keys on, and its CI identity leg measured the 401 for
  a call with no key on 2026-10-13. Felyx calls that same gateway and sent no
  x-fuse-key anywhere (a search of crates/ at 4fc39b3 finds none), so with
  client keys on every question Felyx asked was refused.

  @decided 2026-10-13: an optional console setting names a gateway client key
  that Felyx sends as x-fuse-key on its gateway calls; the key is never logged
  or shown, and leaving the setting empty changes nothing. It is a reference,
  GENARYX_COPILOT_FUSE_KEY_REF, `env:<NAME>` or `file:<path>`, the same form
  as GENARYX_COPILOT_API_KEY_REF, so the configuration holds where the key is
  and never the key itself.

  # @test:anthropic_request_carries_the_configured_x_fuse_key_header
  # @test:openai_compat_request_carries_the_configured_x_fuse_key_header
  # @test:felyx_reads_its_fuse_key_file_and_sends_it_through_the_service
  Scenario: The operator gives Felyx a key for a gateway with client keys on
    Given the gateway refuses a call that carries no known client key
    And GENARYX_COPILOT_FUSE_KEY_REF names a file or a variable holding Felyx's key
    When Felyx asks its model a question through either provider client
    Then the request carries that key, trimmed, in the x-fuse-key header
    And the provider key and x-fuse-agent-id are still sent as before

  # @test:no_fuse_key_configured_sends_no_x_fuse_key_header
  # @test:resolve_fuse_key_unset_empty_file_env_and_refusals
  Scenario: No key is given
    Given GENARYX_COPILOT_FUSE_KEY_REF is unset, empty, or names a variable or
      a file whose value is empty
    When Felyx asks its model a question
    Then no x-fuse-key header is sent, exactly as before this setting existed

  # @test:config_from_env_reads_the_provider_surface
  Scenario: The console reads the setting from its environment
    Given GENARYX_COPILOT_FUSE_KEY_REF is set in the console's environment
    When the console builds Felyx's configuration
    Then the reference reaches the configuration as written
    And the key it points at is read only when Felyx is built

  # @test:resolve_fuse_key_unset_empty_file_env_and_refusals
  # @test:two_hundred_seeds_of_hostile_fuse_key_files_never_leak_the_key
  # @test:a_fuse_key_that_cannot_be_a_header_is_refused_without_repeating_it
  # @test:a_fuse_key_felyx_cannot_send_disables_it_without_repeating_the_key
  Scenario: The key cannot be sent as given
    Given GENARYX_COPILOT_FUSE_KEY_REF is not `env:` or `file:`, names a
      variable that is not set or a file that cannot be read, or points at a
      key with a space, a control character, a non-ASCII byte or more than
      1024 bytes
    When the console builds Felyx
    Then Felyx reports itself disabled with a reason naming the setting
    And the reason repeats neither the key nor the reference, which may be the
      key pasted in the wrong place

  # @test:a_gateway_that_echoes_the_fuse_key_never_gets_it_into_felyxs_error
  # @test:a_keyed_gateway_refusal_reaches_the_operator_without_the_fuse_key
  # @test:an_error_that_quotes_the_fuse_key_is_redacted_in_every_variant
  Scenario: A gateway echoes the key back in its error
    Given the gateway answers 401 with a body that quotes the x-fuse-key it received
    When Felyx reports the refusal to the operator
    Then the operator sees the 401 and a redaction marker where the key was
    And the key appears nowhere in the error text or its debug form

  # @test:a_fuse_key_is_sensitive_and_never_printed
  Scenario: The key is printed nowhere
    Given Felyx holds a gateway client key
    When the key, the header it becomes, or the provider holding it is printed
    Then no part of the key appears, and the header value is marked sensitive
