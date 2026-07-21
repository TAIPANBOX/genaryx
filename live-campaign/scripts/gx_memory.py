#!/usr/bin/env python3
# Genaryx live-validation memory-plane seeder - Engram (episodes, facts, recall, why).
#
# Why this script exists
# -----------------------
# RESULTS.md from the 2026-07-17/18 run: "Verdryx (quality) + engram (memory)
# stores were NOT seeded this run (empty)." A little was added since (5
# episodes, 0 facts - see the "before" stats this script prints), but the
# store still has no semantic facts and far too few episodes to make the
# Memory tab's timeline, recall, and provenance views mean anything. This
# script writes the rest, for real: every id it prints comes back from a
# genuine Engram write (embedding computed by the real fastembed model,
# inserted into the real SQLite + sqlite-vec store at ~/.taipan/engram.engram),
# never a row inserted by hand.
#
# Genaryx never calls remember() - and neither does this script, quite
# --------------------------------------------------------------------
# crates/connectors/src/engram.rs (read 2026-07-20, not modified) spawns the
# real `engram-mcp` binary over stdio and wraps four of its five tools:
# stats, recall, why, forget. Its module doc says why remember is missing on
# purpose: "agents write their own memories; a governance console does not
# fabricate them." This script plays the part of those agents - it is not
# Genaryx, it is a stand-in for the 17 meridian.example agents each recording
# what they actually did.
#
# Two different layers of "the repo's own API", used for two different jobs:
#   - WRITING: engram.core.Engram.observe()/.assert_fact() directly (the
#     public API), not the MCP `remember` tool. Read engram/mcp_server.py's
#     _remember(): it only forwards content/subject/predicate/object, never
#     actors/tags/salience/confidence/source, so a memory written through it
#     would be less specific than what Engram itself can record. Since the
#     task is to make these memories "specific and plausible", the richer
#     public API is the right tool, and it is exactly what _remember calls
#     into one layer down.
#   - READING (recall/why/stats): engram.mcp_server's own private functions
#     _recall/_why/_stats, imported directly rather than reimplemented. This
#     matters most for why(): the PUBLIC Engram.why(fact_id) (engram/core.py)
#     only ever looks up FACTS and raises KeyError on an episode id. The
#     dual-kind lookup (fact first, then episode, with the encoding/access
#     metadata shape genaryx's EngramProvenance::Episodic expects) exists
#     only in mcp_server.py's module-level _why(). Calling it directly here
#     (in-process, not over a spawned stdio pipe) exercises the exact same
#     code the real engram-mcp server runs for Genaryx, without the extra
#     moving part of hand-rolled MCP JSON-RPC framing.
#   All four (_EngramPool, _recall, _why, _stats) are plain, independently
#   testable functions per mcp_server.py's own module docstring - not
#   FastMCP-wrapped, so importing them never requires the `mcp` SDK
#   (confirmed: `mcp` is only imported lazily inside _build_server()).
#
# Why 17 separate Engram instances, and why that's correct here
# -----------------------------------------------------------------
# engram/store.py: every write (insert_episode, and every per-agent read) is
# scoped by the STORE's own bound agent_id, set once at Store construction
# from Engram(agent_id=...) - never by a field on the Episode object handed
# to insert_episode. So the only way to have episodes come out tagged with
# 17 different real agent ids is 17 different (pooled, lazily-created,
# cached-by-agent-id) Engram instances over the same file, which is exactly
# what _EngramPool (engram/mcp_server.py) already is, and exactly what a
# long-lived engram-mcp process serving 17 different callers would do.
# Facts are the one exception, by design: assert_fact() takes no agent_id at
# all and the `facts` table has no agent_id column (DESIGN.md 11: facts are
# "shared across agents"), so every fact below is written once through a
# single pooled instance regardless of which agent is asserting it.
#
# Unscoped reads still see everything written under a scope
# -------------------------------------------------------------
# engram/store.py Store.episode_count(): `if self._agent_id is not None:
# WHERE agent_id = ? ... else: SELECT COUNT(*) FROM episodes` - the ELSE
# branch is a global count, not a "WHERE agent_id IS NULL" count. Recall's
# SQL filters the same way. So stats(agent_id=None) / recall(agent_id=None)
# - what this script uses throughout, and almost certainly what Genaryx's
# default Memory view uses too, since genaryx-live's env config carries no
# --agent-id for engram-mcp - correctly aggregate across all 17 agents this
# script writes to, even though each one was written through its own scoped
# instance.
#
# Re-running this script
# ------------------------
# Safe, but additive, not idempotent. observe()/assert_fact() mint a fresh
# uuid4() id on every call (engram/core.py), so a second run appends another
# full copy of this narrative under new ids - it will not corrupt the store,
# will not touch the 5 episodes / 0 facts already there, and costs nothing
# (observe/assert_fact never call an LLM - DESIGN.md's "no network calls at
# write time" - only reflect() does, and this script never calls reflect()).
# Only re-run deliberately if you want the story duplicated.
import json
import time
from pathlib import Path

