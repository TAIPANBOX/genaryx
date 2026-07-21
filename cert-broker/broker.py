#!/usr/bin/env python3
"""The Pocket cert broker - design (A), production build (itrat-console/14).

One job: mediate the ACME DNS-01 challenge for a relay so the relay can obtain a
publicly trusted certificate for <relay-id>.pocket.it-rat.com WITHOUT ever
holding the DNS-zone credential, and WITHOUT the broker ever seeing the relay's
private key.

  relay (ACME client, genaryx-relay/src/acme.rs)   broker (this)        DNS zone
  ----------------------------------------------   ------------         --------
  generate keypair locally
  order <id>.pocket.it-rat.com
  receive the DNS-01 challenge value
  POST /present {fqdn, value}  ----------------->   authenticate relay
                                                    check fqdn is THIS relay's
                                                    set the _acme-challenge TXT -> live
  tell the CA "ready"; CA validates the TXT
  download the signed cert (key stays on relay)
  POST /cleanup {fqdn, value}  ----------------->   remove the TXT           -> gone

Two security properties this enforces:
  1. The DNS credential lives ONLY on the broker. The relay never gets it.
  2. The relay's private key lives ONLY on the relay. The broker only ever
     handles the challenge token, never a CSR or a key.

The broker also enforces that a relay may publish a challenge ONLY for its own
subdomain (`_acme-challenge.<relay-id>.pocket.it-rat.com`), authenticated by a
per-relay token; relay A cannot obtain relay B's certificate.

The DNS backend is PLUGGABLE (BROKER_BACKEND):
  - `challtestsrv` : pebble-challtestsrv's REST API - the local proof/test path.
  - `cloudflare`   : the real pocket.it-rat.com zone via a SCOPED token
                     (Zone.DNS:Edit on that one zone only) that lives ONLY here.

Wire format is lego's `httpreq` provider (default mode) and genaryx-relay's own
BrokerClient: POST {fqdn, value} -> publish a TXT at `fqdn` with contents `value`.

Auth: HTTP Basic, username = relay id, password = the relay's broker token.
Relays are loaded from BROKER_RELAYS_FILE (JSON: {"<relay-id>": "<token>"}).
"""
import base64
import hmac
import json
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ZONE = os.environ.get("BROKER_ZONE", "pocket.it-rat.com")
BACKEND = os.environ.get("BROKER_BACKEND", "challtestsrv")
CHALLTESTSRV = os.environ.get("CHALLTESTSRV_URL", "http://127.0.0.1:8055")
CF_TOKEN = os.environ.get("CLOUDFLARE_API_TOKEN", "")
CF_ZONE_ID = os.environ.get("CLOUDFLARE_ZONE_ID", "")
CF_API = "https://api.cloudflare.com/client/v4"

# A relay id is one DNS label: it becomes `<id>.pocket.it-rat.com` and the Basic
# auth username. Constrain it so a careless id cannot make a malformed challenge
# name or an unauthenticatable user (a colon would break Basic auth).
RELAY_ID_RE = re.compile(r"^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$")
MAX_BODY = 64 * 1024


def _http(url, method="GET", body=None, headers=None, timeout=10):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method, headers=headers or {})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        raw = r.read()
    return json.loads(raw) if raw else {}


# --- DNS backends -----------------------------------------------------------
# Each backend implements set_txt(fqdn, value) and clear_txt(fqdn, value). The
# broker never persists a key or a CSR; it only publishes/removes the TXT.


class ChalltestsrvBackend:
    """pebble-challtestsrv's REST API. Local proof/test only."""

    def set_txt(self, fqdn, value):
        _http(f"{CHALLTESTSRV}/set-txt", "POST",
              {"host": fqdn, "value": value},
              {"Content-Type": "application/json"})

    def clear_txt(self, fqdn, value):
        _http(f"{CHALLTESTSRV}/clear-txt", "POST",
              {"host": fqdn}, {"Content-Type": "application/json"})


