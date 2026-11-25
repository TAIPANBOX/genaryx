Feature: The Record tab shows what Trailryx sealed, and verifies it on request

  Trailryx seals the stack's events into segments and Keyryx signs each store
  root. Whether the record is current, signed, and whether it verifies were
  questions for a shell on the box and two command lines.

  @decided 2026-11-23: the console gets read-only screens over files already on
  the box; Record shows Trailryx's last seal and verifies it.

  # @test:the_root_shown_is_the_root_trailryx_computes_for_one_two_and_fifty_segments
  # @test:the_last_seal_shows_segments_records_and_the_root
  Scenario: The last seal is read with the node's own decoder
    Given a data directory the node has sealed segments into
    When the operator opens Record
    Then the screen shows the segments, the records, the last segment and the store root
    And the root is the one Trailryx computes over those manifests

  # @test:a_signature_on_file_for_the_current_root_reads_as_signed_with_its_key_and_time
  # @test:a_signature_on_file_says_so_with_its_key_and_that_verify_proves_it
  # @test:a_signature_for_another_root_does_not_sign_this_one
  Scenario: A Keyryx signature on file for the current root is shown as on file
    Given the node stored a signature for the current root
    When the operator opens Record
    Then the screen names the key and the time, and says Verify is what proves it

  # @test:a_root_waiting_for_its_signature_reads_as_queued_never_as_signed
  # @test:a_root_waiting_for_its_signature_says_so
  Scenario: A root waiting for its signature says so
    Given the current root is queued for a signature
    When the operator opens Record
    Then the screen says it is waiting and never that it is signed

  # @test:a_box_with_no_data_dir_named_says_so_and_names_the_setting
  # @test:a_data_dir_not_named_says_so_and_names_the_setting
  # @test:the_data_dir_setting_is_an_absolute_path_with_no_parent_steps
  Scenario: A box that names no data directory says so
    Given no Trailryx data directory is named on the box
    When the operator opens Record
    Then the screen names GENARYX_TRAILRYX_DATA_DIR

  # @test:a_named_directory_that_is_not_there_is_unreadable_not_empty
  # @test:an_empty_directory_is_nothing_sealed
  # @test:an_unreadable_store_is_not_shown_as_nothing_sealed
  Scenario: An unreadable store is never shown as nothing sealed
    Given the named directory is missing, or a manifest in it cannot be read
    When the operator opens Record
    Then the screen says it could not read the store, and why

  # @test:a_linked_manifest_or_a_directory_named_like_one_makes_the_store_unreadable
  # @test:a_manifest_under_another_segments_name_makes_the_store_unreadable
  # @test:an_oversized_or_undecodable_manifest_makes_the_store_unreadable
  # @test:a_signing_file_this_console_cannot_read_is_unreadable_never_unsigned
  # @test:two_hundred_seeds_of_mutated_manifests_never_panic_and_never_read_under_another_segment
  # @test:two_hundred_seeds_of_hostile_signing_files_never_read_as_signed
  # @test:other_shards_are_counted_and_not_read
  Scenario: A file the node did not write the way it writes is refused, not skipped
    Given a linked, moved, oversized or corrupted manifest, or a malformed signing file
    When the operator opens Record
    Then the store or the signature reads as unreadable, never as smaller or unsigned

  # @test:not_run_says_not_verified_since_this_console_started
  Scenario: Verify runs only when asked
    When the operator opens Record and has not asked for a verification
    Then the screen says it has not been verified since the console started

  # @test:verify_packs_the_store_and_believes_only_an_agreeing_verified
  # @test:verify_now_calls_record_verify_and_shows_the_verdict
  # @test:the_two_steps_run_with_exact_arguments_and_path_alone
  Scenario: Verify packs the record and believes trailryx-verify only when it agrees with itself
    When the operator asks to verify
    Then the console packs the store and runs trailryx-verify with no shell and PATH alone
    And it shows Verified only when the exit code is 0 and the last line is VERIFIED

  # @test:not_verified_is_said_with_its_findings
  # @test:a_store_the_node_refuses_to_pack_is_refused_with_its_words
  # @test:a_store_the_node_refuses_reads_as_refused_in_red
  # @test:a_verdict_needs_the_exit_code_and_the_last_line_to_agree
  # @test:findings_are_bounded_and_only_the_bracketed_lines
  Scenario: A record that does not verify says so with its findings
    Given trailryx-verify answers NOT VERIFIED with exit 1
    Then the screen says not verified and lists the findings

  # @test:a_verified_line_that_is_not_the_last_is_not_a_verdict
  # @test:an_exit_code_and_a_last_line_that_disagree_could_not_be_checked
  # @test:a_pack_step_that_fails_or_overruns_could_not_be_checked
  # @test:a_check_that_could_not_be_made_says_it_is_not_a_verdict
  Scenario: A check that could not be made is neither verdict
    Given the pack step fails or overruns, or the verifier's answer disagrees with itself
    Then the screen says it could not be checked and that this is not a verdict

  # @test:a_busy_box_or_no_data_dir_runs_nothing
  Scenario: A busy box or an unnamed store runs nothing
    Given every proof slot is in use, or no data directory is named
    When the operator asks to verify
    Then no process starts and the screen says why

  # @test:a_verdict_for_an_earlier_root_says_so
  # @test:a_verdict_is_stale_when_the_root_shown_is_another_or_none
  # @test:a_verdict_with_no_readable_root_now_is_stale
  # @test:a_record_sealed_further_while_it_was_packed_is_no_verdict
  # @test:a_check_that_made_no_verdict_or_an_older_one_does_not_replace_the_kept_verdict
  Scenario: A verdict holds only for the root it was taken for
    Given the record was sealed further during or after the last verification, or can no longer be read
    Then a verification that saw the store move is no verdict
    And a kept verdict names its root and says it is not the root shown now
    And a later check that made no verdict does not replace it

  # @test:reading_the_record_and_verifying_it_are_viewer_reads
  # @test:the_only_control_is_verify
  # @test:the_record_tab_is_in_the_assure_section
  # @test:wire_text_renders_as_text_with_hidden_characters_written_out
  Scenario: Nothing on the Record tab writes to the record
    When the operator looks at the Record tab in Assure
    Then its only control is Verify, a viewer may use it, and it changes nothing
