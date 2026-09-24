Feature: Operator sign-in keeps working when the IdP rotates its keys

  The console's IdP sign-in verified an ID token against a STATIC JWKS
  (GENARYX_WEB_OIDC_JWKS, docs/CONSOLE-IDP.md). Okta rotates its signing keys
  automatically, about four times a year and without notice; Entra rotates on
  no fixed interval and immediately in an emergency. With a static JWKS,
  every rotation locked operators out of IdP sign-in until someone edited
  configuration by hand. The static path stays: it is the default and the
  air-gap path. GENARYX_WEB_OIDC_JWKS_URL is the optional live alternative
  (CLAUDE.md invariant 11).

  # @test:a_rotated_key_verifies_after_the_kid_miss_refresh
  Scenario: A rotation is picked up without a restart
    Given the console trusts a JWKS it fetched from the IdP
    And the IdP rotates in a new signing key under a new kid
    When an operator signs in with a token signed by the new key
    Then the console refreshes the key set on the unknown kid and verifies the token
    And no restart or configuration edit was needed

  # @test:an_outage_keeps_verifying_on_the_last_good_set
  Scenario: An IdP outage keeps sign-in working on the last good keys
    Given the console has already fetched a good key set
    And the IdP becomes unreachable
    When the current set goes stale and a refresh is attempted
    Then the failed refresh does not evict the last good set
    And an operator can still sign in with a token the last good set verifies

  # @test:a_hundred_concurrent_unknown_kids_within_one_cooldown_make_exactly_one_fetch
  Scenario: A flood of unknown key ids costs at most one fetch per cooldown
    Given many sign-in attempts arrive at once, each carrying a kid the console has never seen
    When the console handles all of them inside one five-minute cooldown window
    Then it fetches the JWKS exactly once, never once per attempt

  # @test:after_max_age_a_refresh_that_drops_a_key_fails_that_keys_token
  Scenario: A key the IdP removed stops working after the hour
    Given the console trusts a key the IdP later removes from its JWKS
    When more than one hour passes and the console refreshes successfully
    Then a token signed by the removed key no longer verifies
    And a token signed by the IdP's current key verifies instead

  # @test:hostile_jwks_bodies_are_refused_and_the_last_good_set_stays
  Scenario: A published private key is refused
    Given the console asks the IdP for its JWKS
    When the response carries a key with a private-key member, or an unsupported key type, or is otherwise not a well-formed key set
    Then the console refuses the whole fetched set
    And it keeps verifying with the last good set it already trusted

  # @test:both_jwks_sources_set_refuses_to_start
  Scenario: Both sources set refuses to start
    Given both GENARYX_WEB_OIDC_JWKS and GENARYX_WEB_OIDC_JWKS_URL are set
    When genaryx-web starts
    Then it refuses to start rather than guess which key source to trust
