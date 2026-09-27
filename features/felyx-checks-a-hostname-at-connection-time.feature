Feature: The residency gate accepts an allow-listed hostname, checked at connection time

  Measured 2026-09-27: the launchers route Felyx through the stack's own
  TokenFuse gateway by default, reached in Kubernetes as a Service name
  (`http://tokenfuse-gateway:4100`) and in Compose as a service name on a
  private bridge network. The residency gate accepted only a literal
  loopback/RFC1918/link-local address or the literal `localhost`; any other
  hostname was refused outright, because the gate could not prove it
  resolved to a private address. The only existing workaround,
  `GENARYX_COPILOT_ALLOW_REMOTE`, opens every destination, which defeats the
  gate's own purpose (a sensitive install must not leak a prompt or a
  number to a public endpoint).

  @decided 2026-09-27: an operator-named hostname allow-list
  (GENARYX_COPILOT_LOCAL_HOSTNAMES) may be resolved and checked instead of
  refused outright, and the check re-runs on every connection - not only
  once when the provider is built - so a name that answers privately at
  startup and publicly later (DNS rebinding, or simply a Service's backing
  pod changing) is still caught. A hostname NOT on the allow-list, or the
  BYO-cloud opt-in (GENARYX_COPILOT_ALLOW_REMOTE), are both unchanged.

  # @test:a_name_resolving_only_to_private_addresses_is_accepted
  # @test:a_hostname_resolving_only_to_a_private_address_is_accepted_end_to_end
  Scenario: An allow-listed hostname resolves only to local addresses
    Given the operator has named the hostname in GENARYX_COPILOT_LOCAL_HOSTNAMES
    When it resolves to nothing but loopback/RFC1918/link-local addresses
    Then the provider builds, reports itself local, and a real request
      through it reaches the address the resolver handed back

  # @test:a_hostname_not_on_the_allow_list_is_refused_with_no_dns_lookup_at_all
  Scenario: A hostname is not on the allow-list
    Given the operator has not named the hostname in GENARYX_COPILOT_LOCAL_HOSTNAMES
    And GENARYX_COPILOT_ALLOW_REMOTE is not set
    When the provider is constructed
    Then it is refused, naming the endpoint, and no DNS lookup is ever made

  # @test:a_name_resolving_to_one_private_and_one_public_address_is_refused
  # @test:a_hostname_resolving_to_one_private_and_one_public_address_is_refused_at_construction
  Scenario: An allow-listed hostname resolves to a mix of addresses
    Given the hostname is on the allow-list
    When it resolves to one local address and one public address
    Then the provider is refused, naming the public address, even though
      some of the addresses it resolves to are local

  # @test:a_name_resolving_to_a_public_ipv6_address_is_refused
  Scenario: An allow-listed hostname resolves only to a public IPv6 address
    Given the hostname is on the allow-list
    When it resolves to a single public IPv6 address
    Then it is refused exactly as a public IPv4 address would be

  # @test:a_name_that_does_not_resolve_is_refused
  # @test:an_unresolvable_hostname_is_refused_at_construction
  Scenario: An allow-listed hostname does not resolve at all
    Given the hostname is on the allow-list
    When the lookup fails, or resolves to no addresses
    Then the provider is refused, naming the hostname and the reason, never
      treated as local by default

  # @test:the_resolver_rechecks_on_every_call_and_catches_a_later_public_answer
  # @test:a_hostname_that_resolves_privately_at_build_and_publicly_at_request_is_refused_at_request_time
  # @test:anthropic_hostname_path_also_rechecks_at_request_time
  Scenario: An allow-listed hostname resolves privately at build time and publicly later
    Given the hostname resolved to a local address when the provider was built
    When the same hostname resolves to a public address on the next connection
    Then that connection is refused, naming the public address, rather than
      trusting the address the build-time check saw

  # @test:refuses_public_by_default_allows_when_opted_in
  # @test:allows_public_endpoint_when_opted_in
  Scenario: The existing literal-address and BYO-cloud behaviour is unchanged
    Given a literal public IP or GENARYX_COPILOT_ALLOW_REMOTE=1
    When the provider is constructed
    Then it behaves exactly as it did before this hostname allow-list existed

  # @test:a_gated_client_on_a_literal_local_address_does_not_follow_a_redirect
  # @test:a_gated_client_on_an_allow_listed_hostname_does_not_follow_a_redirect
  Scenario: A local endpoint answers with a redirect elsewhere
    Given Felyx points at a local address or an allow-listed hostname
    When that endpoint answers 307 with a Location somewhere else
    Then Felyx does not follow it, and nothing reaches the other address

  # @test:a_gated_client_ignores_the_proxy_the_environment_names
  Scenario: The process inherited a proxy setting
    Given HTTP_PROXY names a proxy in Felyx's environment
    When Felyx sends a turn to its local endpoint
    Then the turn goes to the endpoint directly and never to the proxy