class CloudflareBackend:
    """The real zone via a SCOPED Cloudflare token that lives only here.

    Requires CLOUDFLARE_API_TOKEN (Zone.DNS:Edit on the delegated
    pocket.it-rat.com zone) and CLOUDFLARE_ZONE_ID. The token never leaves this
    process and is never logged.
    """

    def __init__(self):
        if not CF_TOKEN or not CF_ZONE_ID:
            raise RuntimeError(
                "cloudflare backend needs CLOUDFLARE_API_TOKEN + CLOUDFLARE_ZONE_ID")
        self._auth = {"Authorization": f"Bearer {CF_TOKEN}",
                      "Content-Type": "application/json"}

    def _call(self, path, method="GET", body=None):
        """A Cloudflare API call that treats `success:false` (returned with
        HTTP 200) as the error it is, surfacing the message instead of letting a
        refused publish look like success."""
        result = _http(f"{CF_API}/zones/{CF_ZONE_ID}{path}", method, body, self._auth)
        if not result.get("success", True):
            msgs = "; ".join(e.get("message", str(e)) for e in (result.get("errors") or []))
            raise RuntimeError(f"cloudflare {method} {path}: {msgs or 'request failed'}")
        return result

    def _records(self, fqdn):
        name = urllib.parse.quote(fqdn.rstrip("."), safe="")
        return self._call(f"/dns_records?type=TXT&name={name}").get("result") or []

    def set_txt(self, fqdn, value):
        name = fqdn.rstrip(".")
        try:
            self._call("/dns_records", "POST",
                       {"type": "TXT", "name": name, "content": value, "ttl": 60})
        except RuntimeError:
            # Idempotent: if the exact record already exists (a retried present),
            # that is success, not a failure.
            if not any(r.get("content") == value for r in self._records(fqdn)):
                raise

    def clear_txt(self, fqdn, value):
        # Remove only OUR record (this name + this challenge value), never a
        # concurrent order's live TXT at the same name. Best-effort: a stale
        # challenge TXT is harmless.
        for rec in self._records(fqdn):
            if rec.get("content") == value and rec.get("id"):
                try:
                    self._call(f"/dns_records/{rec['id']}", "DELETE")
                except Exception:
                    pass


def load_backend():
    if BACKEND == "challtestsrv":
        return ChalltestsrvBackend()
    if BACKEND == "cloudflare":
        return CloudflareBackend()
    sys.exit(f"unknown BROKER_BACKEND={BACKEND!r} (challtestsrv|cloudflare)")


def load_relays():
    """relay id -> token. From BROKER_RELAYS_FILE (JSON), else a single dev
    relay so the proof flow can run. Ids must be a single DNS label."""
    path = os.environ.get("BROKER_RELAYS_FILE", "")
    relays = {"proof01": "proof-relay-token"}
    if path and os.path.exists(path):
        with open(path) as f:
            relays = json.load(f)
    for rid in relays:
        if not RELAY_ID_RE.match(rid):
            sys.exit(f"invalid relay id {rid!r}: must match {RELAY_ID_RE.pattern}")
    return relays


BACKEND_IMPL = load_backend()
RELAYS = load_relays()


class Handler(BaseHTTPRequestHandler):
    def _reply(self, code, msg=""):
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps({"ok": code < 400, "msg": msg}).encode())

    def log_message(self, fmt, *args):
        sys.stderr.write("broker: " + (fmt % args) + "\n")

    def _relay_id(self):
        """Authenticate the caller. Returns the relay id or None. The token is
        compared in constant time so it cannot be recovered by timing."""
        h = self.headers.get("Authorization", "")
        if not h.startswith("Basic "):
            return None
        try:
            user, _, pw = base64.b64decode(h[6:]).decode().partition(":")
        except Exception:
            return None
        expected = RELAYS.get(user)
        if expected is None:
            return None
        return user if hmac.compare_digest(expected, pw) else None

    def _allowed(self, relay_id, fqdn):
        """A relay may only touch the _acme-challenge for its OWN subdomain."""
        want = f"_acme-challenge.{relay_id}.{ZONE}."
        return fqdn.rstrip(".") + "." == want

    def do_POST(self):
        relay_id = self._relay_id()
        if not relay_id:
            return self._reply(401, "unknown relay")
        try:
            n = int(self.headers.get("Content-Length", 0))
            if n < 0 or n > MAX_BODY:
                return self._reply(400, "bad content-length")
            body = json.loads(self.rfile.read(n) or b"{}")
        except (ValueError, json.JSONDecodeError):
            return self._reply(400, "bad request")
        fqdn = body.get("fqdn", "")
        value = body.get("value", "")
        if not self._allowed(relay_id, fqdn):
            return self._reply(403, f"{relay_id} may not touch {fqdn}")
        try:
            if self.path == "/present":
                BACKEND_IMPL.set_txt(fqdn, value)
                return self._reply(200, f"set {fqdn}")
            if self.path == "/cleanup":
                BACKEND_IMPL.clear_txt(fqdn, value)
                return self._reply(200, f"cleared {fqdn}")
        except Exception as e:
            return self._reply(502, f"dns backend: {e}")
        self._reply(404, "no such route")


if __name__ == "__main__":
    port = int(os.environ.get("BROKER_PORT", sys.argv[1] if len(sys.argv) > 1 else 9000))
    bind = os.environ.get("BROKER_BIND", "127.0.0.1")
    print(f"pocket cert broker on {bind}:{port} -> backend={BACKEND} "
          f"zone={ZONE} relays={list(RELAYS)}", flush=True)
    ThreadingHTTPServer((bind, port), Handler).serve_forever()
