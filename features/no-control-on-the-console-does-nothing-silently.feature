Feature: No control on the console does nothing and says nothing

  The agent card offered Transfer owner, Reassign unit, Edit budget and Edit
  behaviour. Each rests on a command only the mock preview answers: no box has
  an arm for `org_directory`, `agent_record` or the four edits, so a real box
  answered 404, the caller swallowed it into null, and the control did nothing.
  The same card hid the controls that do work on a box (freeze, kill, revoke,
  stop and lift), because it waited for a record no box keeps.

  @decided 2026-11-23: on a real box the budget and owner controls of the agent
  card are hidden or marked, and the store they need is designed separately.
  Nothing the console sends may be a command the box does not handle unless it
  is marked on purpose as answered only by the preview.

  # @test:a_real_box_does_not_offer_the_edits_it_cannot_make_and_says_why
  Scenario: A real box does not offer edits it cannot make, and says why
    Given the console is signed in to a real box
    When the operator opens an agent's card
    Then Transfer owner, Reassign unit, Edit budget and Edit behaviour are not offered
    And the card says those need an ownership store this box does not keep yet

  # @test:a_real_box_still_offers_the_controls_that_act_on_the_planes
  Scenario: The controls that work on a box stay on the card
    Given the console is signed in to a real box
    When the operator opens an agent's card
    Then Stop agent and Freeze are offered

  # @test:a_real_box_card_sends_none_of_the_preview_only_edit_commands
  # @test:a_real_box_refuses_every_preview_only_command_before_any_request
  Scenario: A command only the preview answers never leaves the browser on a box
    Given the console is signed in to a real box
    When any control would send a command only the preview answers
    Then no request is made
    And the caller is told the command is preview-only

  # @test:a_real_box_still_sends_a_command_it_has_an_arm_for
  Scenario: A command the box handles is still sent
    Given the console is signed in to a real box
    When a control sends a command genaryx-web has an arm for
    Then the request goes to the box

  # @test:a_refused_edit_in_the_preview_says_so_and_keeps_its_form
  Scenario: In the preview an edit that did not land says so
    Given the mock preview keeps an editable agent record
    When the operator applies an edit the backend refuses
    Then the form stays open
    And it says the change was not made and why

  # @test:the_preview_offers_all_four_edits_where_the_record_is_kept
  # @test:the_org_directory_which_no_box_answers_is_preview_only
  Scenario: The preview keeps offering the edits it can make
    Given the mock preview keeps an editable agent record
    When the operator opens an agent's card
    Then all four edits are offered
