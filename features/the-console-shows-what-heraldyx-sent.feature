Feature: The console shows what heraldyx sent, and to whom

  heraldyx mails a person when an event is worth their evening, and writes one
  hash-chained record per message to its own journal, sent.ndjson, on its own
  volume: who was written to, what about, which transport carried it, and
  whether that transport took it. The console had no screen for it, so an
  operator proving they were told had to copy a volume out to read the file.

  @decided 2026-11-23: the console gets read-only screens over files already on
  the box, Notifications first, over heraldyx's dispatch journal. Nothing on the
  screen sends mail.

  # @test:a_real_heraldyx_journal_reads_as_heraldyx_reads_it
  # @test:the_chain_hash_is_the_one_heraldyx_writes
  Scenario: A journal heraldyx wrote reads the way heraldyx reads it
    Given a journal written by heraldyx with four messages, one refused by the mail server
    When the operator opens Notifications
    Then it counts four messages, three accepted and one refused
    And it says the chain verifies, three chained from one head

  # @test:records_come_newest_first_with_who_was_written_to_and_the_outcome
  # @test:a_sent_message_shows_who_was_written_to_and_how_the_transport_answered
  Scenario: Each message names who was written to and how the transport answered
    Given a message the mail server refused
    When the operator opens Notifications
    Then the row names every recipient, what it was about, and the server's refusal
    And the newest message is first

  # @test:an_edited_record_breaks_the_chain_at_the_line_after_it
  # @test:a_broken_chain_is_said_plainly_with_its_line
  Scenario: A record edited after it was written breaks the chain where it shows
    Given a journal whose third record was changed after it was written
    When the operator opens Notifications
    Then the screen says the chain is broken at line four

  # @test:a_journal_of_one_record_is_not_called_verified
  Scenario: One record binds nothing and is not called verified
    Given a journal holding one record
    When the operator opens Notifications
    Then the chain is not called verified

  # @test:a_box_with_no_journal_named_says_so_and_names_the_setting
  # @test:a_box_that_names_no_journal_says_so_and_names_the_setting
  # @test:the_journal_setting_comes_from_its_own_variable
  Scenario: A box that names no journal says so and names the setting
    Given no journal is named on the box
    When the operator opens Notifications
    Then the screen says so and names GENARYX_HERALDYX_SENT
    And it does not say that nothing was sent

  # @test:a_named_journal_that_is_not_there_is_unreadable_not_empty
  # @test:a_journal_that_could_not_be_read_is_not_shown_as_an_empty_one
  # @test:a_box_that_does_not_answer_is_not_shown_as_nothing_sent
  # @test:a_journal_path_that_is_a_link_or_a_directory_is_not_read
  Scenario: A journal that cannot be read is not shown as an empty one
    Given the named journal is missing, is a link, or the box does not answer
    When the operator opens Notifications
    Then the screen says it could not read the journal, and why

  # @test:an_empty_journal_says_nothing_was_recorded
  Scenario: An empty journal says nothing was recorded
    Given heraldyx has recorded no messages
    When the operator opens Notifications
    Then the screen says no messages were recorded

  # @test:a_malformed_line_is_named_and_makes_the_next_unverifiable_not_broken
  # @test:a_line_from_another_writer_is_named_and_is_not_a_record
  # @test:two_hundred_seeds_of_hostile_journals_never_panic_and_always_add_up
  # @test:journal_text_renders_as_text_with_hidden_characters_written_out
  Scenario: A line that is not a heraldyx record is named and never shown as one
    Given a journal with a malformed line and a line from another writer
    When the operator opens Notifications
    Then each is named by its line and why, and is not counted as a message
    And text from the journal renders as text with hidden characters written out

  # @test:a_journal_longer_than_the_window_is_read_from_its_tail_and_says_so
  # @test:long_text_and_long_recipient_lists_are_bounded
  Scenario: A long journal is read from its newest part and says so
    Given a journal longer than one read
    When the operator opens Notifications
    Then the newest messages are shown and the screen says the counts are over the newest part

  # @test:the_notifications_tab_is_in_the_assure_section
  Scenario: Notifications sits with the other evidence
    When the operator looks at the left rail
    Then Notifications is in the Assure section
