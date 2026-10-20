Feature: The console reads the Keyryx witness's findings, read only, and turns a high one into an incident

  The Keyryx witness reads the shared event bus and notices when a stream it
  already read was rewritten, cut short or restarted, or when a sealed record
  root changed. It never writes the bus it witnesses, so its findings are a
  file of their own, `keyryx.ndjson`, which a launcher mounts into the console
  read only. Until this the console read `events.dir` and nothing else, so no
  finding reached it.

  @decided 2026-10-20: readers are given read-only access to the witness's own
  findings file, and the witness is never given write access to the bus it
  witnesses. In this console that is an optional setting, GENARYX_EVENTS_EXTRA,
  naming streams it tails beside its events directory and never writes.

  # @test:the_extra_streams_setting_keeps_absolute_paths_and_names_every_entry_it_skips
  Scenario: The setting takes absolute stream files and names what it will not read
    Given GENARYX_EVENTS_EXTRA lists a file, a directory, a relative path and a path with ".."
    When the console reads the setting
    Then it keeps the absolute file, once
    And it names the relative entry, the ".." entry and the directory as not read, and resolves none of them

  # @test:a_keyryx_finding_in_an_extra_stream_outside_the_events_dir_reaches_the_store
  Scenario: A witness finding outside the events directory reaches the console's store
    Given the bus holds a wardryx stream and the witness's findings file is in another directory
    And GENARYX_EVENTS_EXTRA names the findings file
    When the console tails its bus
    Then the witness's high finding is stored with source keyryx and the witness's own subject

  # @test:an_extra_stream_that_appears_after_startup_is_picked_up
  Scenario: A findings file that does not exist yet is picked up when it appears
    Given GENARYX_EVENTS_EXTRA names a file that is not there
    When the witness creates it with a finding
    Then the console tails it on the next tick, and that was not an error before

  # @test:an_extra_stream_that_is_a_symlink_or_a_directory_is_not_read
  Scenario: An extra stream that is a symlink or a directory is not read
    Given the findings file has been swapped for a link to wardryx.ndjson, and another entry is a directory
    When the console lists what to tail
    Then it lists neither, and only the bus's own streams

  # @test:a_line_claiming_another_source_in_an_extra_stream_is_quarantined_not_stored
  Scenario: A line in the findings file that claims another plane is quarantined
    Given keyryx.ndjson holds a line claiming source wardryx and a witness finding
    When the console tails it
    Then the wardryx line is quarantined with one fixed reason and nothing from that file is stored as wardryx

  # @test:a_stream_named_both_in_the_events_dir_and_as_an_extra_is_tailed_once
  Scenario: A stream named twice is read once
    Given keyryx.ndjson is in the events directory and is also named in GENARYX_EVENTS_EXTRA
    When the console tails its bus
    Then the file is tailed once and its finding is stored once

  # @test:a_keyryx_high_finding_becomes_an_incident_that_names_its_stream
  Scenario: A high witness finding becomes an incident that names its stream
    Given the witness found that wardryx.ndjson was rewritten, at high
    When the incident centre reads the bus
    Then there is one incident from keyryx titled "witness stream rewritten"
    And its detail names the stream, because the subject is the witness itself

  # @test:a_keyryx_finding_opens_the_bus_explorer_by_decision_not_by_falling_through
  Scenario: A witness finding opens the Bus Explorer, by decision
    Given there is no Keyryx panel and the finding is about a stream on the bus
    When an operator clicks the finding's chip
    Then the Bus Explorer opens, because keyryx is mapped to it by name

  # @test:two_streams_rewritten_are_two_incidents
  Scenario: Two streams rewritten are two incidents
    Given the witness found wardryx.ndjson rewritten twice and tokenfuse.ndjson once
    When the incident centre groups them
    Then there are two incidents, one per stream, with two and one occurrences

  # @test:a_record_root_finding_names_its_tenant
  Scenario: A finding about a sealed record names the record's tenant
    Given the witness found a sealed record root equivocated for tenant local
    When the incident centre reads it
    Then the incident's detail names the tenant

  # @test:a_medium_keyryx_finding_is_not_an_incident_on_the_card
  Scenario: A medium witness finding stays off the fire card
    Given the witness found an unsigned checkpoint, at medium
    When the Overview's incident card reads the bus
    Then it shows no incident for it, as for any medium event

  # @test:demo_bus_shows_a_high_keyryx_finding_as_an_incident_naming_its_stream
  Scenario: The demo shows a witness finding as an incident
    Given the published demo's synthetic bus
    When its Incidents read takes the first 500 events
    Then a high keyryx finding is among them and becomes an incident naming wardryx.ndjson

  # @test:demo_medium_keyryx_finding_is_on_the_incidents_tab_and_not_the_fire_card
  Scenario: The demo's medium witness finding is on the Incidents tab only
    Given the published demo's synthetic bus
    When the fire card and the Incidents tab each read it
    Then the unsigned checkpoint is on the tab and not on the card
