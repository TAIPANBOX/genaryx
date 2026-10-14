Feature: The dashboard cards inset their rows

  A dashboard card has no padding of its own. Each card body insets the rows
  it holds by the same 20px that its title uses, so a row reads as part of the
  card. Some footnotes, filter chips, stat tiles and empty-state lines sat at
  the card's own edge instead, and a browser pass over the views found them.

  Each scenario below is checked by a render test that measures the inset in
  the rendered markup, inside a real card.

  # @test:the_bus_footnote_of_the_incident_center_sits_on_the_20px_inset
  Scenario: The Incident center's bus footnote lines up with the card's rows
    Given the Incident center has bus events it can account for
    When the operator reads the footnote under the incident rows
    Then the footnote starts at the same 20px inset as the rows above it

  # @test:the_incidents_footnote_sits_on_the_20px_inset
  # @test:the_incidents_severity_chips_sit_on_the_20px_inset
  Scenario: The Incidents view keeps its footnote and severity filters inside the card
    Given the Incidents view has no bus events read yet
    When the operator opens the view
    Then its footnote and its severity filter chips start at the 20px inset

  # @test:the_runs_sort_row_sits_on_the_20px_inset
  Scenario: The Runs card's sort keys line up with the runs table
    Given the Money view shows the Runs card
    When the operator looks at the sort keys above the table
    Then they start at the same 20px inset as the table rows

  # @test:the_identity_type_filter_and_note_sit_on_the_20px_inset
  # @test:an_empty_identity_list_says_so_on_the_20px_inset
  Scenario: The identities table keeps its type filter, its note and its empty line inside the card
    Given the Identity view lists identities
    When the operator reads the type filter and the note above the table
    Then both start at the 20px inset, and an empty list says so at that inset too

  # @test:the_identity_alerts_severity_filter_sits_on_the_20px_inset
  # @test:an_empty_identity_alerts_list_says_so_on_the_20px_inset
  Scenario: The Alerts card keeps its severity filter and its empty line inside the card
    Given the Identity view shows the Alerts card
    When the operator reads the severity filter and the empty-state line
    Then both start at the 20px inset

  # @test:the_policy_version_chip_sits_on_the_20px_inset
  # @test:an_empty_policy_list_says_so_on_the_20px_inset
  Scenario: The Policies card keeps its version chip and its empty line inside the card
    Given the Policy view shows the Policies card
    When the operator reads the policy version chip and the empty-state line
    Then both start at the 20px inset

  # @test:a_pending_approval_box_sits_on_the_20px_inset
  # @test:the_approvals_history_label_sits_on_the_20px_inset
  # @test:an_empty_approvals_inbox_says_so_on_the_20px_inset
  Scenario: The Approvals Inbox keeps its pending boxes, its History label and its empty line inside the card
    Given the Policy view shows the Approvals Inbox
    When the operator reads a pending approval, the History label and an empty inbox
    Then each starts at the 20px inset

  # @test:the_memory_store_scope_line_sits_on_the_20px_inset
  # @test:the_memory_store_stat_tiles_sit_on_the_20px_inset_in_every_grid
  Scenario: The Memory Store card keeps its stat tiles and its scope line inside the card
    Given the Memory view shows the Store card with its three tile grids
    When the operator reads the scope line and the first tile of each grid
    Then the scope line and every tile start at the 20px inset

  # @test:the_quality_run_detail_tiles_sit_on_the_20px_inset
  # @test:the_quality_error_box_sits_on_the_20px_inset
  # @test:an_empty_quality_run_detail_says_so_on_the_20px_inset
  Scenario: The Quality Run Detail card keeps its tiles, its error box and its empty line inside the card
    Given the Quality view shows the Run Detail card
    When the operator reads the stat tiles, an error box, and the empty state
    Then each starts at the 20px inset