from engram.mcp_server import _EngramPool, _recall, _stats, _why  # noqa: E402  (see header)

DB_PATH = str(Path.home() / ".taipan" / "engram.engram")
ORG = "meridian.example"
HERO_RUN = "reconciliation-batch-eod-002-LIVE"


def A(team: str, name: str) -> str:
    return f"agent://{ORG}/{team}/{name}"


HERO_AGENT = A("treasury", "reconciliation-batch")

# ----------------------------------------------------------------------
# Episodic memories: what these agents actually observed. Numbers match the
# rest of this campaign exactly - HERO_RUN, the 150-shard fan-out, the three
# named over-cap shards (s007/s063/s128), the ~$93/4,428-call settlement,
# and the $48.00-est-vs-$25.00-threshold approval hold all come from
# gx_fleet_v3.py's HERO_RUN generation and gx_policy_seed.py's pending
# approval for the same run id - this is the same incident, seen from the
# memory plane instead of the money/policy planes.
#
# Each tuple: (agent_id, content, actors, tags, salience). emotional_valence
# is left at 0.0 throughout (Engram.observe's default) - these are
# governance/ops observations, not sentiment.
# ----------------------------------------------------------------------

EPISODES = [
    # --- the incident, in order -------------------------------------------------
    (HERO_AGENT,
     "Started end-of-day reconciliation batch reconciliation-batch-eod-002-LIVE "
     "for the 2026-07 close, fanning out across 150 shards against the general "
     "ledger.",
     ["reconciliation-batch"], ["incident", "reconciliation", "start"], 0.6),
    (HERO_AGENT,
     "First call on reconciliation-batch-eod-002-LIVE pulled a roughly "
     "210K-token ledger context; settled at allow, cost about $0.62.",
     ["reconciliation-batch"], ["incident", "cost"], 0.55),
    (HERO_AGENT,
     "Shards began retrying: several were re-requesting the same oversized "
     "ledger window because the first pass did not resolve the discrepancy.",
     ["reconciliation-batch"], ["incident", "retry"], 0.7),
    (HERO_AGENT,
     "By the 40-minute mark, dozens of the 150 shards were retrying against a "
     "ledger context that kept growing instead of narrowing, each retry "
     "re-sending close to 200K input tokens.",
     ["reconciliation-batch"], ["incident", "retry", "root-cause"], 0.8),
    (HERO_AGENT,
     "Per-run budget ceiling tripped on reconciliation-batch-eod-002-LIVE "
     "itself: allow decisions stopped after the 12th call, every call after "
     "that returned budget_exceeded.",
     ["reconciliation-batch"], ["incident", "budget", "governance"], 0.85),
    (HERO_AGENT,
     "Shard reconciliation-batch-eod-002-s007 breached its own per-run budget "
     "ceiling and was pushed into the exception queue as an over-cap incident.",
     ["reconciliation-batch"], ["incident", "budget", "exception-queue"], 0.75),
    (HERO_AGENT,
     "Shard reconciliation-batch-eod-002-s063 breached its per-run budget "
     "ceiling too, same retry pattern as s007: oversized context, no "
     "convergence.",
     ["reconciliation-batch"], ["incident", "budget", "exception-queue"], 0.75),
    (HERO_AGENT,
     "Shard reconciliation-batch-eod-002-s128 breached its per-run budget "
     "ceiling; three shards now over cap, all fed by the same oversized-context "
     "problem.",
     ["reconciliation-batch"], ["incident", "budget", "exception-queue"], 0.75),
    (HERO_AGENT,
     "Governance held reconciliation-batch-eod-002-LIVE for human approval: "
     "estimated cost $48.00 exceeded the treasury-human-approval policy's "
     "$25.00 threshold.",
     ["reconciliation-batch", "v.koval"], ["incident", "policy", "approval"], 0.9),
    (HERO_AGENT,
     "v.koval reviewed the pending approval for reconciliation-batch-eod-002-LIVE; "
     "by the time the ceiling stopped it, the run had settled only about $93 "
     "across 4,428 calls.",
     ["reconciliation-batch", "v.koval"], ["incident", "approval", "ruling"], 0.9),
    (HERO_AGENT,
     "Across all 150 shards the pattern held: one real attempt against the "
     "ledger, then a wall of budget_exceeded retries that governance blocked "
     "before they could compound the spend.",
     ["reconciliation-batch"], ["incident", "summary"], 0.7),
    (HERO_AGENT,
     "Post-mortem note: the oversized ledger context, not a defect in the "
     "reconciliation logic itself, is the leading theory for the retry storm; "
     "flagged for eod-003's context-window budget.",
     ["reconciliation-batch"], ["incident", "post-mortem", "root-cause"], 0.85),
    # --- prior closes, for contrast ---------------------------------------------
    (HERO_AGENT,
     "Recalled the prior month-end close, reconciliation-batch-eod-001: every "
     "shard finished in a single pass, no budget breaches, ledger context "
     "stayed well under 50K tokens.",
     ["reconciliation-batch"], ["history", "comparison"], 0.5),
    (HERO_AGENT,
     "eod-001 settled in under two hours with governance never once pausing a "
     "shard for approval, a sharp contrast with eod-002's retry storm.",
     ["reconciliation-batch"], ["history", "comparison"], 0.5),
    (HERO_AGENT,
     "Noted for next month: eod-001's ledger context stayed flat because the "
     "source ledger export was pre-filtered before the run started; eod-002's "
     "export was not, and that is the leading theory for the blow-up.",
     ["reconciliation-batch"], ["history", "root-cause"], 0.6),
    # --- normal work, the other 16 agents ---------------------------------------
    (A("fraud", "fraud-triage-copilot"),
     "Reviewed a flagged wire transfer for a first-time payee, escalated to a "
     "tier-2 fraud analyst after the pattern matched prior mule-account "
     "activity.",
     ["fraud-triage-copilot"], ["fraud", "triage"], 0.45),
    (A("fraud", "txn-anomaly-scorer"),
     "Scored 1,842 transactions in the 03:00 UTC batch, flagged 6 as anomalous "
     "(z-score above 3.5), most cache-served at near-zero marginal cost.",
     ["txn-anomaly-scorer"], ["fraud", "scoring"], 0.4),
    (A("kyc-aml", "kyc-intake-agent"),
     "Extracted passport MRZ fields for onboarding case KY-88213, confidence "
     "0.97, no manual review needed.",
     ["kyc-intake-agent"], ["kyc", "extraction"], 0.4),
    (A("kyc-aml", "sanctions-screener"),
     "Cleared 1,204 names against the OFAC SDN list update; zero true matches, "
     "one false positive on a common transliteration overturned on review.",
     ["sanctions-screener"], ["kyc", "sanctions"], 0.45),
    (A("kyc-aml", "aml-case-copilot"),
     "Compiled the case narrative for SAR-2026-0714, cited 12 supporting "
     "transactions, held for human sign-off under the aml-max-steps policy.",
     ["aml-case-copilot", "t.fedirko"], ["kyc", "aml", "case"], 0.5),
    (A("lending", "underwriting-copilot"),
     "Wrote the credit rationale for application UW-0231, recommended approval "
     "at 6.2% APR citing DTI 0.31 and a 730 FICO score.",
     ["underwriting-copilot", "s.tkachenko"], ["lending", "rationale"], 0.45),
    (A("lending", "doc-intake-ocr"),
     "OCR'd 214 pages of loan collateral documents; 3 flagged for manual "
     "re-scan due to low confidence on stamped seals.",
     ["doc-intake-ocr"], ["lending", "ocr"], 0.4),
    (A("lending", "collateral-valuator"),
     "Valued the commercial property collateral for loan LN-4471 at $2.1M "
     "using the October comparable set.",
     ["collateral-valuator"], ["lending", "valuation"], 0.4),
    (A("support", "support-tier1-bot"),
     "Resolved 340 chat sessions in the evening shift, escalated 9 to tier-2, "
     "mean handle time 94 seconds.",
     ["support-tier1-bot"], ["support", "tier1"], 0.35),
    (A("support", "support-tier2-bot"),
     "Took over an escalated billing dispute from tier-1, resolved it after "
     "confirming a duplicate charge and issuing a refund.",
     ["support-tier2-bot", "o.marchenko"], ["support", "tier2"], 0.45),
    (A("support", "escalation-router"),
     "Routed 58 tier-1 escalations to the correct specialist queue; one "
     "misroute corrected after a queue-tag typo.",
     ["escalation-router"], ["support", "routing"], 0.35),
    (A("treasury", "cashflow-forecaster"),
     "Produced the T+3 liquidity forecast for treasury, flagged a projected "
     "shortfall in the EUR sweep account.",
     ["cashflow-forecaster", "v.koval"], ["treasury", "forecast"], 0.5),
    (A("treasury", "spend-optimizer"),
     "Recommended shifting 20% of the kyc-aml document-extraction workload to "
     "a cheaper cached tier, projected about $40/day in savings.",
     ["spend-optimizer", "v.koval"], ["treasury", "finops"], 0.45),
    (A("compliance", "model-risk-validator"),
     "Completed the quarterly model-risk review of underwriting-copilot's "
     "rationale generator; no material findings.",
     ["model-risk-validator", "n.boiko"], ["compliance", "model-risk"], 0.5),
    (A("compliance", "control-tester"),
     "Ran the quarterly SOX control test over the reconciliation approval "
     "workflow; the control operated effectively.",
     ["control-tester"], ["compliance", "controls"], 0.45),
    (A("compliance", "evidence-assembler"),
     "Assembled the audit evidence package for the Q2 model-risk review, 42 "
     "artifacts indexed.",
     ["evidence-assembler"], ["compliance", "evidence"], 0.4),
]

