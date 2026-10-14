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

  # @test:the_demo_egress_panel_shows_example_rows_on_documentation_domains_only
  Scenario: The demo's Web Egress panel shows example rows, on documentation domains only
    Given the demo build, under the "Demo, simulated data" pill
    When the egress panel asks for its recent rows
    Then it shows fetches enforced per request and at the navigation only, and refusals
    And every host it names is an example.com, example.net or example.org documentation domain

  # @test:the_gateways_body_is_inset_like_the_other_dashboard_cards
  Scenario: The Gateways card keeps its rows off the card's edges
    Given the Overview's Gateways card, in the demo and in the console
    When it lists the gateways, or says it is still reading them
    Then its rows, pills and note are inset as the other dashboard cards are

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

  # @test:the_demo_passkey_operator_password_is_disabled
  Scenario: The demo passkey password field is disabled and says so
    Given the demo build with no box behind it
    When the Passkeys panel offers the first enrolment
    Then the operator password field is disabled with the placeholder "disabled in the demo"

  # @test:the_demo_add_passkey_still_answers_with_the_simulated_result
  Scenario: Adding a passkey in the demo still answers with the simulated result
    Given the demo build with its password field disabled
    When the operator presses Add passkey
    Then the panel shows that nothing was done

  # @test:the_real_passkey_operator_password_stays_enabled
  Scenario: The real console keeps the operator password field enabled
    Given the real web build talking to a box
    When the Passkeys panel offers the first enrolment
    Then the operator password field is enabled with its own placeholder

  # @test:the_demo_felyx_connect_answers_without_an_api_key
  Scenario: Connecting a cloud provider to Felyx in the demo answers without an API key
    Given the demo build with its API key field disabled
    When the operator presses Connect Felyx for a cloud provider
    Then the connect answers with the simulated result and asks for no key

  # @test:the_demo_secret_fields_are_disabled_and_say_so
  Scenario: Every secret field in the demo is disabled and says so
    Given the demo build with fields that take a password, key or token
    When the field is shown
    Then it is disabled with the placeholder "disabled in the demo"

  # @test:the_demo_key_requirement_is_waived_so_buttons_answer
  Scenario: A button that needs a key still answers in the demo
    Given the demo build with its key field disabled
    When a button that would need a key is pressed
    Then the requirement is waived and the button answers its simulated result

  # @test:every_view_insets_its_cards_and_spaces_them
  Scenario: Every view keeps its cards off the rails and apart from each other
    Given any view of the console, Web Egress and Statistics among them
    When it lays out its cards
    Then they are inset from the side rails and separated by the same gap as on Quality
