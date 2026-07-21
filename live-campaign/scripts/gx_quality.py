#!/usr/bin/env python3
# Genaryx live-validation quality-plane seeder - Verdryx (eval, drift, cost-per-correct).
#
# Why this script exists
# -----------------------
# RESULTS.md from the 2026-07-17/18 run says it plainly: "Verdryx (quality) +
# engram (memory) stores were NOT seeded this run (empty)." The console's
# Quality panel had nothing real to read, so the gx-quality.webp capture was
# thin. This script fixes that honestly: it authors a real eval set about
# THIS bank's agent tasks, runs it TWICE through verdryx's real CLI against a
# real Anthropic model (no --model stub), snapshots a baseline, checks drift,
# and runs cost-per-correct. Nothing here writes a row into verdryx.db
# directly - every row in eval_runs/scores/baselines is put there by
# verdryx's own Store.save_run()/set_baseline() (verdryx/store.py), called
# from inside the real `verdryx` CLI subprocess.
#
# Where Genaryx actually reads this from (crates/connectors/src/verdryx.rs,
# read 2026-07-20, not modified): verdryx has no --json/--format output on
# any subcommand (eval/baseline/drift/cost-per-correct all print human text
# only), so the console's Quality panel opens verdryx.db READ-ONLY and
# SELECTs eval_runs/scores/baselines directly. That means the only things
# that matter for the console are the three tables this script grows via the
# real `eval`/`baseline`/`drift` subcommands. `cost-per-correct` prints a
# report to stdout and saves nothing to any store (verdryx/cli.py
# _cmd_cost_per_correct has no Store.* call) - Genaryx does not read it. It
# is run anyway because the task asks for it and because it is a real,
# useful check of this campaign's unit economics; its input file is
# documented below as authored, not queried live (see COST_RECORDS).
#
# Grader mix, and why it is not "32 cases x 2 real model calls"
# ---------------------------------------------------------------
# verdryx's eval loop (verdryx/cli.py:run_eval) calls adapter.complete(case.prompt)
# for every case EXCEPT GraderKind.OUTCOME_TAG (where case.prompt already IS
# the recorded production outcome tag - there is nothing to send a model).
# GraderKind.LLM_JUDGE additionally calls adapter.judge(...) to grade that
# output, a SECOND real model call. So the real spend per run is:
#   exact/regex cases  -> 1 call each  (complete only)
#   llm_judge cases     -> 2 calls each (complete, then judge)
#   outcome_tag cases   -> 0 calls      (table lookup against a recorded tag)
# This eval set (see build_evalset()) is deliberately NOT all llm_judge: five
# of its cases grade this campaign's OWN recorded reconciliation-batch
# outcomes (escalated/abandoned/case_resolved) via OutcomeTagGrader, which is
# exactly what that grader is for (verdryx/README.md "Eval set format": "For
# outcome_tag cases, prompt holds the outcome tag itself, since there is
# nothing to send a model when grading an already-recorded production
# outcome"). That is real tool use with zero API cost, not a cost dodge.
#
# Cost control (verdryx/pricing.py PriceBook.default(): claude-haiku-4-5 is
# $1.00/Mtok in, $5.00/Mtok out): 32 cases/run, 15 exact + 6 regex + 6
# llm_judge (12 calls) + 5 outcome_tag (0 calls) = 33 real model calls/run,
# 66 across both runs. Every prompt in this eval set instructs the model to
# answer in one word/line or a capped sentence count specifically to bound
# output tokens (the model under evaluation is billed for its own output,
# same as any real usage). Back-of-envelope at those instructed lengths is
# roughly 3-5 cents combined for both runs; RUN_ABORT_USD/TOTAL_WARN_USD
# below are a hard safety net in case a real response runs long, checked by
# reading actual cost_usd back out of verdryx.db between runs (never
# estimated in place of a real number - see run_cost_usd()).
#
# The key (read once, never printed, never argv)
# -------------------------------------------------
# /Users/factory/.taipan/genaryx-live.anthropic.key (mode 600, one line) is
# read in this process and injected ONLY into the environment of the
# `verdryx` subprocess (ANTHROPIC_API_KEY=...), exactly the variable
# verdryx/config.py's Config.from_env() already looks for. It is never
# interpolated into an argv list (which `ps` can show) and never written to
# any file, log, or the eval set/cost-per-correct artifacts this script
# produces.
#
# Re-running this script
# ------------------------
# Safe, but NOT free and NOT idempotent. verdryx eval always mints a fresh
# uuid4() EvalRun id (verdryx/cli.py:run_eval) and Store.save_run() is an
# INSERT, never an update of a prior run - so a second run of this script
# adds two MORE real eval runs, one MORE baseline, and spends real API
# budget again. It will never corrupt or wipe verdryx.db (nothing here
# issues DELETE/DROP, and the two pre-existing eval runs / 10 scores / 1
# baseline from earlier work are left untouched), but running it twice does
# duplicate this campaign's story under fresh ids. Only re-run deliberately.
import json
import os
import re
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

