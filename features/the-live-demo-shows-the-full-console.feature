Feature: The Live Demo shows the console's sidebar foot and says what it simulates

  The public Live Demo runs the real console over a mock transport. Before this,
  the signed-in session was unknown in that build, so the sidebar foot showed no
  role, no user and no Passkeys or "Connect this machine" entry. The demo now
  answers a fixed operator. Its actions answer plainly that nothing was done, and
  a panel that must never show invented data keeps its own honest note.

  The real web build is unchanged: it still asks the box who is signed in.

  # @test:the_demo_sidebar_foot_shows_the_signed_in_role_and_user
  Scenario: The demo sidebar foot shows the signed-in role and user
    Given the demo build with no box behind it
    When the sidebar foot is rendered
    Then it shows the role and the user of the demo operator

  # @test:the_demo_sidebar_foot_offers_passkeys_and_connect_this_machine
  Scenario: The demo sidebar foot offers Passkeys and Connect this machine
    Given the demo build with no box behind it
    When the sidebar foot is rendered
    Then it offers the Passkeys entry and the Connect this machine entry

  # @test:the_demo_build_answers_a_signed_in_admin_without_asking_a_box
  Scenario: The demo answers a fixed admin session without asking a box
    Given the demo build with no box behind it
    When the session hook renders for the first time
    Then it reports a signed-in admin and asks no box for a session

  # @test:the_web_build_still_asks_the_box_for_the_session
  Scenario: The real web build still asks the box for the session
    Given the real web build talking to a box
    When the session hook renders and its effects run
    Then it asks the box for the session once

  # @test:a_bare_preview_has_no_session_and_asks_nothing
  Scenario: A bare preview has no session and asks nothing
    Given a preview with neither a box nor the demo flag
    When the session hook renders and its effects run
    Then it reports no session and asks no box

  # @test:the_demo_lists_no_passkeys_and_asks_no_box
  Scenario: The demo lists no passkeys and asks no box
    Given the demo build with no box behind it
    When the passkey list is read
    Then it is empty and no box is asked

  # @test:the_demo_passkey_enrollment_says_nothing_was_done
  Scenario: Enrolling a passkey in the demo says nothing was done
    Given the demo build with no box behind it
    When the operator enrols a passkey
    Then the answer says it was simulated and nothing was done

  # @test:the_demo_passkey_removal_says_nothing_was_done
  Scenario: Removing a passkey in the demo says nothing was done
    Given the demo build with no box behind it
    When the operator removes a passkey
    Then the answer says it was simulated and nothing was done

  # @test:quality_baselines_answer_the_summary_shape_the_panel_reads
  Scenario: The demo's Quality baselines have the shape the panel reads
    Given the demo build with its simulated baselines
    When the Quality panel reads the baselines
    Then every row carries its source run and a flag for unanswered cases

  # @test:the_demo_egress_panel_holds_no_invented_rows
  Scenario: The demo's Web Egress panel holds no invented rows
    Given the demo build, where the egress panel has no demo data by design
    When the egress panel asks for its recent rows
    Then it is told there is no backend and shows no rows

  # @test:the_demo_tunnel_connect_says_nothing_was_done
  Scenario: Opening a tunnel in the demo says nothing was done
    Given the demo build with no box behind it
    When the operator asks the tunnel to connect
    Then the tunnel is not shown as connected and the answer says nothing was done

  # @test:the_demo_ssh_file_read_says_nothing_was_read
  Scenario: Reading a remote file in the demo says nothing was read
    Given the demo build with no box behind it
    When the operator reads a remote file
    Then the content says nothing was read from any box

  # @test:the_demo_wireguard_issue_and_revoke_leave_the_peer_list_as_it_was
  Scenario: Issuing and revoking WireGuard peers in the demo leaves the device list as it was
    Given the demo build with its simulated device list
    When the operator asks for a config and asks to revoke a device
    Then the device list is unchanged and both requests are refused

  # @test:the_demo_felyx_connect_says_nothing_was_done
  Scenario: Connecting Felyx in the demo says nothing was done
    Given the demo build with no box behind it
    When the operator connects a provider to Felyx
    Then the provider is unchanged and the answer says nothing was done

  # @test:the_demo_marks_its_example_panels_as_simulated
  Scenario: The demo marks its example panels as simulated
    Given the demo build with example data in a remote panel
    When the panel is rendered
    Then a plain line says the data is simulated and no box was contacted

  # @test:the_real_console_renders_no_simulated_notice
  Scenario: The real console renders no simulated notice
    Given the real web build talking to a box
    When the same panel is rendered
    Then no simulated line appears
