Feature: The session cookie is Secure when the console is served over TLS

  A session cookie without the Secure attribute is sent over plain HTTP too.
  The console's own origin says whether a person reaches it over TLS, so the
  cookie should follow that origin rather than rely on a flag someone must
  remember to set.

  @decided 2026-10-17: the session cookie is Secure whenever the configured
  origin is https, or whenever the operator asks for it explicitly. A console
  on a plain HTTP origin, the tunnel case, keeps its cookie as it was.

  # @test:the_session_cookie_is_secure_over_tls
  Scenario: A console whose origin is https sets a Secure session cookie
    Given a console whose configured origin is https
    When an operator signs in with the right password
    Then the session cookie is marked Secure

  # @test:the_session_cookie_stays_plain_over_plain_http
  Scenario: A console on a plain HTTP origin keeps its cookie as it was
    Given a console whose configured origin is plain http
    When an operator signs in with the right password
    Then the session cookie is not marked Secure

  # @test:the_cookie_is_secure_when_the_origin_is_https_or_asked_for
  Scenario: The cookie decision follows the origin or an explicit request
    Given an https origin, and a plain http origin with and without an explicit request
    When the cookie decision is taken for each
    Then the https origin and the explicit request are Secure and the plain origin alone is not