VERDRYX_BIN = Path("/Users/factory/Development/verdryx/.venv/bin/verdryx")
DB_PATH = Path.home() / ".taipan" / "verdryx.db"
KEY_FILE = Path.home() / ".taipan" / "genaryx-live.anthropic.key"
SCRIPT_DIR = Path(__file__).resolve().parent
EVALSET_PATH = SCRIPT_DIR / "gx_quality_evalset.json"
COSTPER_PATH = SCRIPT_DIR / "gx_quality_costper.ndjson"

MODEL = "claude-haiku-4-5-20251001"
ORG = "meridian.example"
HERO_RUN = "reconciliation-batch-eod-002-LIVE"

# Safety net only (see header): checked against REAL cost_usd read back from
# verdryx.db after each run, never against an estimate.
RUN_ABORT_USD = 0.15
TOTAL_WARN_USD = 0.25


def _die(msg: str) -> None:
    print(f"error: {msg}", file=sys.stderr)
    sys.exit(1)


# ----------------------------------------------------------------------
# Eval set: meridian.example agent tasks, grounded in the same story as
# gx_fleet_v3.py / gx_policy_seed.py (same org, same HERO_RUN, same policy
# thresholds: treasury-human-approval requires human sign-off above $25).
# ----------------------------------------------------------------------

ONE_WORD = "Respond with exactly one word, no punctuation: "


def _exact(case_id, prompt, expected):
    return {"id": case_id, "prompt": prompt, "expected": expected, "grader": "exact"}


def _regex(case_id, prompt, pattern):
    return {"id": case_id, "prompt": prompt, "expected": pattern, "grader": "regex"}


def _judge(case_id, prompt, rubric):
    return {"id": case_id, "prompt": prompt, "rubric": rubric, "grader": "llm_judge"}


def _outcome(case_id, tag):
    return {"id": case_id, "prompt": tag, "grader": "outcome_tag"}


TXN_ANOMALY = [
    _exact(
        "txn-anom-01",
        "A meridian.example retail customer who has averaged 3 debit transactions "
        "per week for two years suddenly initiates 14 wire transfers in one hour, "
        "each just under the $10,000 reporting threshold, to 14 different "
        "first-time payees. Classify this pattern for the fraud team. "
        + ONE_WORD + "ANOMALOUS or NORMAL.",
        "ANOMALOUS",
    ),
    _exact(
        "txn-anom-02",
        "A meridian.example corporate payroll account sends its regular biweekly "
        "ACH batch of 412 employee salary payments, same payee list and similar "
        "amounts as the prior 25 pay cycles. Classify this pattern for the fraud "
        "team. " + ONE_WORD + "ANOMALOUS or NORMAL.",
        "NORMAL",
    ),
    _exact(
        "txn-anom-03",
        "A dormant meridian.example account with a $40 balance and no activity "
        "for 18 months receives a $250,000 incoming wire, then immediately "
        "initiates three outgoing transfers to newly opened accounts at other "
        "institutions. Classify this pattern for the fraud team. " + ONE_WORD
        + "ANOMALOUS or NORMAL.",
        "ANOMALOUS",
    ),
    _exact(
        "txn-anom-04",
        "A meridian.example small-business customer's card is used for its usual "
        "weekly inventory purchase at the same wholesaler it has used for three "
        "years, for an amount within 5 percent of its typical order. Classify "
        "this pattern for the fraud team. " + ONE_WORD + "ANOMALOUS or NORMAL.",
        "NORMAL",
    ),
    _exact(
        "txn-anom-05",
        "A meridian.example customer's card, last used in Kyiv six hours ago, is "
        "used again now for an in-person purchase in Lagos, a distance impossible "
        "to travel in that time. Classify this pattern for the fraud team. "
        + ONE_WORD + "ANOMALOUS or NORMAL.",
        "ANOMALOUS",
    ),
    _exact(
        "txn-anom-06",
        "A meridian.example customer withdraws their usual $200 from the same "
        "ATM they use every Friday on their way home from work. Classify this "
        "pattern for the fraud team. " + ONE_WORD + "ANOMALOUS or NORMAL.",
        "NORMAL",
    ),
]

