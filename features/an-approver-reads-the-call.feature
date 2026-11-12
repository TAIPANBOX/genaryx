# Provenance: @decided 2026-11-12 (paraphrased): a person approving a tool call
# is shown its arguments, taken from the enforcement point and never from model
# text; secrets are hidden before anything is shown; a decision without seeing
# them is allowed and marked. The console is the half that opens what wardryx
# sealed for it and says, in the person's signed statement, what was shown.
# This file states that in the repository's own words; it quotes no person.

Feature: An approver reads the call before deciding it

  # @test:a_preview_wardryx_sealed_opens_under_the_console_key_for_both_kems
  # @test:the_info_and_the_digest_are_wardryxs_byte_for_byte
  Scenario: The console opens what wardryx sealed for it
    Given a preview wardryx sealed to the console's key
    When the console opens it
    Then it reads the document wardryx built, for both kinds of key

  # @test:a_ciphertext_does_not_open_under_another_binding_or_another_key
  # @test:a_ciphertext_that_opens_to_another_document_is_refused_for_its_digest
  # @test:hostile_ciphertexts_are_refused_without_panic
  Scenario: A preview moved to another hold, changed, or not the one the hold records is refused
    Given a sealed preview
    When it is opened for another org or call, with another key, cut short, with a bit changed, or it holds another document
    Then nothing is shown

  # @test:only_the_two_allowed_kems_and_well_formed_keys_are_accepted
  # @test:the_key_never_prints_its_material
  # @test:the_preview_key_file_comes_from_its_own_variable
  Scenario: The console's preview key is its own and never printed
    Given an operator who generates the console's preview key
    When the console reads it from its own variable
    Then only the two allowed kinds of key load, and the key never appears in a log line

  # @test:a_held_preview_is_read_as_its_state_and_digest_and_never_more
  # @test:a_fetched_preview_never_prints_its_ciphertext
  Scenario: The list of holds says what can be shown, and never the arguments
    Given a hold wardryx recorded with a preview
    When the console lists it
    Then the row carries the preview's state and digest and nothing of the arguments

  # @test:the_preview_shows_the_arguments_and_says_whose_words_they_are
  # @test:a_hidden_value_shows_its_rule_and_type_and_nothing_of_the_value
  # @test:a_vault_handle_a_blob_and_a_cut_show_what_they_are
  # @test:a_repeated_key_is_shown_twice_and_marked
  Scenario: The person reads the arguments with secrets hidden
    Given an opened preview
    When the inbox shows it
    Then the person reads the values, sees what was hidden and why, and is told these are data from the agent

  # @test:text_an_agent_wrote_renders_as_text_never_as_markup
  # @test:a_character_that_reverses_text_is_written_out_and_flagged
  Scenario: Text the agent wrote cannot act on the page or hide how it reads
    Given arguments holding markup or a character that reverses text
    When the inbox shows them
    Then the markup is shown as text and the character is written out and marked

  # @test:a_sealed_hold_offers_to_show_its_arguments_and_holds_the_grant_until_then
  # @test:a_grant_of_a_sealed_hold_is_allowed_once_the_preview_is_open
  # @test:a_grant_of_a_sealed_hold_names_the_preview_the_person_saw
  # @test:a_grant_of_a_sealed_hold_nobody_opened_or_opened_stale_is_refused
  Scenario: A grant waits until the person opened the arguments, and then names them
    Given a hold whose arguments are sealed for the console
    When the person has not opened them
    Then Grant is held back, and once opened the grant names the preview the person saw

  # @test:a_digest_the_browser_sends_without_an_open_behind_it_is_refused
  # @test:an_open_is_remembered_for_that_person_and_that_hold_only
  Scenario: A browser cannot claim the person saw what they never opened
    Given a digest anyone can copy from the list of holds
    When a decision names it without that person opening that preview here
    Then the console refuses the decision

  # @test:a_deny_of_a_sealed_hold_may_name_no_preview
  Scenario: A person may say no without reading
    Given a hold whose arguments are sealed
    When the person denies it without opening them
    Then the denial is signed as having seen no preview

  # @test:a_call_whose_arguments_could_not_be_shown_is_granted_unseen_and_says_so
  # @test:a_hold_with_no_sealed_preview_is_decided_blind_and_named_so
  # @test:a_policy_that_requires_a_preview_blocks_an_unseen_grant
  # @test:require_preview_refuses_a_blind_grant_and_not_a_call_with_nothing_to_show
  Scenario: A call whose arguments could not be shown is granted unseen and marked
    Given a hold whose arguments were too large or had no preview
    When the person grants it
    Then the button says the grant is unseen, the statement names no preview, and a policy that requires one blocks it

  # @test:a_version_2_statement_names_the_call_and_the_preview_the_person_was_shown
  # @test:a_version_1_statement_carries_no_tcd_or_pvd
  # @test:a_malformed_tcd_or_pvd_is_refused_before_anything_is_signed
  # @test:what_the_person_was_shown_goes_into_a_version_2_statement
  # @test:a_hold_from_a_wardryx_without_previews_gets_the_version_1_statement
  # @test:a_hold_with_no_call_names_none_for_both
  # @test:a_hold_from_a_wardryx_without_previews_keeps_the_plain_grant
  Scenario: The signed statement names the call and the preview, and an older wardryx gets the old one
    Given a person's decision confirmed by a passkey
    When the console signs the statement
    Then it names the call's digest and the preview's digest, or none, and a hold from a wardryx without previews gets the statement it reads

  # @test:reading_a_held_calls_preview_is_an_approver_read_with_no_ceremony
  Scenario: Reading a held call's arguments takes the role that decides it
    Given the command that opens a preview
    When a viewer or an approver asks for it
    Then only an approver may, and reading changes nothing so it needs no passkey
