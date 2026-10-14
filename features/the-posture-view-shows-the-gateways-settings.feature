Feature: The Posture view shows the switches the gateway runs with

  The firewall mode, the PII mode, the run budget ceiling and the wire a
  gateway runs with were environment variables an operator could only read on
  the box, and the console never read the gateway's price book. An operator
  could not see from the console what governs their agents' calls.

  @decided 2026-10-14: the dashboards show what the product does today: a
  posture view of the firewall mode, PII mode, run budget ceiling, wire,
  client keys, identity map, price book and tiers, read from the gateway's
  own settings report when it has one, and saying "not reported by this
  gateway version" otherwise.

  # @test:the_posture_view_lists_the_gateways_own_settings_when_it_reports_them
  # @test:the_posture_view_shows_the_gateways_switches_keys_and_price_book
  Scenario: The posture view shows what the gateway reports about itself
    Given the forge gateway reports firewall enforce, PII shadow, a 5.0 USD run ceiling and the anthropic wire
    And two of its three listed client keys are configured and its identity map is in enforce
    And its price book adds openai/gpt-4.1-mini and has one prompt-length tier
    When the operator opens the Posture view
    Then the Gateway settings section shows each switch, the keys summary and the price book with its tier

  # @test:a_gateway_without_the_settings_route_says_not_reported_never_off
  # @test:a_gateway_without_the_settings_route_reads_not_reported_never_off
  Scenario: A gateway without the settings route is not shown as off
    Given the gateway answers 404 for its settings
    When the operator opens the Posture view
    Then the switches read "not reported by this gateway version", never off
    And the keys and the price book are still shown

  # @test:a_gateway_older_than_the_price_book_route_says_not_reported
  Scenario: A gateway older than the price book says so
    Given the gateway answers 404 for its price book
    When the operator opens the Posture view
    Then the price book reads not reported, with a note saying why

  # @test:a_secret_the_gateway_echoes_under_another_name_is_never_shown
  # @test:a_value_under_a_secret_shaped_name_is_withheld_and_the_rest_is_shown
  Scenario: A secret the gateway reports by mistake is never shown
    Given the gateway's settings carry an admin key, a closure secret and an upstream API key
    When the operator opens the Posture view
    Then none of those values appears, each reads withheld, and the other switches are shown

  # @test:a_gateway_that_refuses_every_read_is_an_error_not_an_empty_posture
  Scenario: A gateway that refuses every read is an error, not an empty posture
    Given the gateway refuses the console's key on every read
    When the operator opens the Posture view
    Then the section says the posture could not be read
    And a single refused read costs only its own part, with a note
