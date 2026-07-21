# Next live campaign: re-run, re-shoot, re-publish (planned 2026-07-20)

**Read this first, then `RESULTS.md` (the previous campaign's archive and runbook).**
This file is the complete brief for a fresh session. It assumes no prior conversation context.

---

## 1. Why we are re-running

The 2026-07-19 capture set is already published on the public site (`it-rat.com/enterprise.html`,
20 `.webp` files) and shipped to Tania. Two defects make it unusable long-term:

1. **The console Overview reads `0 killed of 9,288`.** Nobody ever called the kill endpoint in that
   dataset, so the counter is honestly zero, while the article and the site both claim a
   hardware-signed kill and a kill from the wrist. Visible contradiction on a public page.
2. **The phone/watch show a different dataset than the console.** The mobile shots came from a
   separate small seed (`gx_mobile_seed.py`, a second cloud instance on `:8083`): phone shows
   `1.22 $/min · spent $48.63`, console shows `$4,314`. There is no shared data flow.

Yurii's requirement for the re-run: **every screenshot must be maximally filled with real data, and
the data flow must be one coherent story across console tabs, iPhone simulator and Apple Watch
simulator, so a viewer can follow what was done, where it was verified, and how it looks.**

---

## 2. What Yurii provides (and what only he can do)

- **The box.** He provisions a Hetzner **CPX62-class** (16 vCPU / 32 GB, Ubuntu) with the public key
  below added *at create time*, gives the IP, and **tears the box down himself afterwards**.
- **SSH key (already generated, do not create another):**
  - private: `~/.ssh/hetzner-genaryx-20260720`
  - public: `~/.ssh/hetzner-genaryx-20260720.pub`
  - fingerprint line: `ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFpBcIOoNMLewthCaDFDULORht3vu1400RwZ9nYkdTLN genaryx-live-validation-20260720`
  - **Never delete any key on your own initiative.** "Tear down the box" is not permission to delete keys.
- **Anthropic API key (open decision).** Short-lived, only so Felyx thinks with a real model in the
  Copilot screenshot (as on 2026-07-19). Without it Felyx runs on a local model, still honest but a
  weaker proof. Cost is negligible (tens of calls).
- **One Touch ID press.** The console kill ceremony is hardware-signed; it needs his finger. Roughly
  five seconds of his time. Phone/watch kills are Face ID in the simulator and can be driven without him.
- **Site deploy approval.** The site is deployed by manual copy to `it-rat/it-rat.github.io`; deploy
  only on his explicit go.

---

## 3. Architecture decision that shapes the whole run

**Do NOT shrink the fleet, and do NOT use a second small mobile dataset.** Yurii's correction:
the phone is a pager, not a fleet browser. It must show only critical situations (over cap, near cap,
runaway). That is exactly D12's design, and it is already built on `main`:

- `crates/relay/src/exceptions.rs` - `ExceptionClass` (`over_cap`, `near_cap`, `runaway`, ...),
  `is_hard()` = `OverCap | Runaway` (hard events push unfiltered, never suppressible),
  `classify_fraction()`, `ExceptionSnapshot { queue }`.
- Relay routes (`crates/relay/src/main.rs`): `GET /relay/v1/exceptions`, `POST /relay/v1/pair`,
  `GET /v1/summary`, `POST /v1/runs/{run}/kill`, `POST /v1/runs/{run}/budget`,
  `POST /v1/incidents/{id}/ack`, plus `/admin/*` (device, disconnect, pairing-info, pairing-window).
- iOS side already consumes it: `ios/Sources/ExceptionQueueView.swift`, `RelayModels.swift`
  (decodes `/relay/v1/exceptions`, mirrors `is_hard` exactly), `RelayPairing.swift`, `PinnedTLS.swift`.

So the chain is: **cloud holds the full fleet → relay computes the exception slice → phone/watch show
only exceptions, plus `/v1/summary` fleet totals.** The big numbers therefore match the console by
construction, and the phone lists stay short and readable. This also demonstrates the pager idea on
the site instead of "another dashboard in your pocket".

**Validate this chain FIRST, before shooting anything**: bring the relay up, pair the phone by QR,
confirm the exception slice arrives from the full fleet and that `/v1/summary` matches the console.
If it does not line up, say so before spending hours on captures.

---

## 4. Sim-first constraints (checked on `main`, 2026-07-20)

- `crates/relay/src/license.rs`: the ML-DSA licence gate is **bypassed in sim mode** (TODO R1). The
  relay starts without a licence.
- `crates/relay/src/push.rs`: push is a **NullSender** that only logs "would push to device token ...".
  Real APNs is R1 work and needs the Apple Developer account, which Yurii deliberately deferred to the
  end (after simulator proof).
- Therefore the notification screenshot is produced with a locally injected payload (`simctl push`).
  This is visually identical because iOS renders the banner either way. **Never claim the notification
  was delivered by our relay over APNs.** Use the same banner text the relay actually composes
  (kind / run id / incident), so the shot stays truthful.
- The only hard startup requirement for the relay is a reachable cloud health check.

---

## 5. The one story every screenshot must tell

Protagonist: `agent://meridian.example/treasury/reconciliation-batch`, the month-end reconciliation
that fans out into ~150 shards and keeps retrying an oversized ledger context. The 2026-07-19 shots
already use `reconciliation-batch-eod-002-LIVE`; keep that identifier so the story is recognisable.

Thread it through, in this order, with the **same run id and the same fleet numbers visible**:

| # | Surface | What must be visible |
|---|---|---|
| 1 | Console · Overview | fleet spend, governed savings, incidents, **non-zero "killed" counter** |
| 2 | Console · Money | the run in the board, over cap, with its history |
| 3 | Phone · notification | banner naming the same run (`budget_exhausted xN`), over the app |
| 4 | Phone · exception queue | only over-cap / near-cap items, fleet burn matching `/v1/summary` |
| 5 | Phone · run detail | slide-to-kill armed |
| 6 | Phone · Face ID | the confirm ceremony |
| 7 | Phone · killed | the same run now killed |
| 8 | Watch · fleet | same burn number as the phone |
| 9 | Watch · kill signed | same run id, signed by the watch key |
| 10 | Console · Overview again | killed counter incremented, incident acknowledged |
| 11 | Console · Policy | the approval this agent tripped ($48 over the $25 treasury threshold) |
| 12 | Console · Identity | the same agent, delegation chain, detector alerts |
| 13 | Console · Copilot (Felyx) | asks about this incident, proposes killing this exact run, cites ids |
| 14 | Console · Replay / Evidence / Graph / Bus Explorer | all about the same run |
| 15 | Menu bar · Genaryx Bus | same total, "last runaway" = the same agent |

---

## 6. Four planes were EMPTY last time and must be really seeded

`RESULTS.md`: "Verdryx (quality) + engram (memory) stores were NOT seeded this run (empty)". That is
why `gx-quality.webp` and `gx-memory.webp` look thin on the site. **Write new seeders and run the real
tools on the box** (no fabricated rows):

- **Verdryx (quality)**: real eval runs + scores + cost-per-correct over this fleet's traces.
  Genaryx reads Verdryx's SQLite directly (it has no JSON CLI output).
- **Engram (memory)**: real episodes/facts for these agents, including a `why()` provenance answer.
  Genaryx reads Engram over MCP stdio (tools: stats, recall, why, remember, forget).
- **Qryx (crypto)**: a real scan of the deployed stack producing CBOM findings and PQC risk.
- **Mockryx (drills)**: a fire drill replaying this exact runaway pattern, showing the guardrail held.

New scripts to add next to the existing ones in `scripts/`: `gx_quality.py`, `gx_memory.py`,
`gx_crypto.sh`, `gx_drills.sh`.

---

## 7. Capture protocol (this is what prevents the current mismatch)

1. Finish ALL state changes first (seed everything, perform the kills).
2. Then capture every screenshot **in one session**, without touching the data in between.
3. **At the same moment**, run `gx_verify.sh` and save the metric dump.
4. **Write every number in the article and on the site from that dump only.** Never from memory,
   never from an older archive. The dump goes into the new `RESULTS.md`.
5. Capture at a window size where text is still legible after the site's `.webp` downscale; verify by
   opening the final file at the size it is displayed on the page, not by eyeballing the original.

---

## 8. Rollout targets (everything that carries these numbers or images)

- **Article 10 (Enterprise)**, both languages, three formats each:
  - `~/Development/Вихідники статей/10-Enterprise/{UA,EN}/Enterprise-{ua,en}.md`
  - `~/Development/Статті для Тані/10-Enterprise/Enterprise-{ua,en}.html` + re-rendered `.pdf`
- **Explainer for Tania** (its Genaryx section quotes $4,314 / $2,993 / $2,370):
  `~/Development/Пояснення для Тані/TAIPANBOX-пояснення-для-Тані.html` + re-rendered `.pdf`
- **Screenshot folders (4), each with its `_Порядок і підписи.txt` index**:
  `Статті для Тані/01-General/Скріншоти`, `Статті для Тані/10-Enterprise/Скріншоти`,
  `Брифи для Тані/Скріншоти`, `Пояснення для Тані/Скріншоти`
- **Public site**: `~/Development/it-rat/assets/shots/enterprise/*.webp` - **all 20 files are used by
  `enterprise.html`**: `gx-{overview,money,policy,identity,quality,crypto?,memory,evidence,graph,posture,copilot,bus}`,
  `ph-{connect,finops,incidents,alert,kill-arm,faceid,killed}`, `watch-{fleet,kill}`.
  Deploy = manual copy to `it-rat/it-rat.github.io`, **only on Yurii's explicit go**.
- **Campaign archive**: `~/Development/genaryx/live-campaign/` - new `shots/<date>/`,
  `enterprise-article-images/`, updated `RESULTS.md`, and the new seeder scripts.

---

## 9. Existing assets worth reusing as composition templates

The 2026-07-19 set (`shots/2026-07-19/`, 46 files) is well composed; re-shoot it rather than reinvent it:
16 console tabs (`01-overview` … `16-drills`), 8 mobile screens, watch (`w01-watch-fleet-overcap`,
`w02-watch-kill-signed`), a 5-step phone kill workflow + `phone-kill-flow.mp4`, and `workflow/WORKFLOW.md`.

`m07b-phone-notification-inapp.png` is the best template for the push shot (banner over the app naming
the run). When re-shooting it, fix: numbers must come from the shared dataset, avoid the `◀ Sphere`
back-link artifact in the status bar, and prefer the exception queue over the plain fleet list.

---

## 10. Standing rules that apply to this work

- **No long em dashes** anywhere (chat, docs, code comments, commit messages). Reword, or use a comma,
  colon or short hyphen.
- Ukrainian in conversation with Yurii; wiki content in English.
- **Never embed app screenshots inside Tania's documents.** They ship as a separate folder next to the
  HTML+PDF, each with its index. (He stated this twice.)
- Bold the stack service names in Ukrainian deliverables (**TokenFuse**, **Engram**, **Idryx**, **Qryx**,
  **Wardryx**, **Verdryx**, **Mockryx**, **Genaryx**, **Felyx**).
- Numbers must be verified from a dump or a screenshot, never invented or recalled.
- Honesty framing that must survive editing: the bank is modelled (`meridian.example`); the arithmetic,
  the governance and the console are real. State what a run did NOT cover rather than letting a reader assume.
- Print CSS for any regenerated PDF must keep headings with their content (`break-after: avoid` on
  headings/`.tech dt`/`.chip` as a block box, `orphans/widows: 3`), otherwise orphaned headings return.

---

## 11. Previous campaign's numbers (for reference only - REPLACE, do not reuse)

From the 2026-07-17/18 run: spend `$4,314.42`, prevented `$2,370.40`, cache `$358.20`, router `$264.10`,
total saved `$2,992.70`, budget breaks `180`, runs `9,287`, calls `34,824`, incidents `176`,
`reconciliation-batch` `$85.57` over `4,350` calls, 16 production agents + 1 runaway, 29 identities,
44 identity alerts, 6 policies, 5 pending approvals. The console screenshot of the same run reads
slightly higher (`9,288 / 178 / 34,839 / $2,994.86 / $2,372.56 / 181`) because it was read a moment later:
that drift is exactly the problem the capture protocol in section 7 removes.
