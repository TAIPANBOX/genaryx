Feature: Felyx sends its own agent identity on every model call

  Measured on a live cluster, 2026-09-27: Felyx (the copilot) was pointed at
  the stack's own TokenFuse gateway with the gateway's Wardryx hook in
  enforce mode, and every call was refused before the provider was ever
  contacted:
    400 {"error":{"reason":"policy enforcement is on and this request
    carries no agent identity; send one in `x-fuse-agent-id`",
    "retryable":false,"type":"identity_required"}}
  Felyx's own provider clients sent `x-api-key`/bearer auth and
  `x-fuse-run-id` (the C2 self-budget), but never `x-fuse-agent-id`, so the
  console's own AI could not be governed by the stack's own gateway whenever
  policy was enforced.

  @decided 2026-09-27: Felyx sends a configurable agent id on every model
  call. The launchers (stack-k8s, stack-single, stack-up) are not changed by
  this: setting `GENARYX_COPILOT_AGENT_ID` in a running stack is a separate,
  later decision.

  # @test:anthropic_request_carries_the_configured_x_fuse_agent_id_header
  # @test:openai_compat_request_carries_the_configured_x_fuse_agent_id_header
  Scenario: The gateway enforces policy and Felyx asks a question
    Given the gateway enforces policy and refuses a call with no agent identity
    When Felyx (either the Anthropic or the OpenAI-compatible provider client)
      makes a model call
    Then the request carries an x-fuse-agent-id header naming Felyx
    And the existing x-fuse-run-id header is still sent beside it

  # @test:resolved_agent_id_defaults_explicit_empty_and_malformed
  Scenario: No agent id is configured
    Given GENARYX_COPILOT_AGENT_ID is unset, or set to an empty string
    When the copilot config resolves the header value
    Then the default is agent://<GENARYX_ORG_DOMAIN, default "local">/genaryx/felyx

  # @test:resolved_agent_id_defaults_explicit_empty_and_malformed
  Scenario: An explicit agent id is configured
    Given GENARYX_COPILOT_AGENT_ID is set to a value matching the estate's
      agent-id grammar (agent://<domain>/<path>)
    When the copilot config resolves the header value
    Then the explicit value is used verbatim, never the default

  # @test:resolved_agent_id_defaults_explicit_empty_and_malformed
  Scenario: A malformed agent id is configured
    Given GENARYX_COPILOT_AGENT_ID is set to a value that does not match
      agent://<domain>/<path>
    When the copilot config resolves the header value
    Then it is refused, naming GENARYX_COPILOT_AGENT_ID and the value, never
      silently substituted or silently corrected
    And Felyx (like every other misconfigured copilot provider setting)
      reports itself disabled with that reason, rather than the console
      failing to serve every other panel
