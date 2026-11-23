Feature: The console shows what Keyryx custody holds and what its witness signed

  Keyryx custody holds the stack's signing keys and publishes its inventory on
  routes anyone may read; its witness keeps a signed log of what it read. The
  console had no screen for either, so which key signs approvals today, when
  it last rotated, and whether custody raised an alarm were questions for a
  shell on the box.

  @decided 2026-11-23: rotating and revoking a Keyryx key is custody's command
  line over its own unix socket, as root, with no network admin key; the
  console shows the state read-only.

  # @test:custody_and_the_witness_are_read_and_the_last_rotation_is_named
  # @test:the_last_rotation_is_when_the_newest_key_of_a_name_was_made
  # @test:custody_shows_each_key_name_with_its_active_and_next_key_and_last_rotation
  Scenario: Each key name shows its active and next key and when it last rotated
    Given custody holds an approval key that rotated once and has a next key published
    When the operator opens Keys
    Then the row names the active key, the next key and when it activates
    And it says when the newest key of that name was made

  # @test:an_alarm_custody_could_not_vouch_for_is_shown
  Scenario: An alarm custody raised is shown
    Given custody could not vouch for a state on its disk
    When the operator opens Keys
    Then the alarm is shown with its reason

  # @test:the_witness_head_shows_size_root_and_when_it_was_signed
  # @test:a_witness_with_no_head_yet_says_so
  Scenario: The witness's latest tree head is shown, or that there is none yet
    Given the witness has signed a tree head
    When the operator opens Keys
    Then the screen shows its size, its root, when it was signed and the two key ids

  # @test:the_panel_sends_only_the_three_reads_and_no_credential
  # @test:the_keys_tab_has_no_control_that_changes_a_key
  Scenario: The console changes no key and holds no admin key
    When the operator opens Keys
    Then the console sends only the three public reads, with no credential
    And the screen has no control

  # @test:nothing_named_is_not_configured_and_names_each_setting
  # @test:a_custody_or_witness_not_named_says_so_and_names_the_setting
  # @test:the_two_settings_come_from_their_own_variables
  # @test:a_setting_that_is_not_a_url_is_refused_by_name
  Scenario: A box that names neither says so and names the settings
    Given neither custody nor the witness is named on the box
    When the operator opens Keys
    Then the screen names GENARYX_KEYRYX_URL and GENARYX_KEYRYX_WITNESS_URL

  # @test:a_custody_that_does_not_answer_is_unreachable_not_empty
  # @test:an_unreachable_custody_is_not_shown_as_an_empty_inventory
  # @test:an_inventory_that_fails_while_the_state_answers_is_said
  # @test:an_inventory_that_did_not_answer_is_said_and_no_rotation_is_invented
  Scenario: A part that does not answer is never shown as an empty inventory
    Given custody does not answer, or answers its state and not its inventory
    When the operator opens Keys
    Then the screen says which read failed and invents no rotation

  # @test:a_redirect_is_not_followed_and_an_oversized_answer_is_not_read
  # @test:alarms_and_entries_are_bounded_and_entries_come_newest_first
  # @test:wire_text_renders_as_text_with_hidden_characters_written_out
  Scenario: What custody sends is bounded and rendered as text
    Given an answer that redirects, is oversized, or carries hidden characters
    When the operator opens Keys
    Then a redirect is not followed, an oversized answer is not read, and text renders with hidden characters written out

  # @test:the_keys_tab_is_in_the_assure_section
  Scenario: Keys sits with the other evidence
    When the operator looks at the left rail
    Then Keys is in the Assure section