# Index of the episode (0-based, into EPISODES above) that becomes the
# episodic why() demonstration: the approval-hold moment, the single episode
# that most directly explains the incident's climax.
WHY_EPISODE_INDEX = 8

# ----------------------------------------------------------------------
# Semantic facts: fleet structure. subject/predicate/object triples, written
# via assert_fact() (manual, no LLM - confidence 1.0, not an extracted
# guess). Policy facts name the exact policies gx_policy_seed.py registers in
# wardryx (treasury-human-approval, underwriting-approval, aml-max-steps,
# kyc-require-attestation, support-spend-cap) so this plane and the policy
# plane agree with each other instead of inventing a second set of names.
# Each tuple: (subject, predicate, object, source).
# ----------------------------------------------------------------------

REGISTRY_SRC = "meridian agent registry, seeded 2026-07-20"
POLICY_SRC = "wardryx policy store (gx_policy_seed.py), seeded 2026-07-18"

FACTS = [
    # team membership
    (HERO_AGENT, "belongs_to_team", "treasury", REGISTRY_SRC),
    (A("kyc-aml", "aml-case-copilot"), "belongs_to_team", "kyc-aml", REGISTRY_SRC),
    (A("lending", "underwriting-copilot"), "belongs_to_team", "lending", REGISTRY_SRC),
    (A("fraud", "fraud-triage-copilot"), "belongs_to_team", "fraud", REGISTRY_SRC),
    (A("support", "support-tier2-bot"), "belongs_to_team", "support", REGISTRY_SRC),
    (A("compliance", "model-risk-validator"), "belongs_to_team", "compliance", REGISTRY_SRC),
    # model assignment
    (HERO_AGENT, "uses_model", "gpt-4o", REGISTRY_SRC),
    (A("kyc-aml", "aml-case-copilot"), "uses_model", "gpt-4o", REGISTRY_SRC),
    (A("treasury", "cashflow-forecaster"), "uses_model", "gpt-4o", REGISTRY_SRC),
    (A("lending", "underwriting-copilot"), "uses_model", "claude-sonnet-5", REGISTRY_SRC),
    (A("fraud", "fraud-triage-copilot"), "uses_model", "claude-sonnet-5", REGISTRY_SRC),
    # ownership
    (HERO_AGENT, "owned_by", "v.koval", REGISTRY_SRC),
    (A("treasury", "cashflow-forecaster"), "owned_by", "v.koval", REGISTRY_SRC),
    (A("lending", "underwriting-copilot"), "owned_by", "s.tkachenko", REGISTRY_SRC),
    (A("kyc-aml", "aml-case-copilot"), "owned_by", "t.fedirko", REGISTRY_SRC),
    (A("support", "support-tier2-bot"), "owned_by", "o.marchenko", REGISTRY_SRC),
    # policy governance (matches gx_policy_seed.py's real wardryx policies)
    (HERO_AGENT, "governed_by_policy", "treasury-human-approval", POLICY_SRC),
    (A("lending", "underwriting-copilot"), "governed_by_policy", "underwriting-approval", POLICY_SRC),
    (A("kyc-aml", "aml-case-copilot"), "governed_by_policy", "aml-max-steps", POLICY_SRC),
    (A("kyc-aml", "kyc-intake-agent"), "governed_by_policy", "kyc-require-attestation", POLICY_SRC),
    (A("support", "support-tier2-bot"), "governed_by_policy", "support-spend-cap", POLICY_SRC),
]