SANCTIONS = [
    _exact(
        "sanx-01",
        "Screen the name 'Aleksandr V. Petrov, DOB 1974-03-11' against a "
        "sanctions list that contains 'Petrov, Aleksandr Viktorovich, DOB "
        "1974-03-11, designated 2022'. " + ONE_WORD + "MATCH or NO_MATCH.",
        "MATCH",
    ),
    _exact(
        "sanx-02",
        "Screen the name 'Maria Santos' against a sanctions list that contains "
        "only 'Santos, Mariana Elena, DOB 1990-06-02' and no other Santos "
        "entries. " + ONE_WORD + "MATCH or NO_MATCH.",
        "NO_MATCH",
    ),
    _exact(
        "sanx-03",
        "Screen the company 'Nordic Freight Solutions LLC' against a sanctions "
        "list that contains 'Nordic Freight Solutions LLC, also known as NFS "
        "Cargo, designated 2023 for sanctions evasion'. " + ONE_WORD
        + "MATCH or NO_MATCH.",
        "MATCH",
    ),
    _exact(
        "sanx-04",
        "Screen the name 'John Smith' against a sanctions list that has no "
        "exact or close match to any 'Smith' entry. " + ONE_WORD
        + "MATCH or NO_MATCH.",
        "NO_MATCH",
    ),
    _exact(
        "sanx-05",
        "Screen the name 'O. Ivanenko' against a sanctions list that contains "
        "'Ivanenko, Oleh Dmytrovych, DOB 1968-11-30, designated 2024 for asset "
        "freeze', where the transliteration and initials plausibly match. "
        + ONE_WORD + "MATCH or NO_MATCH.",
        "MATCH",
    ),
]

KYC_EXTRACTION = [
    _regex(
        "kyc-ext-01",
        "Extract the IBAN from this text and reply with only the IBAN: 'Please "
        "credit account IBAN UA213223130000026007233566001 for the onboarding "
        "deposit.'",
        "UA213223130000026007233566001",
    ),
    _regex(
        "kyc-ext-02",
        "Extract the passport number from this text and reply with only the "
        "passport number: 'Applicant presented passport number FF7291834, "
        "issued by Ukraine.'",
        "FF7291834",
    ),
    _regex(
        "kyc-ext-03",
        "Extract the date of birth in YYYY-MM-DD format from this text and "
        "reply with only the date: 'Full name Olena Kovalenko, born 14 May "
        "1989, resident of Lviv.'",
        r"1989-05-14",
    ),
    _regex(
        "kyc-ext-04",
        "Extract the tax identification number from this text and reply with "
        "only the number: 'Corporate applicant Meridian Logistics TOV, EDRPOU "
        "code 39481207, requesting a business account.'",
        "39481207",
    ),
    _regex(
        "kyc-ext-05",
        "Extract the country of citizenship as an ISO 3166-1 alpha-2 code from "
        "this text and reply with only the code: 'Applicant holds a Ukrainian "
        "passport and resides in Warsaw, Poland.'",
        r"\bUA\b",
    ),
    _regex(
        "kyc-ext-06",
        "Extract the SWIFT/BIC code from this text and reply with only the "
        "code: 'Wire instructions: beneficiary bank SWIFT MERIUA2XXXX, account "
        "held at Meridian Bank Kyiv branch.'",
        "MERIUA2XXXX",
    ),
]

