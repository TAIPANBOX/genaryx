Feature: The Overview shows a gateway that stopped reporting

  Every gateway with a Cloud sink pushes its calls, and an empty heartbeat
  every 30 seconds by default, so the Cloud knows when it last heard from
  each site. A gateway that crashed, lost its network or its key stops
  pushing; its runs, spend and kills stop moving through the Cloud, and every
  other view goes quiet about it without saying why. The console never read
  the Cloud's per-site record, so a silent gateway was invisible in the
  operator's own UI.

  @decided 2026-10-14: the dashboards show what the product does today: the
  console reads the Cloud's gateways and shows a gateway that stopped
  reporting.

  # @test:a_gateway_that_stopped_pushing_reads_silent_and_leads_the_list
  # @test:a_gateway_that_stopped_reporting_reads_silent_on_the_overview
  Scenario: A gateway the Cloud has not heard from for 47 minutes reads silent
    Given the Cloud last heard from forge-openai 47 minutes ago and from forge 20 seconds ago
    When the operator opens the Overview
    Then forge-openai leads the Gateways section, marked silent for 47 min
    And forge reads reporting, heard 20 s ago

  # @test:a_gateway_is_silent_only_past_five_minutes_without_a_push
  Scenario: Silent means more than five minutes without a push
    Given one gateway last pushed exactly five minutes ago and another five minutes and one millisecond ago
    When the console judges them
    Then the first is reporting and the second is silent

  # @test:a_cloud_without_the_gateways_route_says_not_reported_never_none_silent
  # @test:a_cloud_without_the_gateways_route_is_not_shown_as_none_silent
  Scenario: A Cloud without the gateways route is not shown as all reporting
    Given the Cloud answers 404 for its gateways
    When the operator opens the Overview
    Then the section says this Cloud cannot show whether a gateway stopped reporting

  # @test:a_cloud_that_refuses_the_gateways_read_is_an_error_not_an_empty_list
  Scenario: A Cloud that refuses the read is an error, not an empty list
    Given the Cloud answers 500 for its gateways
    When the operator opens the Overview
    Then the section says the gateways could not be read

  # @test:reading_which_gateways_report_is_a_viewer_read
  Scenario: Any operator can see which gateways report
    Given a signed-in viewer
    When they read the gateways
    Then the role gate lets the read through
