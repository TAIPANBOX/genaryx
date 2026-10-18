Feature: The console sends security headers on every response

  The console is a single page that holds the operator's control room. Until
  now none of its responses said that it may not be framed, that its content
  type must be trusted as sent, or what it may load.

  @decided 2026-10-17: every response, the API, the health probe, the static
  pages and a not-found answer alike, carries a content security policy that
  forbids framing and every source the console does not use, an
  X-Frame-Options of DENY, nosniff and a no-referrer policy.

  # @test:every_response_carries_the_security_headers
  Scenario: Every response carries the security headers
    Given the console router
    When the health probe, the session route and an unknown page are requested
    Then each answer carries a policy that sets frame-ancestors to none, X-Frame-Options DENY, nosniff and no-referrer