UNDERWRITING_RATIONALE = [
    _judge(
        "uw-rationale-01",
        "Write a one-paragraph credit rationale (max 3 sentences) for "
        "underwriting application UW-0231: applicant DTI 0.31, FICO 730, "
        "requested $180,000 mortgage, 20% down payment, stable 6-year "
        "employment history. State a clear approve or decline recommendation.",
        "Recommends approval (DTI 0.31 is well within policy, FICO 730 is "
        "good, stable employment, 20% down are all strong signals), and cites "
        "at least two of DTI/FICO/down-payment/employment by name. Score 1.0 "
        "if both hold, 0.5 if only one holds, 0.0 if neither or it recommends "
        "decline.",
    ),
    _judge(
        "uw-rationale-02",
        "Write a one-paragraph credit rationale (max 3 sentences) for "
        "underwriting application UW-0238: applicant DTI 0.52, FICO 615, "
        "requested $95,000 personal loan, no down payment, employment history "
        "under 6 months. State a clear approve or decline recommendation.",
        "Recommends decline or at minimum flags material risk (DTI 0.52 is "
        "well above typical policy limits, FICO 615 is subprime, employment "
        "history is short), and cites at least two of those risk factors by "
        "name. Score 1.0 if both hold, 0.5 if only one holds, 0.0 if it "
        "recommends approval without qualification.",
    ),
    _judge(
        "uw-rationale-03",
        "Write a one-paragraph explanation (max 3 sentences) of why "
        "underwriting-copilot's decision on application UW-0231 required human "
        "sign-off even though the model recommended approval, given "
        "meridian.example's underwriting-approval policy requires human review "
        "above $10.00 in estimated model cost and this evaluation cost $12.40.",
        "Correctly explains the hold is because the estimated cost ($12.40) "
        "exceeded the policy threshold ($10.00), not because of the "
        "applicant's creditworthiness. Score 1.0 if it clearly attributes the "
        "hold to the cost threshold, 0.5 if vague, 0.0 if it invents an "
        "unrelated reason.",
    ),
    _judge(
        "uw-rationale-04",
        "In two to three sentences, explain to a loan officer why "
        "underwriting-copilot's rationale should always cite DTI, FICO, and "
        "down payment explicitly rather than just giving a yes/no answer.",
        "Explains that citing the specific metrics makes the recommendation "
        "auditable/reviewable by a human, and/or lets a reviewer independently "
        "verify the reasoning rather than trusting a bare verdict. Score 1.0 "
        "if that reasoning is present, 0.5 if generic but not wrong, 0.0 if it "
        "does not address auditability or reviewability at all.",
    ),
    _judge(
        "uw-rationale-05",
        "Write a one-paragraph credit rationale (max 3 sentences) for "
        "underwriting application UW-0245: applicant DTI 0.29, FICO 780, "
        "requested $310,000 mortgage, 35% down payment, 12-year employment at "
        "the same employer. State a clear approve or decline recommendation.",
        "Recommends approval clearly, and cites at least two of the four "
        "strong metrics (DTI, FICO, down payment, employment tenure). Score "
        "1.0 if both hold, 0.5 if only one holds, 0.0 if neither or it "
        "recommends decline.",
    ),
    _judge(
        "uw-rationale-06",
        "In two to three sentences, explain what could go wrong if "
        "underwriting-copilot's rationale generator quietly regressed and "
        "started approving applications with DTI above 0.50 without flagging "
        "them, from a bank's perspective.",
        "Identifies increased default risk/credit losses and/or a compliance "
        "or model-risk failure as the consequence. Score 1.0 if either risk is "
        "clearly named, 0.5 if it gestures at risk vaguely, 0.0 if it does not "
        "identify any real consequence.",
    ),
]

RECON_CLASSIFICATION = [
    _exact(
        "recon-cls-01",
        "Classify this ledger discrepancy: the sub-ledger shows a payment "
        "posted on 2026-06-30 but the general ledger shows the same payment "
        "posted on 2026-07-01, same amount, same counterparty. Reply with "
        "exactly one label, no punctuation: TIMING_DIFFERENCE, "
        "DUPLICATE_POSTING, FX_ROUNDING, or DATA_ENTRY_ERROR.",
        "TIMING_DIFFERENCE",
    ),
    _exact(
        "recon-cls-02",
        "Classify this ledger discrepancy: the same $18,420.00 vendor payment "
        "appears twice in the general ledger under two different transaction "
        "ids, one hour apart, otherwise identical. Reply with exactly one "
        "label, no punctuation: TIMING_DIFFERENCE, DUPLICATE_POSTING, "
        "FX_ROUNDING, or DATA_ENTRY_ERROR.",
        "DUPLICATE_POSTING",
    ),
    _exact(
        "recon-cls-03",
        "Classify this ledger discrepancy: a EUR 50,000.00 transfer converted "
        "to USD shows a 0.03 USD difference between the sub-ledger's and the "
        "general ledger's exchange-rate calculation, both using the same "
        "day's rate. Reply with exactly one label, no punctuation: "
        "TIMING_DIFFERENCE, DUPLICATE_POSTING, FX_ROUNDING, or "
        "DATA_ENTRY_ERROR.",
        "FX_ROUNDING",
    ),
    _exact(
        "recon-cls-04",
        "Classify this ledger discrepancy: a $4,250.00 payment was keyed into "
        "the sub-ledger as $2,450.00, transposed digits, no other explanation "
        "fits. Reply with exactly one label, no punctuation: "
        "TIMING_DIFFERENCE, DUPLICATE_POSTING, FX_ROUNDING, or "
        "DATA_ENTRY_ERROR.",
        "DATA_ENTRY_ERROR",
    ),
]

