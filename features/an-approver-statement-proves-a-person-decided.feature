Feature: An approval a person decided carries a statement the console signs

  The approval statement is the console's proof that a person, not a bearer
  key, decided an approval. It is a compact signature, made with the console
  key only after the person's passkey assertion has verified, and it names
  the approver, the credential, the approval and the decision. The verifier
  checks it against the console's published key set.

  @decided 2026-10-19: an approval decided through the console carries a
  statement the console signs after the person's passkey assertion. The
  statement names the approver, the credential, the approval and the decision,
  and its header pins the algorithm, the type and the key by thumbprint.

  # @test:a_statement_verifies_under_the_console_key_and_carries_its_claims
  Scenario: A statement verifies under the console key and returns its claims
    Given a console key
    When the console signs a statement for an approval
    Then the statement verifies under that key and gives back the same claims

  # @test:the_header_pins_es256_the_statement_type_and_the_console_thumbprint
  Scenario: The header pins the algorithm and the type, names the key, and carries no key of its own
    Given a signed statement
    When its header is read
    Then the algorithm is ES256, the type is the approver statement type, the key id is the console thumbprint, and no key or certificate is inside

  # @test:the_thumbprint_is_the_sha256_of_the_canonical_ec_members
  Scenario: The key id is the RFC 7638 thumbprint of the console key
    Given a console public key
    When its thumbprint is taken
    Then it is the SHA-256 of the canonical members of the key, base64url encoded

  # @test:a_tampered_payload_does_not_verify
  Scenario: A changed decision breaks the signature
    Given a signed statement that grants an approval
    When the decision in its payload is changed to deny
    Then the statement does not verify

  # @test:a_statement_under_another_console_key_does_not_verify
  Scenario: A statement made under another key is refused
    Given a statement made by one console key
    When it is checked against a different console key
    Then it does not verify

  # @test:claims_outside_what_wardryx_accepts_are_refused_before_anything_is_signed
  Scenario: Claims wardryx would refuse are never signed
    Given claims with no subject, a control character or bidi override in the subject, a credential id that is not base64url, an unknown decision, a lifetime over five minutes, or no statement id
    When the console tries to sign them
    Then nothing is signed and the claims are refused

  # @test:a_statement_outside_its_window_does_not_verify
  Scenario: A statement outside its window is refused
    Given a signed statement that expires at 1120 seconds and was issued at 1000
    When it is checked at 1120 seconds, or at 900 seconds
    Then it is refused as expired, or as issued in the future

  # @test:the_public_jwks_publishes_the_key_the_statement_names
  Scenario: The published key set names the key the statements use
    Given a console public key
    When the key set is published
    Then it holds one EC P-256 signing key whose id is the thumbprint that statements name
