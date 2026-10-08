# Phase 4 follow-up run 2026-07-18: WireGuard transport through the app, and a signed kill

Goal: close the two asterisks left by the 2026-07-17 campaign, where the channel
was a manual `ssh -L` rather than the app's own WireGuard transport, and where
no signed kill against the remote Cloud had been documented.

## RESULT: EXIT GATE PASSED (2026-07-18)

- The WireGuard tunnel was brought up BY THE APP ITSELF (native SwiftUI, run as root): keypair -> key exchange with the box -> `wireguard-go` -> handshake -> about 28 MB through `utun6`. Not `ssh -L`.
- All three planes reachable ONLY through the tunnel (`10.9.0.1:8080/8090/8081`): money $4,314, policy 6, identity 29.
- Control plane closed to the internet (`ufw`: only `:22` and `:51820/udp` public; `:8080` times out from outside).
- A hardware-signed kill (Touch ID + ES256 break-glass + an operator reason) of the runaway `cashflow-forecaster-0217` went THROUGH the WireGuard tunnel -> `killed=true` on the box, ACTIVE RUNS 9,289 -> 9,288.
- Both asterisks of 2026-07-17 are closed (the channel is the app's WireGuard transport; the signed kill went through that tunnel).
- **Bug found and fixed:** `wg.rs set_addr` on macOS used `ifconfig ... alias`, which silently assigns no IP on a utun -> replaced with `netmask 255.255.255.255`; test updated; connectors 98/98.
- **Feature gap (not blocking):** the app fixes the money descriptor at startup; switching planes to the active tunnel without a restart is a future clean fix (worked around in the campaign with a loopback -> WireGuard forward).

## Box state (ready, done autonomously)

- Box: Hetzner CPX62, `5.75.234.176` (Hetzner reissued the same IP as on 07-17; the host key
  is pinned in a new file, `~/.ssh/known_hosts_genaryx_followup_20260718`).
- SSH key (fresh): `~/.ssh/hetzner-genaryx-20260718` (the 07-17 key was left untouched).
- Stack via `stack-up`: Cloud `0.0.0.0:8080`, wardryx/idryx/gateway on `127.0.0.1`.
- Planes seeded (the figures match 07-17 deterministically):
  - money: $4,314.54 spent, $2,370.40 prevented, $2,992.70 saved, 180 breaks, 9,289 runs, 34,834 calls, 176 incidents.
  - policy: 6 policies + 5 pending approvals.
  - identity: meridian idryx on `127.0.0.1:8082`, 29 identities, 44 alerts.

## WireGuard server (kernel wg-quick on the box)

- `wg0` up: server addr `10.9.0.1/24`, listen `:51820`.
- Server pubkey: `4OhTOyJS92ml7CTrXxio1ziAPc+9m5CtpPPtHYOog2U=`
- Endpoint: `5.75.234.176:51820`
- socat forwards on wg0 (the services are localhost-only): `10.9.0.1:8090 -> 127.0.0.1:8090`,
  `10.9.0.1:8081 -> 127.0.0.1:8082` (meridian idryx), `10.9.0.1:4100 -> 127.0.0.1:4100`.
  The Cloud is already on `0.0.0.0`, so it is visible on `10.9.0.1:8080` through the tunnel.
- ufw: ONLY `22/tcp` + `51820/udp` public; the whole control plane is closed from outside
  (checked: `5.75.234.176:8080` times out). This is D11's "not exposed to internet" requirement.

## Values for the app's Remote panel (Mac client)

- WireGuard peer pubkey (server): `4OhTOyJS92ml7CTrXxio1ziAPc+9m5CtpPPtHYOog2U=`
- WireGuard endpoint: `5.75.234.176:51820`
- allowed-ips (what to route into the tunnel): `10.9.0.0/24`
- local (tunnel) address: `10.9.0.2/32`
- peer (tunnel) address: `10.9.0.1`
- keepalive: `25`
- wireguard-go bin: `~/.taipan/bin/wireguard-go` (installed locally; the connector finds it itself)

Service descriptor (through the tunnel): cloud `http://10.9.0.1:8080`, wardryx `http://10.9.0.1:8090`,
idryx `http://10.9.0.1:8081`, gateway `http://10.9.0.1:4100` (enforce).

## The step that needs the operator (root on the Mac)

Bringing up a utun on macOS needs root. In the app, `WgTunnel::bring_up` spawns
`wireguard-go`, which needs privileges for the tun device. Options for the operator:
1. Run the app under `sudo` (once, for the demo), or
2. Grant `wireguard-go` the right to create a utun in advance, or
3. Enter the sudo password once in the terminal when the app asks for it.

Once the app generates the console keypair and shows the console pubkey, add it
as a peer on the box (done by the agent as soon as it has the pubkey):

```
ssh -i ~/.ssh/hetzner-genaryx-20260718 -o UserKnownHostsFile=~/.ssh/known_hosts_genaryx_followup_20260718 \
  root@5.75.234.176 'wg set wg0 peer <CONSOLE_PUBKEY> allowed-ips 10.9.0.2/32 && wg show wg0'
```

## State at the end of the autonomous session (2026-07-18)

Done autonomously:
- Box up, three planes seeded (figures match), WireGuard server + socat forwards + ufw (control plane closed from outside, checked).
- UI redesign: all 14 tabs of both shells moved to the dashboard form + `FreshBadge` (LIVE/AUTO/SNAPSHOT/ON-DEMAND/WINDOW/PAUSED). Gates re-run in person and green: Tauri `tsc --noEmit` + `pnpm build`, SwiftUI `swift build` (all 45 files). The mutation models (kill/budget/grant/deny/forget) were NOT changed, only the view layers; Touch ID in place; the Identity 20s loop removed (parity fix). The work was NOT committed.
- To review the look with real data: persistent SSH forwards were brought up (8080/8090/8081->8082/4100, no root needed) and the native `Genaryx.app` restarted (the process picks the data up through the forward). This is a showcase of the look, NOT the WireGuard exit gate.

Waiting on the operator (root / presence):
- computer-use access to Genaryx was declined (`user_denied`), so native screenshots of the new tabs could not be taken autonomously.
- WireGuard tunnel through the Remote panel + signed kill: bringing up a utun on the Mac needs root (the sudo password), which the agent is not allowed to enter.

## Signed kill (exit gate)

Candidate: the live run `cashflow-forecaster-0217` (about $5.85, the largest unkilled one at the time
of the check). Through the app: Money -> the run's row -> Kill -> break-glass reason
(SwiftUI: + Touch ID) -> an ES256-signed `money_kill_run` goes IN THE TUNNEL to `10.9.0.1:8080`.
This closes "a hardware-signed kill against a remote client-hosted Cloud through the
Genaryx transport". Then verify the run is killed, and take a screenshot.