# Recorded production outcomes of the reconciliation-batch incident itself
# (the same run/shard ids as gx_fleet_v3.py's HERO_RUN and gx_policy_seed.py's
# pending approval). No model call: OutcomeTagGrader grades the tag verdryx's
# README describes as "an already-recorded production outcome" directly.
RECON_OUTCOMES = [
    _outcome(f"{HERO_RUN}-outcome", "escalated"),  # held for human approval ($48 > $25)
    _outcome("reconciliation-batch-eod-002-s007-outcome", "abandoned"),  # over cap, killed
    _outcome("reconciliation-batch-eod-002-s063-outcome", "abandoned"),
    _outcome("reconciliation-batch-eod-002-s128-outcome", "abandoned"),
    _outcome("reconciliation-batch-eod-001-clean-outcome", "case_resolved"),  # prior month, clean
]


def build_evalset() -> dict:
    cases = (
        TXN_ANOMALY
        + SANCTIONS
        + KYC_EXTRACTION
        + UNDERWRITING_RATIONALE
        + RECON_CLASSIFICATION
        + RECON_OUTCOMES
    )
    return {"id": "meridian-agent-quality-v1", "cases": cases}


# ----------------------------------------------------------------------
# Cost-per-correct input. verdryx cost-per-correct reads {outcome, cost_usd}
# records (verdryx/costper.py load_records); this is a hand-rolled NDJSON
# export in that exact shape, the second of the two input forms verdryx's
# own README documents ("a hand-rolled export from agent-event / trace
# outcome tags"). No TokenFuse cloud instance is running in this session to
# export a live Parquet trace from (checked: nothing listening on 8080-8091),
# so these per-run cost_usd figures are authored to sit inside the real cost
# ranges gx_fleet_v3.py's FLEET table and HERO_RUN/shard generation use, and
# the two run ids that overlap with gx_policy_seed.py's pending approvals
# (HERO_RUN and underwriting-copilot-0231) use that script's own numbers
# ($48.00 est vs 12.40 for the two, respectively) rather than inventing new
# ones. This is disclosed here and again in the printed report: it is
# authored input to a real computation, not a fabricated result.
# ----------------------------------------------------------------------

COST_RECORDS = [
    {"run_id": HERO_RUN, "outcome": "escalated", "cost_usd": 7.02},
    {"run_id": "reconciliation-batch-eod-002-s007", "outcome": "abandoned", "cost_usd": 0.58},
    {"run_id": "reconciliation-batch-eod-002-s063", "outcome": "abandoned", "cost_usd": 0.57},
    {"run_id": "reconciliation-batch-eod-002-s128", "outcome": "abandoned", "cost_usd": 0.56},
    {"run_id": "reconciliation-batch-eod-002-s011", "outcome": "abandoned", "cost_usd": 0.59},
    {"run_id": "reconciliation-batch-eod-002-s084", "outcome": "abandoned", "cost_usd": 0.55},
    {"run_id": "reconciliation-batch-eod-002-s142", "outcome": "abandoned", "cost_usd": 0.60},
    {"run_id": "reconciliation-batch-eod-001-clean", "outcome": "case_resolved", "cost_usd": 0.61},
    {"run_id": "cashflow-forecaster-0180", "outcome": "case_resolved", "cost_usd": 4.05},
    {"run_id": "underwriting-copilot-0231", "outcome": "escalated", "cost_usd": 12.40},
    {"run_id": "underwriting-copilot-0090", "outcome": "case_resolved", "cost_usd": 1.35},
    {"run_id": "aml-case-copilot-0210", "outcome": "case_resolved", "cost_usd": 4.62},
    {"run_id": "support-tier2-bot-0310", "outcome": "case_resolved", "cost_usd": 0.94},
    {"run_id": "support-tier2-bot-0450", "outcome": "escalated", "cost_usd": 1.10},
]


