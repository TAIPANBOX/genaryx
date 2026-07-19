# TokenFuse Pocket - phone + watch UX workflow (for the website animation)

Captured 2026-07-19 from the live iOS + watchOS simulators against real data
(box Cloud `meridian.example` for the rich money views over an SSH tunnel; a
light 35-run fleet on the local Cloud for the killable Runs/pager, because the
9,287-run meridian fleet is too heavy for the phone/watch Runs view to render).

Everything the mobile app confirms in code, shown end to end: install -> connect
by QR -> get data -> every tab -> critical alert -> kill an agent with Face ID,
on the phone and on the watch.

## The story, in order (frames + motion clips)

Each numbered still is one animation beat; the `.mp4`s are the real on-screen
motion for those beats (screen-recorded from the simulator), ready to embed as
`<video autoplay muted loop>`.

1. **Install / first launch** - `p01-connect-onboarding.png`
   "TokenFuse Pocket - Scan the QR on your Genaryx desktop to connect this
   iPhone." Scan-QR button + paste-link + manual options; the security note:
   a signing key is generated on the iPhone, the relay cert is pinned from the
   QR itself (a mismatch aborts pairing).
2. **Paired -> live data** - `p02-runs-overcap.png`
   Fleet burn rate, spend, per-run fuses with over-cap runs in ember.
3. **Every tab (with scroll)** - `phone-tabs-flow.mp4` (31s)
   Fleet -> FinOps -> Incidents -> Governance, scrolling to reveal the data
   below each. Rich meridian versions of these tabs are the stills one folder
   up: `../mobile/m03-finops-savings-meridian.png` ($2,994.86 saved),
   `m04-agents-meridian.png`, `m05-incidents-meridian.png`,
   `m06-governance-euaiact.png` (EU AI Act mapping, 34,839 decisions).
4. **Critical alert** - `../mobile/m07-phone-notification.png`
   The pager push: "Agent running hot - reconciliation-batch-eod-002-LIVE over
   budget - budget_exhausted x13. Tap to review and kill."
5. **Kill an agent, confirmed by Face ID** - the hero, both as stills and one clip:
   - `p03-run-detail-slidekill.png` - the run's spend + "Slide to arm kill",
     "Kill is signed by this device - Face ID".
   - `p04-faceid-confirm-kill.png` - the Face ID prompt after arming.
   - `p05-killed-result.png` - the run flips to KILLED in the list.
   - `phone-kill-flow.mp4` (20s) - the whole ceremony in motion:
     tap run -> slide to arm -> Face ID -> matched -> KILLED.

## On the watch (mirrors the phone, wrist-sized)

- `../mobile/w01-watch-fleet-overcap.png` - fleet burn + over-cap runs in ember (the critical glance).
- `../mobile/w02-watch-kill-signed.png` - the on-wrist kill: run spend + a red "Kill run", "Signed on this Apple Watch" (kill signed by the watch's own device key).
- `watch-flow.mp4` (29s) - the watch loads the fleet, then opens the kill screen.

## Notes for whoever assembles the page

- Face ID is real (`Biometrics.confirm`, LocalAuthentication); on the sim it is
  enrolled and approved with a Matching Face. The kill/acknowledge/budget
  actions all gate on it.
- The push alerts were delivered to the simulator with `simctl push` (no real
  APNs; a physical-device push needs an Apple Developer account).
- Money figures differ between the rich meridian tab stills ($4,314 fleet) and
  the kill/pager clips (light $41 fleet) on purpose - the heavy fleet does not
  render on a phone/watch Runs view, so the killable demo uses the small fleet.
