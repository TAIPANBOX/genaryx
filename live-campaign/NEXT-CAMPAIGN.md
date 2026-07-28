# Next live campaign: re-run, re-shoot, re-publish (planned 2026-07-20)

**Read this first, then `RESULTS.md` (the previous campaign's archive and runbook).**
This file is the complete brief for a fresh session. It assumes no prior conversation context.

---

## 1. Why we are re-running

The 2026-07-19 capture set is already published on the public site (`it-rat.com/enterprise.html`,
20 `.webp` files) and shipped to Tania. Two defects make it unusable long-term:

1. **The console Overview reads `0 killed of 9,288`.** Nobody ever called the kill endpoint in that
   dataset, so the counter is honestly zero, while the article and the site both claim a
   hardware-signed kill. Visible contradiction on a public page.
2. **Shots were taken against more than one dataset**, so the numbers on one surface do not add up
   against the numbers on another. There was no shared data flow behind them.

Yurii's requirement for the re-run: **every screenshot must be maximally filled with real data, and
the data flow must be one coherent story across the console tabs, so a viewer can follow what was
done, where it was verified, and how it looks.**

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
- **One passkey ceremony.** The console kill is signed per action; it needs him at the keyboard.
  Roughly five seconds of his time.
- **Site deploy approval.** The site is deployed by manual copy to `it-rat/it-rat.github.io`; deploy
  only on his explicit go.

---

## 3. The one story every screenshot must tell

Protagonist: `agent://meridian.example/treasury/reconciliation-batch`, the month-end reconciliation
that fans out into ~150 shards and keeps retrying an oversized ledger context. The 2026-07-19 shots
already use `reconciliation-batch-eod-002-LIVE`; keep that identifier so the story is recognisable.

Thread it through, in this order, with the **same run id and the same fleet numbers visible**:

| # | Surface | What must be visible |
|---|---|---|
| 1 | Console · Overview | fleet spend, governed savings, incidents, **non-zero "killed" counter** |
| 2 | Console · Money | the run in the board, over cap, with its history |
| 3 | Console · Money, run detail | the kill armed on that run |
| 4 | Console · the passkey ceremony | the per-action confirmation |
| 5 | Console · Money again | the same run now killed |
| 6 | Console · Overview again | killed counter incremented, incident acknowledged |
| 7 | Console · Policy | the approval this agent tripped ($48 over the $25 treasury threshold) |
| 8 | Console · Identity | the same agent, delegation chain, detector alerts |
| 9 | Console · Copilot (Felyx) | asks about this incident, proposes killing this exact run, cites ids |
| 10 | Console · Replay / Evidence / Graph / Bus Explorer | all about the same run |

---

## 4. Four planes were EMPTY last time and must be really seeded

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

## 5. Capture protocol (this is what prevents the current mismatch)

1. Finish ALL state changes first (seed everything, perform the kills).
2. Then capture every screenshot **in one session**, without touching the data in between.
3. **At the same moment**, run `gx_verify.sh` and save the metric dump.
4. **Write every number in the article and on the site from that dump only.** Never from memory,
   never from an older archive. The dump goes into the new `RESULTS.md`.
5. Capture at a window size where text is still legible after the site's `.webp` downscale; verify by
   opening the final file at the size it is displayed on the page, not by eyeballing the original.

---

## 6. Rollout targets (everything that carries these numbers or images)

- **Article 10 (Enterprise)**, both languages, three formats each:
  - `~/Development/Вихідники статей/10-Enterprise/{UA,EN}/Enterprise-{ua,en}.md`
  - `~/Development/Статті для Тані/10-Enterprise/Enterprise-{ua,en}.html` + re-rendered `.pdf`
- **Explainer for Tania** (its Genaryx section quotes $4,314 / $2,993 / $2,370):
  `~/Development/Пояснення для Тані/TAIPANBOX-пояснення-для-Тані.html` + re-rendered `.pdf`
- **Screenshot folders (4), each with its `_Порядок і підписи.txt` index**:
  `Статті для Тані/01-General/Скріншоти`, `Статті для Тані/10-Enterprise/Скріншоти`,
  `Брифи для Тані/Скріншоти`, `Пояснення для Тані/Скріншоти`
- **Public site**: `~/Development/it-rat/assets/shots/enterprise/*.webp`, the `gx-*` set the Genaryx
  page renders. Deploy is a push to `it-rat/it-rat.github.io`, **only on Yurii's explicit go**.
- **Campaign archive**: `~/Development/genaryx/live-campaign/` - new `shots/<date>/`,
  `enterprise-article-images/`, updated `RESULTS.md`, and the new seeder scripts.

---

## 7. Existing assets worth reusing as composition templates

The 2026-07-19 console set (`shots/2026-07-19/`, `01-overview` … `16-drills`) is well composed;
re-shoot it rather than reinvent it. The one thing to fix when re-shooting: every number must come
from the one shared dataset, so the same figure reads the same on every tab.

---

## 8. Standing rules that apply to this work

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

## 9. Previous campaign's numbers (for reference only - REPLACE, do not reuse)

From the 2026-07-17/18 run: spend `$4,314.42`, prevented `$2,370.40`, cache `$358.20`, router `$264.10`,
total saved `$2,992.70`, budget breaks `180`, runs `9,287`, calls `34,824`, incidents `176`,
`reconciliation-batch` `$85.57` over `4,350` calls, 16 production agents + 1 runaway, 29 identities,
44 identity alerts, 6 policies, 5 pending approvals. The console screenshot of the same run reads
slightly higher (`9,288 / 178 / 34,839 / $2,994.86 / $2,372.56 / 181`) because it was read a moment later:
that drift is exactly the problem the capture protocol in section 7 removes.
