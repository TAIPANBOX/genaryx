Feature: A sealed case is one canonical document, and its digest is in the console's chain

  A case that was assembled, proved and decided is worth something later only if
  what was seen can be shown to be what was seen. The case, the proofs asked for
  it and the person's decision are one document written in the form RFC 8785
  fixes, so another implementation can reproduce its bytes, and the sha256 of
  those bytes is the target of one console_command line in the console's own
  chain, which the bus readers already pack and sign.

  @decided 2026-11-04: no new service and no new event type; the existing
  console_command carries the action case_close with the digest as its target. The
  server assembles the case and asks for the proofs again, so nothing in the pack
  is the browser's word. A seal that cannot be journaled stores nothing. An
  application of the remedy outside the console is the operator's statement, and
  the pack says whose. Sealing changes nothing in any plane.

  # @test:members_are_sorted_by_utf16_code_units_not_by_bytes
  # @test:the_form_has_no_whitespace_and_the_members_are_in_order
  # @test:strings_are_escaped_the_way_json_stringify_escapes_them
  # @test:numbers_are_written_the_way_ecmascript_writes_them
  # @test:the_canonical_text_reads_back_to_a_value_with_the_same_canonical_text
  Scenario: The canonical form is the one RFC 8785 fixes
    Given JSON with members in any order, awkward strings, floats and nested values
    When it is written in canonical form
    Then the members are sorted by UTF-16 code units, there is no whitespace, strings and numbers are written as ECMAScript writes them
    And the text reads back to a value that writes the same text

  # @test:an_integer_a_double_cannot_hold_is_refused_and_one_it_can_is_written_whole
  # @test:a_pack_with_a_number_a_double_cannot_hold_or_past_the_bound_is_not_sealed
  Scenario: A number the form would round is refused, and so is a pack past its bound
    Given an integer past 2^53 - 1 and a pack larger than a mebibyte
    When each is sealed
    Then each is refused, so a digest never depends on a rounding

  # @test:the_same_inputs_make_the_same_bytes_and_the_same_digest
  # @test:the_digest_is_the_sha256_of_the_canonical_text_and_nothing_else
  # @test:the_same_case_sealed_twice_at_the_same_time_is_the_same_pack
  Scenario: The same inputs make the same document and the same digest
    Given a case, its proofs and a decision, and the time they are sealed at
    When they are sealed twice
    Then the bytes and the digest are the same, and the digest is the sha256 of those bytes
    And the pack holds no digest of itself

  # @test:sealing_journals_the_digest_as_the_target_and_stores_the_pack
  Scenario: The digest is the target of a chain line, and a file is named by it
    Given an incident in the console's store
    When its case is sealed with a decision
    Then the console's chain gains one console_command line whose action is console.case_close and whose target is the digest
    And a private file named by the digest holds the same document, and is handed back by case_pack

  # @test:the_proofs_that_could_not_be_made_are_in_the_pack_as_what_they_are
  Scenario: A proof that could not be made is sealed as that, never as a proof
    Given proofs asked for on a box with no archive and no trace directory
    When the case is sealed
    Then each proof is in the pack with its request and the cause it could not be made

  # @test:a_seal_that_cannot_be_journaled_stores_nothing
  Scenario: A seal that is not in the chain does not exist
    Given a console with no live event bus
    When a case is sealed
    Then the seal is refused and no file is stored

  # @test:a_pack_is_the_pack_its_digest_names_only_if_it_is_whole_canonical_and_of_this_format
  # @test:a_stored_pack_that_was_altered_is_not_handed_back
  Scenario: A document is the pack its digest names only if it hashes to it
    Given a stored pack that was altered, reformatted, reordered, of another format or replaced
    When it is read back
    Then each is refused with the way it differs

  # @test:only_a_sha256_in_lowercase_hex_is_a_digest
  # @test:a_digest_that_is_not_a_digest_never_reaches_the_filesystem
  Scenario: Only a sha256 in lowercase hex is ever turned into a file name
    Given digests with a path, the wrong length, capitals or another algorithm
    When a pack is asked for by each
    Then each is refused as not a digest before the filesystem is touched

  # @test:a_decision_is_trimmed_bounded_and_free_of_control_characters
  # @test:a_proof_spec_and_a_decision_read_from_the_shape_the_command_takes_and_nothing_more
  # @test:a_request_that_is_not_sealed_is_refused_before_the_case_is_read
  # @test:an_incident_the_store_does_not_hold_is_not_sealed
  Scenario: A request that is not sealable is refused by name
    Given a note that is too long or holds a control character, more than four proofs, unknown fields and an incident the store does not hold
    When the case is sealed
    Then each is refused before anything is written

  # @test:sealing_a_case_is_an_approvers_act_and_reading_a_pack_is_a_read
  Scenario: Sealing is an approver's act and reading a pack is a read
    Given a viewer and an approver
    When they ask to seal and to read
    Then sealing needs an approver and no ceremony, and reading needs a viewer