# ----------------------------------------------------------------------
# verdryx CLI plumbing
# ----------------------------------------------------------------------


def load_api_key() -> str:
    if not KEY_FILE.exists():
        _die(f"key file not found: {KEY_FILE}")
    mode = KEY_FILE.stat().st_mode & 0o777
    if mode != 0o600:
        print(f"warning: {KEY_FILE} is mode {oct(mode)}, expected 0600", file=sys.stderr)
    key = KEY_FILE.read_text().strip()
    if len(key) < 20:
        _die("key file content is too short to be a real Anthropic API key; refusing to use it")
    print(
        f"Anthropic API key: loaded from {KEY_FILE} "
        f"({KEY_FILE.stat().st_size} bytes on disk, value not printed)"
    )
    return key


def run_verdryx(args: list, env: dict) -> subprocess.CompletedProcess:
    if not VERDRYX_BIN.exists():
        _die(f"verdryx CLI not found at {VERDRYX_BIN}")
    return subprocess.run(
        [str(VERDRYX_BIN), *args], env=env, capture_output=True, text=True, check=False
    )


def run_eval_once(env: dict, label: str) -> str:
    proc = run_verdryx(
        ["eval", str(EVALSET_PATH), "--model", MODEL, "--db", str(DB_PATH)], env
    )
    print(proc.stdout)
    if proc.stderr.strip():
        print(proc.stderr, file=sys.stderr)
    if proc.returncode != 0:
        _die(f"{label}: verdryx eval exited {proc.returncode}")
    m = re.search(r"Eval run ([0-9a-fA-F-]{36})", proc.stdout)
    if not m:
        _die(f"{label}: could not find an eval run id in verdryx's output")
    return m.group(1)


def run_cost_usd(run_id: str) -> tuple:
    """Real cost_usd/tokens/case-count for `run_id`, read back from verdryx.db
    itself (never estimated) so the budget safety check below is checking a
    fact, not a guess."""
    conn = sqlite3.connect(f"file:{DB_PATH}?mode=ro", uri=True)
    try:
        row = conn.execute(
            "SELECT COALESCE(SUM(cost_usd),0), COALESCE(SUM(tokens),0), COUNT(*) "
            "FROM scores WHERE run_id = ?",
            (run_id,),
        ).fetchone()
    finally:
        conn.close()
    return row


