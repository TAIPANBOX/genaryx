Feature: A provider key pasted where its reference belongs is not repeated

  GENARYX_COPILOT_API_KEY_REF takes env:NAME or file:/path, and the console
  reads the value as a reference to the provider's key. An operator who pastes
  the key itself into that setting gets Felyx reported as disabled, and the
  console shows that reason to any signed-in viewer. The reason must say what
  the setting accepts, and it must not repeat what the setting holds, because
  what it holds is the key.

  # @test:an_api_key_ref_that_holds_the_key_is_refused_without_repeating_it
  Scenario: The provider key is pasted where the reference belongs
    Given GENARYX_COPILOT_API_KEY_REF holds a provider key instead of env:NAME or file:/path
    When the console builds Felyx
    Then Felyx reports itself disabled with a reason that names the setting and the two accepted forms
    And the reason, in its text and in its debug form, contains neither the key nor the setting's value
