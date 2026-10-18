Feature: The sensitive commands come from the command table

  Every command the console can run is placed once, with the role that may
  run it. The passkey ceremony is a property of the same row, so the list of
  commands that need a fresh confirmation cannot drift away from the list of
  roles. Until now the ceremony set was a second, hand-written list in the
  web router, and a command could be placed in one and not the other.

  @decided 2026-10-17: the ceremony set is one column of the command table.
  A command the table does not know asks for the ceremony, as it already
  asks for the admin role.

  # @test:the_ceremony_is_a_column_of_the_command_table
  Scenario: The ceremony set is exactly the commands the decision names
    Given the command table of the web console
    When the ceremony column is read
    Then it names the eight commands that move money, a delegation, a tunnel peer, a retained reservation, a unit cap or an approval

  # @test:an_unknown_command_is_treated_as_a_ceremony_command
  Scenario: A command the table does not know asks for the ceremony
    Given a name the command table does not place
    When the console asks whether that command needs a ceremony
    Then the answer is yes

  # @test:a_read_needs_no_ceremony_and_a_sensitive_write_does
  Scenario: Reads need no ceremony and sensitive writes do
    Given a read of the unit caps and a kill of a run
    When the console asks which of them needs a ceremony
    Then only the kill does, and an approval grant also does though its role is lower

  # @test:the_ceremony_list_is_written_once_in_the_command_table
  Scenario: No second ceremony list is kept in the router
    Given the web router source
    When it is searched for a list of sensitive commands of its own
    Then none is found
