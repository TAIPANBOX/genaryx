Feature: Felyx shows the output tokens a provider bills, reasoning included

  Measured 2026-10-07 against Vertex AI's OpenAI-compatible endpoint
  (gemini-2.5-flash, one answer): prompt 14, completion 59, reasoning 560,
  total 633. Google's completion count leaves the reasoning out and bills it
  at the output rate, so Felyx showed 59 output tokens for an answer a
  TokenFuse gateway in front charges 619 for.

  @decided 2026-10-08: Felyx counts output as the larger of the completion
  count and the total less the prompt, the rule TokenFuse and CostCrew already
  apply to the same answer.

  # @test:a_reasoning_model_usage_counts_its_reasoning_as_output
  Scenario: A thinking model's reasoning is shown as output
    Given an answer reports prompt 14, completion 59 and total 633
    When Felyx reads its usage
    Then it shows 14 prompt tokens and 619 output tokens

  # @test:an_openai_shaped_usage_is_read_unchanged
  Scenario: An answer whose completion already holds its reasoning is read as before
    Given an answer reports prompt 10, completion 25, total 35 and 20 reasoning tokens inside the completion
    When Felyx reads its usage
    Then it shows 25 output tokens, never the reasoning counted twice

  # @test:a_short_total_never_lowers_the_output_below_the_completion_count
  # @test:a_total_with_no_prompt_count_is_shown_as_output
  Scenario: A total that does not add up never hides output
    Given an answer whose total is short of prompt plus completion, or names no prompt count
    When Felyx reads its usage
    Then the output is never below the completion count
    And a total with no prompt count is shown whole as output

  # @test:hostile_usage_figures_saturate_and_never_wrap
  Scenario: An impossible count saturates instead of reading as a small one
    Given an answer reports counts past what Felyx keeps, or values that are not counts
    When Felyx reads its usage
    Then a count past the limit shows as the limit, not as a small wrapped number
    And a value that is not a count reads as absent