def print_db_summary() -> None:
    conn = sqlite3.connect(f"file:{DB_PATH}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    try:
        runs = conn.execute(
            "SELECT id, model, started_at, finished_at FROM eval_runs ORDER BY started_at"
        ).fetchall()
        n_scores = conn.execute("SELECT COUNT(*) FROM scores").fetchone()[0]
        print(f"\n=== verdryx.db summary (read back from SQLite, {DB_PATH}) ===")
        print(f"eval_runs: {len(runs)}")
        print(f"scores:    {n_scores}")
        print(f"{'run_id':<38} {'model':<28} {'cases':>5} {'mean':>6} {'cost_usd':>9}")
        for r in runs:
            agg = conn.execute(
                "SELECT COUNT(*), AVG(value), COALESCE(SUM(cost_usd),0) "
                "FROM scores WHERE run_id = ?",
                (r["id"],),
            ).fetchone()
            n, mean, cost = agg
            mean_s = f"{mean:.3f}" if mean is not None else "  n/a"
            print(f"{r['id']:<38} {r['model']:<28} {n:>5} {mean_s:>6} {cost:>9.4f}")
        baselines = conn.execute(
            "SELECT id, eval_run_id, mean_score, created_at, label FROM baselines "
            "ORDER BY created_at"
        ).fetchall()
        print(f"\nbaselines: {len(baselines)}")
        for b in baselines:
            label = b["label"] or "(no label)"
            print(
                f"  {b['id']}  run={b['eval_run_id']}  "
                f"mean_score={b['mean_score']:.3f}  label={label!r}"
            )
    finally:
        conn.close()


def main() -> None:
    t0 = time.time()
    key = load_api_key()
    child_env = dict(os.environ)
    child_env["ANTHROPIC_API_KEY"] = key
    key = None  # drop the local reference now that it is only in child_env

    evalset = build_evalset()
    EVALSET_PATH.write_text(json.dumps(evalset, indent=2) + "\n")
    n_exact = sum(1 for c in evalset["cases"] if c["grader"] == "exact")
    n_regex = sum(1 for c in evalset["cases"] if c["grader"] == "regex")
    n_judge = sum(1 for c in evalset["cases"] if c["grader"] == "llm_judge")
    n_outcome = sum(1 for c in evalset["cases"] if c["grader"] == "outcome_tag")
    calls_per_run = n_exact + n_regex + 2 * n_judge
    print(f"Eval set written: {EVALSET_PATH}")
    print(
        f"  {len(evalset['cases'])} cases: {n_exact} exact, {n_regex} regex, "
        f"{n_judge} llm_judge, {n_outcome} outcome_tag"
    )
    print(
        f"  -> {calls_per_run} real Anthropic calls per run, "
        f"{calls_per_run * 2} across both runs, model={MODEL}"
    )
    print(
        "  estimated combined cost at claude-haiku-4-5 rates "
        "($1.00/Mtok in, $5.00/Mtok out), given the bounded-output prompts "
        "used here: roughly $0.03-0.10 total (target: under $0.20; safety "
        f"abort if a single run alone exceeds ${RUN_ABORT_USD:.2f})"
    )

    print("\n=== eval run 1/2 (this run becomes the baseline) ===")
    run1 = run_eval_once(child_env, "run 1")
    cost1, tok1, n1 = run_cost_usd(run1)
    print(f"run 1 = {run1}: {n1} scored cases, {tok1} tokens, ${cost1:.4f} real spend")
    if cost1 > RUN_ABORT_USD:
        _die(
            f"run 1 alone cost ${cost1:.4f}, over the ${RUN_ABORT_USD:.2f} safety "
            "ceiling; stopping before spending on run 2. verdryx.db already has "
            "run 1's real result (not rolled back)."
        )

    print("\n=== baseline (snapshot of run 1) ===")
    proc = run_verdryx(
        ["baseline", run1, "--db", str(DB_PATH), "--label", "meridian-quality-2026-07-20-run1"],
        child_env,
    )
    print(proc.stdout)
    if proc.returncode != 0:
        print(proc.stderr, file=sys.stderr)
        _die(f"verdryx baseline exited {proc.returncode}")
    m = re.search(r"Baseline ([0-9a-fA-F-]{36})", proc.stdout)
    if not m:
        _die("could not find a baseline id in verdryx's output")
    baseline_id = m.group(1)

    print("=== eval run 2/2 (compared against the baseline via drift) ===")
    run2 = run_eval_once(child_env, "run 2")
    cost2, tok2, n2 = run_cost_usd(run2)
    print(f"run 2 = {run2}: {n2} scored cases, {tok2} tokens, ${cost2:.4f} real spend")

    total_cost = cost1 + cost2
    total_tok = tok1 + tok2
    print(
        f"\nTOTAL real Anthropic spend, both eval runs: ${total_cost:.4f} "
        f"({total_tok} tokens, {n1 + n2} scored cases, model={MODEL})"
    )
    if total_cost > TOTAL_WARN_USD:
        print(
            f"WARNING: combined cost ${total_cost:.4f} exceeded the "
            f"${TOTAL_WARN_USD:.2f} target ceiling for this seeding job.",
            file=sys.stderr,
        )

    print("\n=== drift: run 2 vs the baseline ===")
    proc = run_verdryx(
        ["drift", "--baseline", baseline_id, "--db", str(DB_PATH), "--window", "1"], child_env
    )
    print(proc.stdout)
    if proc.returncode != 0:
        print(proc.stderr, file=sys.stderr)
        _die(f"verdryx drift exited {proc.returncode}")

    print("=== cost-per-correct ===")
    lines = "\n".join(json.dumps(r) for r in COST_RECORDS) + "\n"
    COSTPER_PATH.write_text(lines)
    print(f"(input: {COSTPER_PATH}, {len(COST_RECORDS)} authored records -- see header docstring)")
    proc = run_verdryx(["cost-per-correct", "--input", str(COSTPER_PATH)], child_env)
    print(proc.stdout)
    if proc.returncode != 0:
        print(proc.stderr, file=sys.stderr)
        _die(f"verdryx cost-per-correct exited {proc.returncode}")

    print_db_summary()
    print(f"\nDone in {time.time() - t0:.1f}s")


if __name__ == "__main__":
    main()
