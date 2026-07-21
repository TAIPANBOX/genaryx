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

The DNS backend is PLUGGABLE (BROKER_BACKEND):
  - `challtestsrv` : pebble-challtestsrv's REST API - the local proof/test path.
  - `cloudflare`   : the real pocket.it-rat.com zone via the Cloudflare API,
                     using a SCOPED token (Zone.DNS:Edit on that one zone only)
                     that lives ONLY here. Switching Pebble -> Let's Encrypt is
                     purely a relay-side directory URL; switching the mock ->
                     Cloudflare is purely this backend flag. Nothing else moves.

Wire format is lego's `httpreq` provider (default mode): the client POSTs
{fqdn, value} and the broker publishes a TXT at `fqdn` with contents `value`.
genaryx-relay's own BrokerClient speaks the same shape, so one broker serves
both the relay and lego (used to prove the flow during bring-up).

Auth model: HTTP Basic, username = relay id, password = the relay's broker
token. The broker maps the id to the ONE subdomain that relay may touch and
refuses any other fqdn. Relays are loaded from BROKER_RELAYS_FILE (JSON:
{"<relay-id>": "<token>"}); each relay gets its own token at provisioning.
"""
import base64
import json
import os
import sys
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ZONE = os.environ.get("BROKER_ZONE", "pocket.it-rat.com")
BACKEND = os.environ.get("BROKER_BACKEND", "challtestsrv")
CHALLTESTSRV = os.environ.get("CHALLTESTSRV_URL", "http://127.0.0.1:8055")
CF_TOKEN = os.environ.get("CLOUDFLARE_API_TOKEN", "")
CF_ZONE_ID = os.environ.get("CLOUDFLARE_ZONE_ID", "")
CF_API = "https://api.cloudflare.com/client/v4"


def _http(url, method="GET", body=None, headers=None, timeout=10):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method, headers=headers or {})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        raw = r.read()
    return json.loads(raw) if raw else {}


# --- DNS backends -----------------------------------------------------------
# Each backend implements set_txt(fqdn, value) and clear_txt(fqdn). The broker
# never persists a key or a CSR; it only publishes/removes the challenge TXT.


class ChalltestsrvBackend:
    """pebble-challtestsrv's REST API. Local proof/test only."""

    def set_txt(self, fqdn, value):
        _http(f"{CHALLTESTSRV}/set-txt", "POST",
              {"host": fqdn, "value": value},
              {"Content-Type": "application/json"})

    def clear_txt(self, fqdn):
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

    def _records(self, fqdn):
        # Cloudflare stores the name without the trailing dot.
        name = fqdn.rstrip(".")
        url = f"{CF_API}/zones/{CF_ZONE_ID}/dns_records?type=TXT&name={name}"
        return _http(url, "GET", headers=self._auth).get("result", [])

    def set_txt(self, fqdn, value):
        name = fqdn.rstrip(".")
        _http(f"{CF_API}/zones/{CF_ZONE_ID}/dns_records", "POST",
              {"type": "TXT", "name": name, "content": value, "ttl": 60},
              self._auth)

    def clear_txt(self, fqdn):
        for rec in self._records(fqdn):
            rid = rec.get("id")
            if rid:
                try:
                    _http(f"{CF_API}/zones/{CF_ZONE_ID}/dns_records/{rid}",
                          "DELETE", headers=self._auth)
                except urllib.error.URLError:
                    pass  # cleanup is best-effort; a stale TXT is harmless


def load_backend():
    if BACKEND == "challtestsrv":
        return ChalltestsrvBackend()
    if BACKEND == "cloudflare":
        return CloudflareBackend()
    sys.exit(f"unknown BROKER_BACKEND={BACKEND!r} (challtestsrv|cloudflare)")


def load_relays():
    """relay id -> token. From BROKER_RELAYS_FILE (JSON), else a single dev
    relay so the proof flow can run out of the box."""
    path = os.environ.get("BROKER_RELAYS_FILE", "")
    if path and os.path.exists(path):
        with open(path) as f:
            return json.load(f)
    return {"proof01": "proof-relay-token"}


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
        """Authenticate the caller. Returns the relay id or None."""
        h = self.headers.get("Authorization", "")
        if not h.startswith("Basic "):
            return None
        try:
            user, _, pw = base64.b64decode(h[6:]).decode().partition(":")
        except Exception:
            return None
        return user if RELAYS.get(user) == pw else None

    def _allowed(self, relay_id, fqdn):
        """A relay may only touch the _acme-challenge for its OWN subdomain."""
        want = f"_acme-challenge.{relay_id}.{ZONE}."
        return fqdn.rstrip(".") + "." == want

    def do_POST(self):
        relay_id = self._relay_id()
        if not relay_id:
            return self._reply(401, "unknown relay")
        n = int(self.headers.get("Content-Length", 0))
        try:
            body = json.loads(self.rfile.read(n) or b"{}")
        except Exception:
            return self._reply(400, "bad json")
        fqdn = body.get("fqdn", "")
        value = body.get("value", "")
        if not self._allowed(relay_id, fqdn):
            return self._reply(403, f"{relay_id} may not touch {fqdn}")
        try:
            if self.path == "/present":
                BACKEND_IMPL.set_txt(fqdn, value)
                return self._reply(200, f"set {fqdn}")
            if self.path == "/cleanup":
                BACKEND_IMPL.clear_txt(fqdn)
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
