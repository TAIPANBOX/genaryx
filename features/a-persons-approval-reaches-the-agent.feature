Feature: A person's approval in the console reaches the agent

  When an operator granted a held call, wardryx returned the approval token once
  and never again, and the console decoded it and handed the web only the parsed
  fields. The enforcement point takes the raw token in the header
  x-fuse-approval-token on a retry, so a grant in the console opened nothing: no
  person could ever present what the agent needed. A token signed through Keyryx
  did not even decode, because the decoder read the first of its three segments.

  @decided 2026-11-07: the console shows the raw token once after a grant, with a
  copy button, the header's name, the expiry countdown and a plain line saying
  what it allows. It is a live credential with one copy, so it lives in that one
  answer and in the memory of the component that shows it, and it reaches no log,
  no record bus, no audit trail, no Felyx, no store and no URL. The inbox shows
  the reason for a hold and the target of the tool, so that nobody decides blind,
  and says so where wardryx sends no arguments or the arguments were truncated.

  # @test:a_grant_answers_with_the_raw_token_wardryx_minted
  # @test:a_grant_through_the_router_answers_the_raw_token_and_keeps_no_copy
  # @test:a_grant_in_the_console_shows_the_raw_token_once_with_its_header
  # @test:the_panel_shows_the_raw_token_the_header_and_the_countdown
  Scenario: A grant shows the raw token once, with the header it goes in
    Given a call held for a person
    When the operator grants it in the console
    Then the console shows the raw token wardryx minted
    And it names the header x-fuse-approval-token
    And the expiry countdown runs beside it

  # @test:the_panel_says_the_token_allows_one_call_bound_to_it_and_cannot_be_shown_again
  # @test:every_plain_line_says_the_token_cannot_be_shown_again
  # @test:the_exception_to_one_call_is_named_where_the_operator_reads_it
  Scenario: A plain line says what the token allows
    Given a token granted for one call
    When the operator reads the panel
    Then it says the token allows one call, bound to that call, and cannot be shown again
    And it names the one setting that makes a token repeat

  # @test:a_token_granted_on_a_hold_with_no_call_says_it_binds_none
  # @test:a_token_granted_on_a_hold_with_no_call_does_not_claim_to_be_bound_to_one
  Scenario: A token granted on a hold that named no call does not claim to be bound to one
    Given a hold that named no single call, a model call for instance
    When the operator grants it
    Then the panel says what the token is bound to instead

  # @test:the_copy_button_puts_the_raw_token_on_the_clipboard_and_says_so
  # @test:a_copy_the_browser_refuses_says_so_and_the_token_stays
  Scenario: The copy button copies the token, and a copy that failed says so
    Given a token on the screen
    When the operator presses copy
    Then the clipboard holds the raw token and the button says it was copied
    But a browser that refuses the copy gets a line saying to copy it by hand
    And the token stays on the screen

  # @test:a_grant_carries_the_claims_it_shows_in_both_token_formats
  # @test:a_token_signed_through_keyryx_decodes_from_its_middle_segment
  # @test:describe_decision_result_reads_a_token_signed_through_keyryx
  Scenario: A token signed through Keyryx shows what it authorises
    Given wardryx signs its tokens through Keyryx, as three segments
    When the operator grants a hold
    Then the panel shows the agent, the run, the tools and the countdown of that token

  # @test:a_grant_whose_claims_cannot_be_read_still_answers_with_the_raw_token
  # @test:a_token_whose_claims_could_not_be_read_is_still_shown
  Scenario: A token the console cannot read still reaches the operator
    Given a token whose claims this console cannot decode
    When the operator grants the hold
    Then the raw token is shown
    And the panel says it cannot tell what the token is bound to

  # @test:a_deny_answers_with_no_token
  # @test:a_deny_never_answers_with_a_token_even_if_wardryx_sent_one
  # @test:an_approval_already_decided_shows_no_token_a_second_time
  # @test:a_grant_answer_becomes_a_token_and_a_denial_does_not
  Scenario: Only a grant answers with a token, and only the first time
    Given an approval
    When it is denied, or decided a second time
    Then no token is shown

  # @test:the_raw_token_reaches_no_journal_row_no_bus_line_and_no_file
  # @test:a_grant_through_the_router_answers_the_raw_token_and_keeps_no_copy
  # @test:two_hundred_seeds_of_hostile_tokens_never_reach_the_journal_text
  # @test:a_decode_error_never_repeats_what_it_refused
  Scenario: The token reaches no journal row, no bus line and no file
    Given a grant that wrote its console_command
    When every file the console wrote is read back
    Then neither the token nor any segment of it is in any of them
    And a token that could not be decoded leaves no trace of itself either

  # @test:a_granted_token_reaches_no_log_line
  # @test:the_answer_prints_without_the_raw_token
  # @test:a_decide_response_prints_without_its_token
  # @test:the_raw_token_serialises_bare_and_prints_redacted
  Scenario: The token reaches no log line
    Given a console with every log level on
    When a grant runs through the router
    Then the process writes neither the token nor any segment of it
    And a printed answer names the token as redacted

  # @test:the_token_reaches_no_browser_storage_no_url_no_cookie_and_no_console_line
  Scenario: The token reaches no browser storage and no URL
    Given a token on the screen
    When the operator copies it and moves on
    Then no browser storage, cookie, history entry, URL or console line holds it

  # @test:leaving_the_view_drops_the_token_and_coming_back_does_not_restore_it
  # @test:dismissing_drops_the_token_and_it_cannot_be_read_back
  # @test:leaving_felyx_drops_the_token_a_card_showed
  # @test:an_expired_token_is_no_longer_drawn
  Scenario: The token is gone on navigation
    Given a token on the screen
    When the operator leaves the view, or dismisses it
    Then the document holds no copy of it
    And coming back does not bring it back
    And a token past its expiry is no longer drawn at all

  # @test:two_grants_in_a_row_keep_both_tokens_on_screen
  # @test:a_second_grant_never_takes_the_place_of_a_first_token
  Scenario: A second grant does not take the place of a first token
    Given a token the operator has not copied yet
    When the operator grants another hold
    Then both tokens are on the screen

  # @test:a_grant_from_a_felyx_card_shows_the_raw_token_to_the_operator
  # @test:felyx_never_receives_the_token_from_a_grant_on_its_own_card
  # @test:the_copilot_crate_cannot_name_the_token_or_the_call_that_mints_it
  Scenario: A grant from a Felyx card reaches the operator and not Felyx
    Given Felyx proposed granting a hold
    When the operator approves the card
    Then the token is shown beside the transcript
    And the transcript, the audit link and the next question do not carry it
    And the copilot crate does not name the token or the call that mints it

  # @test:a_row_shows_why_the_hold_happened_and_the_target_of_the_tool
  # @test:a_decided_row_keeps_the_reason_and_the_target
  # @test:the_queue_carries_the_reason_and_the_call_wardryx_sent
  # @test:approval_dto_carries_the_held_call_and_no_arguments
  # @test:a_held_tool_call_is_read_by_name_target_and_digest
  Scenario: Each row shows the reason for the hold and the target of the tool
    Given a call held with a reason and a target
    When the operator reads the inbox
    Then the row shows both, pending and decided

  # @test:a_row_says_wardryx_did_not_send_the_arguments
  # @test:the_arguments_hover_names_the_digest_a_grant_binds
  Scenario: A row says wardryx did not send the arguments
    Given wardryx records a call by name, target and digest and not its arguments
    When the operator reads the row
    Then the row says the arguments were not sent, in place of a blank
    And the hover names the digest a grant binds

  # @test:a_row_says_when_the_arguments_were_truncated
  # @test:the_approval_trail_carries_the_target_and_whether_the_arguments_were_truncated
  Scenario: A row shows that the arguments were truncated
    Given a call whose arguments the enforcement point cut short
    When the operator reads the row
    Then the row says so, and that a grant binds only the tool and the target
    And the approval trail carries it

  # @test:a_hold_with_no_single_call_says_so_instead_of_leaving_a_blank
  # @test:a_hold_with_no_reason_says_so_instead_of_leaving_a_blank
  # @test:a_target_that_did_not_arrive_is_not_recorded_and_an_empty_one_is_none_stated
  # @test:approval_dto_of_a_hold_with_no_call_says_null
  # @test:a_hold_with_no_tool_call_says_none_and_an_unreadable_one_says_unknown
  Scenario: A row never leaves a blank where the plane said nothing
    Given a hold with no tool call, with no reason, or with a call the console cannot read
    When the operator reads the row
    Then the row says which of those it is

  # @test:a_target_cannot_hide_part_of_itself_from_the_person_reading_it
  # @test:two_hundred_seeds_of_hostile_targets_never_reach_a_row_with_a_hidden_character
  # @test:two_hundred_seeds_of_hostile_contexts_never_cost_the_row
  Scenario: A target cannot hide part of itself from the person deciding
    Given a target an agent chose, with control or direction characters in it
    When the operator reads the row
    Then those characters are written out by code point
    And a hold's context of any shape costs the row nothing
