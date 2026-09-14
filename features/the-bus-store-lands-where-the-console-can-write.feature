Feature: The console's durable bus store lands where the console can write

  The console keeps one environment's bus history in `console.sqlite`, under
  `<TAIPAN_HOME>/genaryx/<env>`. Both launchers point `TAIPAN_HOME` at
  `/etc/genaryx/taipan`, a root-owned directory that holds one read-only
  mount of `environments/`, so `create_dir_all` failed with a bare
  `Permission denied` and the Bus Explorer was empty on every install of
  stack-single and every stack-k8s cluster, at v0.1.2 and v1.0.0 alike
  (genaryx#71, measured 2026-09-14). The error named no path, which is how it
  was first read as a problem with the bus files.

  @decided 2026-09-14: close it on this side, with one behaviour and a
  clearer error: an explicit `GENARYX_STATE_DIR` is asked first, then the
  path every desktop install has used, then `<HOME>/.taipan/genaryx/<env>`
  when `TAIPAN_HOME` cannot be written; a skipped candidate is named on
  stderr; a store nowhere creatable names every path it tried and the
  variable that fixes it.

  # @test:the_store_dir_is_asked_for_in_the_order_state_dir_taipan_home_then_home
  Scenario: The candidates come in a fixed order and a descriptor name cannot choose a path
    Given GENARYX_STATE_DIR, TAIPAN_HOME and HOME are all set
    When the console resolves where the durable store may live
    Then it asks for <GENARYX_STATE_DIR>/<env> first, <TAIPAN_HOME>/genaryx/<env> second and <HOME>/.taipan/genaryx/<env> last
    And with TAIPAN_HOME unset the HOME candidate is not listed twice
    And an environment name carrying a slash or ".." is reduced to one path segment

  # @test:the_store_lands_under_home_when_taipan_home_cannot_be_written
  Scenario: The launchers' shape, a read-only TAIPAN_HOME and a writable HOME
    Given TAIPAN_HOME is a directory the console cannot write
    And HOME is the console's state volume
    When the console creates its durable store
    Then the store is created under <HOME>/.taipan/genaryx/<env>
    And nothing is created under TAIPAN_HOME
    And the skipped candidate is named on stderr

  # @test:an_uncreatable_store_names_every_path_it_tried
  Scenario: Nowhere to write
    Given every candidate directory is read-only
    When the console creates its durable store
    Then the error names every path it tried
    And it says that GENARYX_STATE_DIR is the variable to set

  # @test:an_explicit_state_dir_wins_over_a_writable_taipan_home
  Scenario: A launcher that says where state goes is not second-guessed
    Given GENARYX_STATE_DIR names a directory
    And TAIPAN_HOME is writable too
    When the console creates its durable store
    Then the store is created under <GENARYX_STATE_DIR>/<env>
    And nothing is created under TAIPAN_HOME