# Index of the fact (0-based, into FACTS above) that becomes the semantic
# why() demonstration: the fact that directly explains the approval hold.
WHY_FACT_INDEX = 16  # (HERO_AGENT, "governed_by_policy", "treasury-human-approval", ...)

RECALL_QUERY = "why did the reconciliation batch keep retrying"


def raw_sqlite_counts() -> tuple:
    """Independent cross-check of episode/fact counts, read directly from the
    SQLite file with plain sqlite3 (no sqlite-vec extension needed for these
    two ordinary tables) rather than through Engram's own API - mirrors
    gx_quality.py's print_db_summary() doing the same for verdryx.db."""
    import sqlite3

    conn = sqlite3.connect(f"file:{DB_PATH}?mode=ro", uri=True)
    try:
        episodes = conn.execute("SELECT COUNT(*) FROM episodes").fetchone()[0]
        facts = conn.execute("SELECT COUNT(*) FROM facts").fetchone()[0]
    finally:
        conn.close()
    return episodes, facts


def main() -> None:
    t0 = time.time()
    pool = _EngramPool(DB_PATH, default_agent_id=None, events_path=None)
    try:
        before = _stats(pool, agent_id=None)
        print("=== before (real counts read back from the store) ===")
        print(
            f"episodic={before['counts']['episodic']} "
            f"semantic={before['counts']['semantic']} "
            f"entities={before['entities']} db_size={before['db_size_bytes']}B "
            f"db={before['db_path']}"
        )

        n_agents = len({agent_id for agent_id, *_ in EPISODES})
        print(f"\nWriting {len(EPISODES)} episodic memories across {n_agents} agents...")
        episode_ids = []
        for agent_id, content, actors, tags, salience in EPISODES:
            mem = pool.get(agent_id)  # one Engram instance per distinct agent_id
            mid = mem.observe(content, actors=actors, tags=tags, salience=salience)
            episode_ids.append(mid)
        print(f"  wrote {len(episode_ids)} episodes (first id: {episode_ids[0]})")

        print(f"\nWriting {len(FACTS)} semantic facts...")
        registry = pool.get(None)  # facts are unscoped; any pooled instance works
        fact_ids = []
        for subject, predicate, obj, source in FACTS:
            fid = registry.assert_fact(subject, predicate, obj, confidence=1.0, source=source)
            fact_ids.append(fid)
        print(f"  wrote {len(fact_ids)} facts (first id: {fact_ids[0]})")

        print(f"\n=== recall: {RECALL_QUERY!r} (mode=hybrid, limit=8, all agents) ===")
        hits = _recall(pool, RECALL_QUERY, limit=8, agent_id=None, mode="hybrid")
        if not hits:
            print("  (no hits)")
        for h in hits:
            actors = ",".join(h.get("actors", [])) or "-"
            print(f"  [{h['score']:.3f}] {h['id']}  actors=[{actors}]")
            print(f"      {h['content']}")

        why_episode_id = episode_ids[WHY_EPISODE_INDEX]
        print(f"\n=== why(): episodic memory {why_episode_id} ===")
        print(
            f"    (\"{EPISODES[WHY_EPISODE_INDEX][1][:70]}...\")"
        )
        prov_episodic = _why(pool, why_episode_id)
        print(json.dumps(prov_episodic, indent=2, default=str))

        why_fact_id = fact_ids[WHY_FACT_INDEX]
        print(f"\n=== why(): semantic fact {why_fact_id} ===")
        subj, pred, obj, _src = FACTS[WHY_FACT_INDEX]
        print(f"    (\"{subj} {pred} {obj}\")")
        prov_semantic = _why(pool, why_fact_id)
        print(json.dumps(prov_semantic, indent=2, default=str))

        after = _stats(pool, agent_id=None)
        print("\n=== after (real counts read back from the store) ===")
        print(
            f"episodic={after['counts']['episodic']} "
            f"semantic={after['counts']['semantic']} "
            f"facts_total={after['facts_total']} facts_active={after['facts_active']} "
            f"entities={after['entities']} vector_index_size={after['vector_index_size']} "
            f"db_size={after['db_size_bytes']}B"
        )
        print(
            f"delta: +{after['counts']['episodic'] - before['counts']['episodic']} episodic, "
            f"+{after['counts']['semantic'] - before['counts']['semantic']} semantic"
        )

        raw_episodes, raw_facts = raw_sqlite_counts()
        print(
            f"\nindependent cross-check via plain sqlite3 against {DB_PATH}: "
            f"episodes table={raw_episodes} rows, facts table={raw_facts} rows"
        )
    finally:
        pool.close()  # flushes buffered access-log entries on every pooled instance

    print(f"\nDone in {time.time() - t0:.1f}s")


if __name__ == "__main__":
    main()
