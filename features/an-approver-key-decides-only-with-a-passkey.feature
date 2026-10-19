Feature: An approval decided through the console carries a statement only a passkey can produce

  When the operator gives the console an approver key file, an approval is
  decided only with a statement the console signs after the person's passkey
  assertion has verified. The statement names the person, the credential, the
  approval and the decision. A decision that cannot carry one is refused and
  nothing is sent to wardryx.

  @decided 2026-10-19: with an approver key file configured, a decision is
  refused unless a passkey confirmed it and the statement can be made. With no
  key file configured, no statement is sent and the console says so.

  # @test:a_confirmed_grant_is_signed_for_this_approval_and_decision
  Scenario: A confirmed grant carries a statement for that approval and decision
    Given an approver key file and a passkey that confirmed the grant
    When the console makes the statement for the approval
    Then the statement verifies under the console key and names the approval, the grant, the person and the credential

  # @test:a_confirmed_deny_is_signed_as_deny
  Scenario: A confirmed deny carries a statement that says deny
    Given an approver key file and a passkey that confirmed the deny
    When the console makes the statement for the approval
    Then the statement names the decision deny

  # @test:with_a_key_file_a_decision_no_passkey_confirmed_is_refused
  Scenario: With a key file, a decision no passkey confirmed is refused
    Given an approver key file and no passkey assertion for the decision
    When the console makes the statement for the approval
    Then the decision is refused and nothing is sent to wardryx

  # @test:an_unusable_key_file_refuses_the_decision_and_says_why
  Scenario: An unusable key file refuses the decision and names the reason
    Given an approver key file that could not be read
    When a passkey confirms a decision
    Then the decision is refused with the reason, and nothing is sent to wardryx

  # @test:a_statement_the_claims_refuse_is_refused_not_sent
  Scenario: A statement whose claims wardryx would refuse is refused, not sent
    Given an approver key file and a decision with no approver subject
    When the console makes the statement for the approval
    Then the decision is refused and nothing is sent to wardryx

  # @test:with_no_key_file_no_statement_is_made_and_nothing_is_refused
  Scenario: With no key file, no statement is made and the decision goes on
    Given no approver key file is configured
    When a decision is made with or without a passkey assertion
    Then no statement is made and the decision is not refused

  # @test:a_statement_is_sent_as_approver_statement_when_one_was_made
  Scenario: The decision body carries the statement under its wire name
    Given a grant and a statement that was made
    When the decision body is built for wardryx
    Then the statement is in the field approver_statement beside the decision

  # @test:no_statement_means_the_field_is_absent_from_the_body
  Scenario: The decision body omits the statement field when none was made
    Given a deny and no statement
    When the decision body is built for wardryx
    Then the body has no approver_statement field

  # @test:loading_without_a_path_is_not_configured
  Scenario: No key file path means the approver key is not configured
    Given no approver key file path
    When the console loads its approver key
    Then the key is not configured

  # @test:loading_a_pkcs8_key_file_is_ready
  Scenario: A PKCS#8 key file loads as the console key
    Given a P-256 key in a PKCS#8 PEM file
    When the console loads its approver key
    Then the key is ready to sign

  # @test:loading_a_missing_or_malformed_key_file_is_failed_not_silent
  Scenario: A missing or malformed key file is reported, not ignored
    Given a key file that is missing, or that holds no key
    When the console loads its approver key
    Then the key is marked failed, with a reason

  # @test:a_pkcs8_pem_key_loads_to_the_scalar_it_holds
  Scenario: A P-256 PKCS#8 PEM loads to the scalar it holds
    Given a P-256 PKCS#8 PEM with no public half
    When it is read as a signing key
    Then the signing key holds the same scalar

  # @test:a_pkcs8_key_with_its_public_half_loads_when_the_two_match
  Scenario: A key file that carries its public half loads when the halves match
    Given a P-256 PKCS#8 PEM that carries its public half
    When it is read as a signing key
    Then the key loads

  # @test:a_key_whose_public_half_differs_from_its_scalar_is_refused
  Scenario: A key whose public half does not match its scalar is refused
    Given a P-256 PKCS#8 PEM whose public half belongs to another key
    When it is read as a signing key
    Then the key is refused

  # @test:a_key_that_is_not_p256_pkcs8_pem_is_refused
  Scenario: Anything that is not a P-256 PKCS#8 PEM is refused
    Given text that is not a P-256 PKCS#8 PEM, an RSA or SEC1 key, or an empty body
    When it is read as a signing key
    Then the key is refused

  # @test:the_jwks_of_a_key_file_names_the_key_the_statements_use
  Scenario: The published key set names the key the statements use
    Given an approver key file
    When the approver-jwks command prints its public key set
    Then the key id is the thumbprint that the statements name

  # @test:the_jwks_refuses_a_missing_key_file
  Scenario: The approver-jwks command refuses a missing key file
    Given a key file that does not exist
    When the approver-jwks command runs on it
    Then it refuses and prints no key set

  # @test:the_approver_key_file_comes_from_its_own_variable
  Scenario: The key file is named by its own environment variable
    Given the environment names an approver key file
    When the console reads its settings
    Then the file is the one the variable names, and a blank value names none

  # @test:a_leftover_require_passkey_variable_is_announced_as_no_longer_read
  Scenario: A leftover requirement variable is announced as no longer read
    Given the retired passkey requirement variable is still set
    When the console starts
    Then it logs that the variable is no longer read and that passkeys are required unless the software fallback is allowed
