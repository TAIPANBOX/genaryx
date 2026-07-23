//! `TokenfuseClient`: a thin `ToolRunner` over the TokenFuse gateway CLI, for
//! the one thing the Evidence Center needs from it (docs/PHASE4.md W3): the
//! FOCUS cost export. Grounded in `~/Development/tokenfuse/crates/gateway/src/
//! focusexport.rs` (read 2026-07-17).
//!
//! `tokenfuse focus-export --traces <dir-or-glob> --out <file.csv> [--from
//! <rfc3339>] [--to <rfc3339>]` reads the already-written Parquet call trace and
//! writes a FOCUS 1.2-style CSV (FinOps Open Cost & Usage Specification), one
//! row per LLM call, so a bank/FinOps team can load agent spend into the same
//! tooling they use for cloud spend. It is read-only and never touches the
//! enforcement hot path. Because it only writes to a file (`--out`), this client
//! runs it against a private temp path, reads the CSV back, and removes the temp
//! file - so the caller gets the bytes directly, like every other connector.
//!
//! Fail-closed (06 §0.5): a spawn failure is [`TokenfuseError::Spawn`] (the live
//! test reads it as "tokenfuse absent, skip"); a nonzero exit is
//! [`TokenfuseError::Cli`] with stderr; an unreadable output file is
//! [`TokenfuseError::Read`]. No panics. The caller resolves the binary path and
//! the traces dir (env discovery is the shell's job, like every other
//! connector).
//!
//! I10 ("Felyx optimization recommendations") adds two more read methods
//! against the SAME `TOKENFUSE_DATA_DIR`-scoped CLI, so Felyx can reason about
//! cost/savings without a new plane:
//!
//! - [`TokenfuseClient::savings`] runs `tokenfuse savings` (reads
//!   `TOKENFUSE_DATA_DIR` from the environment, no CLI flag - confirmed
//!   against `~/Development/tokenfuse/crates/gateway/src/main.rs`'s `savings`
//!   arm) and parses its human-readable report
//!   (`crates/gateway/src/savingscli.rs::run`) into `TokenfuseSavings`.
//! - [`TokenfuseClient::cost_per_action`] runs `tokenfuse sql "<query>"` twice
//!   (once grouped by `model`, once by `agent_id`) and parses the result into
//!   `CostPerActionReport`.
//!
//! Both are TEXT-SCRAPES of a CLI meant for a human terminal, not a machine
//! caller, so the fragility is worth stating plainly: `tokenfuse sql` has no
//! `--format json` (or any other machine-readable mode) as of this reading -
//! `main.rs`'s `sql` arm just joins every trailing arg into one query string,
//! and `sqlq.rs::run` always prints DataFusion/Arrow's `pretty_format_batches`
//! box-drawing table. Because of that, `cost_per_action` deliberately never
//! accepts operator- or model-supplied SQL text (that would make an
//! already-fragile parse also attacker/typo-controlled); the two queries are
//! FIXED constants baked into this file (`PER_MODEL_COST_QUERY`/
//! `PER_AGENT_COST_QUERY`), so the only thing that can change the shape this
//! client must parse is a tokenfuse upgrade, not a caller. `tokenfuse sql`
//! also has its own quirk, found empirically (2026-07-23) rather than
//! documented anywhere: a missing/unreadable trace directory prints
//! `sql error: ...` to STDERR but still EXITS 0 (`main.rs`'s `sql` arm never
//! sets a nonzero exit code), so `cost_per_action` treats "empty stdout plus a
//! `sql error:` stderr" as a failure regardless of the exit code; a
//! genuinely-empty-but-readable trace instead prints an empty
//! `pretty_format_batches` table (`"++\n++\n"`, no header) on STDOUT, which
//! this client reads as legitimately zero rows, not an error.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};

// ---- error -----------------------------------------------------------------

/// Every failure mode a [`TokenfuseClient`] call can surface. Fail-closed.
#[derive(Debug, thiserror::Error)]
pub enum TokenfuseError {
    /// The tokenfuse binary could not be spawned (missing, not executable).
    #[error("tokenfuse spawn {bin}: {source}")]
    Spawn {
        bin: String,
        #[source]
        source: std::io::Error,
    },

    /// A subcommand exited nonzero (bad flags, an unreadable traces dir), or
    /// (`sql` only, see this module's doc comment) exited `0` but printed a
    /// `sql error: ...` line to stderr with nothing on stdout.
    #[error("tokenfuse {command} exited {code}: {stderr}")]
    Cli {
        command: &'static str,
        code: i32,
        stderr: String,
    },

    /// The CSV `focus-export` should have written could not be read back.
    #[error("read focus-export output {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// `savings`/`cost_per_action` could not make sense of the CLI's stdout -
    /// see this module's doc comment on why that scrape is inherently
    /// fragile. Never silently guessed at; always surfaced.
    #[error("could not parse tokenfuse {command} output: {detail}")]
    Parse {
        command: &'static str,
        detail: String,
    },
}

// ---- read DTOs (I10) --------------------------------------------------------

/// [`TokenfuseClient::savings`]'s parsed result - mirrors the SIBLING
/// tokenfuse repo's `tokenfuse_core::savings::SavingsReport` field for field
/// (that type is not a Rust dependency of this crate; the shape is
/// duplicated here as data, learned from `savingscli.rs::run`'s actual
/// `println!`s, not imported). Deliberately named `TokenfuseSavings` rather
/// than `SavingsSummary`: that name is already used by the Cloud connector's
/// own `SavingsSummary` (`cloud_rest.rs`), a DIFFERENT source (Cloud's own
/// `/v1/savings` ledger) reporting a similar-shaped number - see
/// `crates/copilot/src/tools/optimize.rs`'s module doc for why both exist
/// rather than one replacing the other.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TokenfuseSavings {
    /// Whether the trace directory had any rows at all. `false` means
    /// `tokenfuse savings` printed its "no trace yet" message (a
    /// missing/unreadable directory, or a genuinely empty trace) - every
    /// other field is then a meaningless `0`/empty, not a real "nothing was
    /// ever blocked" answer, so check this FIRST.
    pub trace_data_found: bool,
    /// Sum of avoided spend across every budget-protection block (a run
    /// stopped before it could spend more).
    pub blocked_spend_microusd: i64,
    /// Total count of budget-protection block rows.
    pub blocked_calls: u64,
    /// Count of DISTINCT runs blocked at least once by budget protection.
    pub budget_breaks_prevented: u64,
    /// Dollars the semantic cache served for free (avoided spend).
    pub cache_saved_microusd: i64,
    /// Dollars the model router avoided by routing to a cheaper model.
    pub router_saved_microusd: i64,
    /// Blocked spend broken down by decision reason (e.g. `budget_exceeded`,
    /// `loop_detected`) - present only when at least one call was blocked.
    pub by_reason_microusd: BTreeMap<String, i64>,
}

/// One row of [`TokenfuseClient::cost_per_action`]'s per-model or per-agent
/// breakdown (`CostPerActionReport::by_model` / `by_agent`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostBreakdownRow {
    /// The model name (in `by_model`) or the agent id (in `by_agent`). `""`
    /// means the underlying trace rows never recorded that field (an older
    /// trace, or a call made with no agent context).
    pub label: String,
    pub calls: u64,
    pub total_cost_microusd: i64,
    /// Sum of `tool_calls` across this group, treating a NULL cell as `0` -
    /// check `tool_calls_known_rows` before reading this as "zero tool calls
    /// happened" rather than "we do not know".
    pub total_tool_calls: u64,
    /// How many rows in this group had a NON-NULL `tool_calls` value. `0`
    /// means every row in this group predates the trace's `tool_calls`
    /// column (I1; it is nullable by design), so `total_tool_calls` is then
    /// "unknown" rather than a real zero - exactly why
    /// `cost_per_tool_call_microusd` is `None` in that case.
    pub tool_calls_known_rows: u64,
    /// `total_cost_microusd / total_tool_calls`, integer division - a coarse
    /// average over the whole window, not a genuine per-call figure. `None`
    /// when `tool_calls_known_rows` is `0` (no data, see above) or
    /// `total_tool_calls` is `0` (known, and genuinely zero, so the ratio is
    /// undefined rather than infinite).
    pub cost_per_tool_call_microusd: Option<i64>,
}

/// [`TokenfuseClient::cost_per_action`]'s result: the same cost/call/tool-call
/// figures, sliced two ways.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CostPerActionReport {
    pub by_model: Vec<CostBreakdownRow>,
    pub by_agent: Vec<CostBreakdownRow>,
}

/// Cost + call-count + tool-call aggregates grouped by `model`. FIXED and
/// INTERNAL - never built from operator/model-supplied text (this module's
/// doc comment explains why). Validated against a live `tokenfuse-gateway
/// sql` invocation over a real trace, 2026-07-23. `tool_calls_known_rows`
/// lets a caller tell "zero tool calls" from "this trace predates the
/// column"; every currently-available local trace on the box this was
/// grounded against predates the column, so that branch is exercised by a
/// hand-built fixture in this module's tests, not a live capture - see the
/// test doc comments.
const PER_MODEL_COST_QUERY: &str = "select coalesce(model,'') as model, count(*) as calls, cast(sum(cost_microusd) as bigint) as total_cost_microusd, cast(sum(coalesce(tool_calls,0)) as bigint) as total_tool_calls, cast(count(tool_calls) as bigint) as tool_calls_known_rows from calls group by model order by model";

/// The same aggregate as [`PER_MODEL_COST_QUERY`], grouped by `agent_id`
/// instead of `model`.
const PER_AGENT_COST_QUERY: &str = "select coalesce(agent_id,'') as agent_id, count(*) as calls, cast(sum(cost_microusd) as bigint) as total_cost_microusd, cast(sum(coalesce(tool_calls,0)) as bigint) as total_tool_calls, cast(count(tool_calls) as bigint) as tool_calls_known_rows from calls group by agent_id order by agent_id";

// ---- client ----------------------------------------------------------------

/// A `ToolRunner` over the tokenfuse gateway CLI. Holds the resolved binary
/// path; `focus_export` is one synchronous invocation.
#[derive(Debug, Clone)]
pub struct TokenfuseClient {
    bin: PathBuf,
}

impl TokenfuseClient {
    /// Construct a client for a resolved `tokenfuse` binary path.
    pub fn new(bin: impl Into<PathBuf>) -> Self {
        Self { bin: bin.into() }
    }

    /// `tokenfuse focus-export --traces <traces_dir> --out <tmp.csv> [--from
    /// <rfc3339>] [--to <rfc3339>]` -> the FOCUS 1.2 CSV bytes. Writes to a
    /// private temp file, reads it back, and removes it, so the caller receives
    /// the bytes directly. `from`/`to` optionally window the export by call
    /// timestamp.
    pub fn focus_export(
        &self,
        traces_dir: &Path,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Vec<u8>, TokenfuseError> {
        // A unique temp output path (process id + an atomic counter; no
        // wall-clock, no extra deps), removed however this returns.
        static N: AtomicU32 = AtomicU32::new(0);
        let out_path = std::env::temp_dir().join(format!(
            "genaryx-focus-{}-{}.csv",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&out_path);

        let traces = traces_dir.to_string_lossy();
        let out_s = out_path.to_string_lossy();
        let mut args: Vec<&str> = vec![
            "focus-export",
            "--traces",
            traces.as_ref(),
            "--out",
            out_s.as_ref(),
        ];
        if let Some(f) = from {
            args.push("--from");
            args.push(f);
        }
        if let Some(t) = to {
            args.push("--to");
            args.push(t);
        }

        let output = std::process::Command::new(&self.bin)
            .args(&args)
            .output()
            .map_err(|source| TokenfuseError::Spawn {
                bin: self.bin.display().to_string(),
                source,
            });
        let output = match output {
            Ok(o) => o,
            Err(e) => {
                let _ = std::fs::remove_file(&out_path);
                return Err(e);
            }
        };
        if !output.status.success() {
            let _ = std::fs::remove_file(&out_path);
            return Err(TokenfuseError::Cli {
                command: "focus-export",
                code: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }

        let bytes = std::fs::read(&out_path).map_err(|source| TokenfuseError::Read {
            path: out_path.display().to_string(),
            source,
        });
        let _ = std::fs::remove_file(&out_path);
        bytes
    }

    /// `tokenfuse savings` (env `TOKENFUSE_DATA_DIR=<traces_dir>`, no CLI
    /// flag) -> the parsed FinOps savings summary. See [`TokenfuseSavings`]
    /// and this module's doc comment for the parse's shape and fragility.
    pub fn savings(&self, traces_dir: &Path) -> Result<TokenfuseSavings, TokenfuseError> {
        let output = self.spawn(&["savings"], traces_dir)?;
        if !output.status.success() {
            return Err(TokenfuseError::Cli {
                command: "savings",
                code: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_savings_text(&stdout)
    }

    /// Cost, call count, and tool-call totals grouped by `model` and (in a
    /// second invocation) by `agent_id`, via the two FIXED `tokenfuse sql`
    /// queries this module defines - never operator/model-supplied SQL, see
    /// this module's doc comment.
    pub fn cost_per_action(
        &self,
        traces_dir: &Path,
    ) -> Result<CostPerActionReport, TokenfuseError> {
        let by_model = self.run_cost_query(PER_MODEL_COST_QUERY, "model", traces_dir)?;
        let by_agent = self.run_cost_query(PER_AGENT_COST_QUERY, "agent_id", traces_dir)?;
        Ok(CostPerActionReport { by_model, by_agent })
    }

    /// Run one fixed aggregate `tokenfuse sql` query and parse its
    /// `pretty_format_batches` table, keying the label column by
    /// `label_column`'s exact header text (`"model"` or `"agent_id"`).
    fn run_cost_query(
        &self,
        query: &str,
        label_column: &str,
        traces_dir: &Path,
    ) -> Result<Vec<CostBreakdownRow>, TokenfuseError> {
        let output = self.spawn(&["sql", query], traces_dir)?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        // `tokenfuse sql` exits 0 even when the trace directory does not
        // exist or is unreadable - `main.rs`'s `sql` arm only `eprintln!`s
        // the error, it never sets a nonzero exit code (confirmed
        // empirically 2026-07-23 against a live binary). Empty stdout plus a
        // `sql error:` stderr is upstream's ONLY signal that something went
        // wrong, so treat it as a CLI failure despite the "0" exit code,
        // rather than silently returning "zero rows" the way a genuinely
        // empty (but readable) trace does - see `parse_cost_rows`'s doc
        // comment for that degenerate-but-legitimate shape.
        if stdout.trim().is_empty() && stderr.trim_start().starts_with("sql error:") {
            return Err(TokenfuseError::Cli {
                command: "sql",
                code: output.status.code().unwrap_or(0),
                stderr: stderr.trim().to_string(),
            });
        }
        if !output.status.success() {
            return Err(TokenfuseError::Cli {
                command: "sql",
                code: output.status.code().unwrap_or(-1),
                stderr: stderr.trim().to_string(),
            });
        }
        parse_cost_rows(&stdout, label_column)
    }

    /// Shell `tokenfuse <args>` with `TOKENFUSE_DATA_DIR` set to `traces_dir`
    /// (the env-var contract `savings`/`sql` both read, confirmed against
    /// `main.rs` - see this module's doc comment). Only [`TokenfuseError::Spawn`]
    /// can come from this step; the caller inspects the returned `Output`
    /// for a nonzero exit or the `sql`-specific exit-0-but-stderr quirk.
    fn spawn(
        &self,
        args: &[&str],
        traces_dir: &Path,
    ) -> Result<std::process::Output, TokenfuseError> {
        std::process::Command::new(&self.bin)
            .args(args)
            .env("TOKENFUSE_DATA_DIR", traces_dir)
            .output()
            .map_err(|source| TokenfuseError::Spawn {
                bin: self.bin.display().to_string(),
                source,
            })
    }
}

// ---- text-scrape parsing (I10) ----------------------------------------------

/// Parse a `${:.6}`-formatted amount (`tokenfuse_core::money::Microusd`'s
/// `Display` impl, confirmed against `crates/core/src/money.rs`) back into
/// microdollars by exact decimal arithmetic - never a float multiply-then-
/// round, which could drift for large amounts. Returns `None` for anything
/// that does not look like `[-]$<digits>[.<digits>]`.
fn parse_usd_to_microusd(s: &str) -> Option<i64> {
    let s = s.trim();
    let (sign, s) = match s.strip_prefix('-') {
        Some(rest) => (-1i64, rest),
        None => (1i64, s),
    };
    let s = s.strip_prefix('$')?;
    let (int_part, frac_part) = s.split_once('.').unwrap_or((s, ""));
    if int_part.is_empty() || !int_part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !frac_part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let int_val: i64 = int_part.parse().ok()?;
    let mut frac = frac_part.to_string();
    frac.truncate(6);
    while frac.len() < 6 {
        frac.push('0');
    }
    let frac_val: i64 = frac.parse().ok()?;
    Some(sign * (int_val * 1_000_000 + frac_val))
}

/// Parse `savingscli.rs::run`'s printed report (see this module's doc
/// comment) into a [`TokenfuseSavings`]. Every line shape this function
/// expects is a literal `println!` in that file, ground-truthed against a
/// live `tokenfuse-gateway savings` run 2026-07-23 (this module's tests embed
/// the captured bytes) - this parse hard-fails (`TokenfuseError::Parse`)
/// rather than silently guessing when a line does not match, since a silent
/// guess would hide an upstream wording change from whoever depends on this
/// number.
fn parse_savings_text(text: &str) -> Result<TokenfuseSavings, TokenfuseError> {
    const CMD: &str = "savings";
    let mut lines = text.lines();
    let header = lines.next().ok_or_else(|| TokenfuseError::Parse {
        command: CMD,
        detail: "empty output".to_string(),
    })?;
    if header.contains("no trace yet") {
        return Ok(TokenfuseSavings {
            trace_data_found: false,
            ..Default::default()
        });
    }
    if !header.starts_with("TokenFuse savings") {
        return Err(TokenfuseError::Parse {
            command: CMD,
            detail: format!("unrecognized header line: {header:?}"),
        });
    }

    let mut out = TokenfuseSavings {
        trace_data_found: true,
        ..Default::default()
    };
    let (mut saw_runaway, mut saw_cache, mut saw_router) = (false, false, false);

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("runaway spend stopped:") {
            let rest = rest.trim();
            let (money_str, paren_rest) =
                rest.split_once('(').ok_or_else(|| TokenfuseError::Parse {
                    command: CMD,
                    detail: format!("no `(...)` clause in runaway line: {line:?}"),
                })?;
            out.blocked_spend_microusd =
                parse_usd_to_microusd(money_str).ok_or_else(|| TokenfuseError::Parse {
                    command: CMD,
                    detail: format!("could not parse USD amount in runaway line: {line:?}"),
                })?;
            let clause =
                paren_rest
                    .trim_end()
                    .strip_suffix(')')
                    .ok_or_else(|| TokenfuseError::Parse {
                        command: CMD,
                        detail: format!("runaway line clause missing its closing paren: {line:?}"),
                    })?;
            let tokens: Vec<&str> = clause.split_whitespace().collect();
            out.blocked_calls = tokens.first().and_then(|t| t.parse().ok()).ok_or_else(|| {
                TokenfuseError::Parse {
                    command: CMD,
                    detail: format!("could not read the blocked-call count in: {line:?}"),
                }
            })?;
            let across_idx = tokens.iter().position(|t| *t == "across").ok_or_else(|| {
                TokenfuseError::Parse {
                    command: CMD,
                    detail: format!("expected the word \"across\" in runaway line: {line:?}"),
                }
            })?;
            out.budget_breaks_prevented = tokens
                .get(across_idx + 1)
                .and_then(|t| t.parse().ok())
                .ok_or_else(|| TokenfuseError::Parse {
                command: CMD,
                detail: format!("could not read the budget-break count in: {line:?}"),
            })?;
            saw_runaway = true;
        } else if let Some(rest) = trimmed.strip_prefix("cache saved:") {
            out.cache_saved_microusd =
                parse_usd_to_microusd(rest.trim()).ok_or_else(|| TokenfuseError::Parse {
                    command: CMD,
                    detail: format!("could not parse USD amount in cache-saved line: {line:?}"),
                })?;
            saw_cache = true;
        } else if let Some(rest) = trimmed.strip_prefix("router saved:") {
            out.router_saved_microusd =
                parse_usd_to_microusd(rest.trim()).ok_or_else(|| TokenfuseError::Parse {
                    command: CMD,
                    detail: format!("could not parse USD amount in router-saved line: {line:?}"),
                })?;
            saw_router = true;
        } else {
            // The only other line shape `savingscli.rs::run` ever prints: a
            // per-reason breakdown, `"<reason padded to 16 wide> <$amount>"`.
            // `split_whitespace` ignores the padding, so this is exactly two
            // tokens regardless of the reason string's length.
            let tokens: Vec<&str> = trimmed.split_whitespace().collect();
            let [reason, money] = tokens.as_slice() else {
                return Err(TokenfuseError::Parse {
                    command: CMD,
                    detail: format!("unrecognized savings output line: {line:?}"),
                });
            };
            let amount = parse_usd_to_microusd(money).ok_or_else(|| TokenfuseError::Parse {
                command: CMD,
                detail: format!("could not parse USD amount in reason line: {line:?}"),
            })?;
            out.by_reason_microusd.insert((*reason).to_string(), amount);
        }
    }

    if !(saw_runaway && saw_cache && saw_router) {
        return Err(TokenfuseError::Parse {
            command: CMD,
            detail: "missing one of the three expected summary lines (runaway/cache/router)"
                .to_string(),
        });
    }
    Ok(out)
}

/// Parse an Arrow `pretty_format_batches` ASCII table (the ONLY output shape
/// `tokenfuse sql` has - see this module's doc comment) into rows of trimmed
/// string cells. Border rows (starting with `+`) are dropped; every
/// surviving line is split on `|`. The first returned row is the header; a
/// completely empty result (`Vec::new()`) means no `|`-prefixed line was
/// found at all, which is exactly what `pretty_format_batches` prints for
/// ZERO result rows (`"++\n++\n"`, confirmed empirically 2026-07-23 both for
/// a genuinely empty trace and for a query matching no groups) - callers
/// treat that as legitimately-zero-rows, not a parse failure.
fn parse_pretty_table(text: &str) -> Vec<Vec<String>> {
    text.lines()
        .map(str::trim_end)
        .filter(|line| line.starts_with('|'))
        .map(|line| {
            line.trim_start_matches('|')
                .trim_end_matches('|')
                .split('|')
                .map(|cell| cell.trim().to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Parse one `run_cost_query` result into [`CostBreakdownRow`]s, keying
/// columns by the header's TEXT (not position) so a reordered `SELECT` list
/// still parses - only a renamed column or a changed border/delimiter
/// character breaks this.
fn parse_cost_rows(
    text: &str,
    label_column: &str,
) -> Result<Vec<CostBreakdownRow>, TokenfuseError> {
    const CMD: &str = "sql";
    let table = parse_pretty_table(text);
    let Some((header, data)) = table.split_first() else {
        // No `|`-prefixed lines at all - see `parse_pretty_table`'s doc
        // comment: this is the degenerate "zero rows" shape, not an error.
        return Ok(Vec::new());
    };
    let col = |name: &str| -> Result<usize, TokenfuseError> {
        header
            .iter()
            .position(|h| h == name)
            .ok_or_else(|| TokenfuseError::Parse {
                command: CMD,
                detail: format!("missing expected column `{name}` in header {header:?}"),
            })
    };
    let label_idx = col(label_column)?;
    let calls_idx = col("calls")?;
    let cost_idx = col("total_cost_microusd")?;
    let tool_calls_idx = col("total_tool_calls")?;
    let known_idx = col("tool_calls_known_rows")?;

    let mut out = Vec::with_capacity(data.len());
    for row in data {
        let get = |i: usize| -> Result<&str, TokenfuseError> {
            row.get(i)
                .map(String::as_str)
                .ok_or_else(|| TokenfuseError::Parse {
                    command: CMD,
                    detail: format!("row {row:?} is shorter than the header {header:?}"),
                })
        };
        let parse_u64 = |field: &'static str, i: usize| -> Result<u64, TokenfuseError> {
            get(i)?.parse().map_err(|_| TokenfuseError::Parse {
                command: CMD,
                detail: format!("non-numeric `{field}` cell in row {row:?}"),
            })
        };
        let label = get(label_idx)?.to_string();
        let calls = parse_u64("calls", calls_idx)?;
        let total_cost_microusd: i64 =
            get(cost_idx)?.parse().map_err(|_| TokenfuseError::Parse {
                command: CMD,
                detail: format!("non-numeric `total_cost_microusd` cell in row {row:?}"),
            })?;
        let total_tool_calls = parse_u64("total_tool_calls", tool_calls_idx)?;
        let tool_calls_known_rows = parse_u64("tool_calls_known_rows", known_idx)?;
        let cost_per_tool_call_microusd = if tool_calls_known_rows == 0 || total_tool_calls == 0 {
            None
        } else {
            Some(total_cost_microusd / total_tool_calls as i64)
        };
        out.push(CostBreakdownRow {
            label,
            calls,
            total_cost_microusd,
            total_tool_calls,
            tool_calls_known_rows,
            cost_per_tool_call_microusd,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_export_missing_binary_is_fail_closed_spawn_error() {
        let c = TokenfuseClient::new("/nonexistent/tokenfuse-binary-xyz");
        match c.focus_export(Path::new("/traces"), None, None) {
            Err(TokenfuseError::Spawn { .. }) => {}
            other => panic!("expected Spawn error, got {other:?}"),
        }
    }

    #[test]
    fn savings_missing_binary_is_fail_closed_spawn_error() {
        let c = TokenfuseClient::new("/nonexistent/tokenfuse-binary-xyz");
        match c.savings(Path::new("/traces")) {
            Err(TokenfuseError::Spawn { .. }) => {}
            other => panic!("expected Spawn error, got {other:?}"),
        }
    }

    #[test]
    fn cost_per_action_missing_binary_is_fail_closed_spawn_error() {
        let c = TokenfuseClient::new("/nonexistent/tokenfuse-binary-xyz");
        match c.cost_per_action(Path::new("/traces")) {
            Err(TokenfuseError::Spawn { .. }) => {}
            other => panic!("expected Spawn error, got {other:?}"),
        }
    }

    // ---- parse_usd_to_microusd -------------------------------------------

    #[test]
    fn parses_a_microusd_amount_exactly_no_float_drift() {
        assert_eq!(parse_usd_to_microusd("$28750.000404"), Some(28_750_000_404));
        assert_eq!(parse_usd_to_microusd("$0.000000"), Some(0));
        assert_eq!(
            parse_usd_to_microusd("  $14375.000202  "),
            Some(14_375_000_202)
        );
        assert_eq!(parse_usd_to_microusd("$5"), Some(5_000_000)); // no fractional part at all
        assert_eq!(parse_usd_to_microusd("not money"), None);
        assert_eq!(parse_usd_to_microusd("$abc.def"), None);
    }

    // ---- parse_savings_text: REAL captured CLI output --------------------
    //
    // These three fixtures are the EXACT bytes `tokenfuse-gateway savings`
    // printed on this box, 2026-07-23, against `~/.taipan/bin/
    // tokenfuse-gateway` and two real trace directories
    // (`~/.taipan/environments/{p2exit,p2gate}.traces/gateway`) plus a
    // nonexistent dir for the empty case. The em dash `savingscli.rs` prints
    // is written as `\u{2014}` (an escape, not the literal glyph) purely so
    // this source file's own bytes stay em-dash-free per house style; the
    // PARSED bytes at runtime are identical to what the real binary printed
    // (verified byte-for-byte via `python3 -c "print(repr(open(...,'rb')...))"`
    // during development).

    #[test]
    fn parses_real_captured_savings_output_p2exit() {
        let text = "TokenFuse savings \u{2014} from /Users/factory/.taipan/environments/p2exit.traces/gateway\n  runaway spend stopped:   $28750.000404   (2 blocked call(s) across 1 budget break(s))\n  cache saved:             $0.000000\n  router saved:            $0.000000\n    budget_exceeded  $28750.000404\n";
        let s = parse_savings_text(text).expect("real captured output must parse");
        assert!(s.trace_data_found);
        assert_eq!(s.blocked_spend_microusd, 28_750_000_404);
        assert_eq!(s.blocked_calls, 2);
        assert_eq!(s.budget_breaks_prevented, 1);
        assert_eq!(s.cache_saved_microusd, 0);
        assert_eq!(s.router_saved_microusd, 0);
        assert_eq!(
            s.by_reason_microusd.get("budget_exceeded").copied(),
            Some(28_750_000_404)
        );
        assert_eq!(s.by_reason_microusd.len(), 1);
    }

    #[test]
    fn parses_real_captured_savings_output_p2gate() {
        let text = "TokenFuse savings \u{2014} from /Users/factory/.taipan/environments/p2gate.traces/gateway\n  runaway spend stopped:   $14375.000202   (1 blocked call(s) across 1 budget break(s))\n  cache saved:             $0.000000\n  router saved:            $0.000000\n    budget_exceeded  $14375.000202\n";
        let s = parse_savings_text(text).expect("real captured output must parse");
        assert!(s.trace_data_found);
        assert_eq!(s.blocked_spend_microusd, 14_375_000_202);
        assert_eq!(s.blocked_calls, 1);
        assert_eq!(s.budget_breaks_prevented, 1);
        assert_eq!(
            s.by_reason_microusd.get("budget_exceeded").copied(),
            Some(14_375_000_202)
        );
    }

    #[test]
    fn parses_real_captured_no_trace_yet_output() {
        let text = "TokenFuse savings \u{2014} no trace yet at /tmp/nonexistent-trace-dir-xyz\n  set TOKENFUSE_DATA_DIR and run some traffic, then try again.\n";
        let s = parse_savings_text(text).expect("the friendly empty message must parse");
        assert!(!s.trace_data_found);
        assert_eq!(s, TokenfuseSavings::default());
    }

    // ---- parse_savings_text: hand-built fixture ---------------------------
    //
    // Every trace on this box happens to hit exactly one budget-protection
    // reason (see the two real-captured tests above), so a MULTI-reason
    // report, and a budget-break count that differs from the blocked-call
    // count, are not naturally available to capture locally. This fixture is
    // HAND-WRITTEN (not captured) to the exact format `savingscli.rs::run`'s
    // source confirms (`crates/gateway/src/savingscli.rs`, read 2026-07-23),
    // to exercise those two branches the real captures above do not reach.

    #[test]
    fn parses_hand_built_multi_reason_savings_fixture() {
        let text = "TokenFuse savings \u{2014} from /tmp/synthetic\n  runaway spend stopped:   $3.500000   (3 blocked call(s) across 2 budget break(s))\n  cache saved:             $0.750000\n  router saved:            $0.100000\n    budget_exceeded  $1.500000\n    loop_detected    $2.000000\n";
        let s = parse_savings_text(text)
            .expect("hand-built fixture in the confirmed format must parse");
        assert!(s.trace_data_found);
        assert_eq!(s.blocked_spend_microusd, 3_500_000);
        assert_eq!(s.blocked_calls, 3);
        assert_eq!(s.budget_breaks_prevented, 2);
        assert_eq!(s.cache_saved_microusd, 750_000);
        assert_eq!(s.router_saved_microusd, 100_000);
        assert_eq!(
            s.by_reason_microusd.get("budget_exceeded").copied(),
            Some(1_500_000)
        );
        assert_eq!(
            s.by_reason_microusd.get("loop_detected").copied(),
            Some(2_000_000)
        );
        // The breakdown sums back to the headline figure, same invariant
        // `tokenfuse_core::savings` itself tests.
        let sum: i64 = s.by_reason_microusd.values().sum();
        assert_eq!(sum, s.blocked_spend_microusd);
    }

    #[test]
    fn savings_text_missing_a_summary_line_is_a_parse_error() {
        let text = "TokenFuse savings \u{2014} from /tmp/x\n  runaway spend stopped:   $1.000000   (1 blocked call(s) across 1 budget break(s))\n";
        match parse_savings_text(text) {
            Err(TokenfuseError::Parse {
                command: "savings", ..
            }) => {}
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn savings_text_with_garbage_header_is_a_parse_error() {
        match parse_savings_text("not a tokenfuse report at all\n") {
            Err(TokenfuseError::Parse {
                command: "savings", ..
            }) => {}
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    // ---- parse_pretty_table / parse_cost_rows -----------------------------

    #[test]
    fn parses_real_captured_per_model_cost_table() {
        // The EXACT bytes `tokenfuse-gateway sql "<PER_MODEL_COST_QUERY>"`
        // printed against the real `p2exit` trace, 2026-07-23. Every row
        // shows `tool_calls_known_rows == 0`: this particular trace predates
        // the I1 `tool_calls` column, which is itself a real (not
        // hypothetical) instance of the "unknown, not zero" case.
        let text = "+-----------------+-------+---------------------+------------------+-----------------------+\n\
                    | model           | calls | total_cost_microusd | total_tool_calls | tool_calls_known_rows |\n\
                    +-----------------+-------+---------------------+------------------+-----------------------+\n\
                    | claude-haiku    | 26    | 5600                | 0                | 0                     |\n\
                    | claude-opus-4-5 | 26    | 28750000404         | 0                | 0                     |\n\
                    | claude-sonnet   | 47    | 241500              | 0                | 0                     |\n\
                    +-----------------+-------+---------------------+------------------+-----------------------+\n";
        let rows = parse_cost_rows(text, "model").expect("real captured table must parse");
        assert_eq!(rows.len(), 3);
        let opus = rows
            .iter()
            .find(|r| r.label == "claude-opus-4-5")
            .expect("opus row present");
        assert_eq!(opus.calls, 26);
        assert_eq!(opus.total_cost_microusd, 28_750_000_404);
        assert_eq!(opus.total_tool_calls, 0);
        assert_eq!(opus.tool_calls_known_rows, 0);
        // Unknown (pre-I1 trace), not a real zero - must not fabricate a rate.
        assert_eq!(opus.cost_per_tool_call_microusd, None);
    }

    #[test]
    fn parses_real_captured_per_agent_cost_table() {
        // The EXACT bytes for the same trace, grouped by `agent_id` instead.
        let text = "+----------------------------------------------+-------+---------------------+------------------+-----------------------+\n\
                    | agent_id                                     | calls | total_cost_microusd | total_tool_calls | tool_calls_known_rows |\n\
                    +----------------------------------------------+-------+---------------------+------------------+-----------------------+\n\
                    | agent://mockryx.local/rehearsal/ops-helper   | 26    | 5600                | 0                | 0                     |\n\
                    | agent://mockryx.local/rehearsal/payments-bot | 73    | 28750241904         | 0                | 0                     |\n\
                    +----------------------------------------------+-------+---------------------+------------------+-----------------------+\n";
        let rows = parse_cost_rows(text, "agent_id").expect("real captured table must parse");
        assert_eq!(rows.len(), 2);
        let bot = rows
            .iter()
            .find(|r| r.label == "agent://mockryx.local/rehearsal/payments-bot")
            .expect("payments-bot row present");
        assert_eq!(bot.calls, 73);
        assert_eq!(bot.total_cost_microusd, 28_750_241_904);
    }

    #[test]
    fn hand_built_fixture_with_known_nonzero_tool_calls_computes_a_rate() {
        // No locally-available trace has a non-null `tool_calls` column (see
        // the module doc comment on `PER_MODEL_COST_QUERY`), so this table is
        // HAND-BUILT in the same confirmed format to exercise the `Some(..)`
        // branch of `cost_per_tool_call_microusd`.
        let text = "+-------+-------+----------------------+-------------------+------------------------+\n\
                    | model | calls | total_cost_microusd | total_tool_calls  | tool_calls_known_rows  |\n\
                    +-------+-------+----------------------+-------------------+------------------------+\n\
                    | m1    | 10    | 1000000              | 20                | 10                     |\n\
                    +-------+-------+----------------------+-------------------+------------------------+\n";
        let rows = parse_cost_rows(text, "model").expect("hand-built fixture must parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "m1");
        assert_eq!(rows[0].total_tool_calls, 20);
        assert_eq!(rows[0].tool_calls_known_rows, 10);
        assert_eq!(rows[0].cost_per_tool_call_microusd, Some(50_000)); // 1_000_000 / 20
    }

    #[test]
    fn degenerate_empty_table_is_zero_rows_not_an_error() {
        // The EXACT bytes `pretty_format_batches` prints for zero result
        // rows - confirmed empirically both for a genuinely empty trace
        // directory and for a real trace whose query matched no groups
        // (2026-07-23). Must be `Ok(vec![])`, never `Err`.
        let rows = parse_cost_rows("++\n++\n", "model").expect("degenerate table is not an error");
        assert!(rows.is_empty());
    }

    #[test]
    fn completely_empty_stdout_is_also_zero_rows_not_an_error() {
        let rows = parse_cost_rows("", "model").expect("empty stdout is not an error");
        assert!(rows.is_empty());
    }

    #[test]
    fn cost_table_missing_an_expected_column_is_a_parse_error() {
        let text = "+-------+-------+\n| model | calls |\n+-------+-------+\n| m1    | 10    |\n+-------+-------+\n";
        match parse_cost_rows(text, "model") {
            Err(TokenfuseError::Parse { command: "sql", .. }) => {}
            other => {
                panic!("expected Parse error (missing total_cost_microusd etc.), got {other:?}")
            }
        }
    }

    // ---- live skip-graceful: the real binary + a real trace, if present ---
    //
    // Mirrors `crypto.rs`'s `live_crypto_scan_when_qryx_is_installed`: if this
    // box happens to have the real tokenfuse-gateway binary AND a populated
    // trace dir, exercise the real CLI end to end; otherwise skip cleanly
    // rather than fail on a box that legitimately has neither.

    #[test]
    fn live_savings_and_cost_per_action_when_tokenfuse_is_installed() {
        let home = std::env::var("HOME").unwrap_or_default();
        let bin = PathBuf::from(format!("{home}/.taipan/bin/tokenfuse-gateway"));
        if !bin.is_file() {
            eprintln!(
                "SKIP live tokenfuse test: no tokenfuse-gateway at {}",
                bin.display()
            );
            return;
        }
        // Prefer a traces dir this development box is known to have real
        // Parquet data under (see this module's doc comment); fall back to
        // skipping if neither is present rather than guessing another path.
        let candidates = [
            format!("{home}/.taipan/environments/p2exit.traces/gateway"),
            format!("{home}/.taipan/environments/p2gate.traces/gateway"),
        ];
        let Some(traces_dir) = candidates.iter().map(PathBuf::from).find(|p| p.is_dir()) else {
            eprintln!("SKIP live tokenfuse test: no populated traces dir among {candidates:?}");
            return;
        };

        let client = TokenfuseClient::new(&bin);
        let savings = client
            .savings(&traces_dir)
            .expect("tokenfuse is installed but `savings` errored");
        assert!(
            savings.trace_data_found,
            "the chosen traces dir should have data"
        );

        let report = client
            .cost_per_action(&traces_dir)
            .expect("tokenfuse is installed but `cost_per_action` errored");
        assert!(
            !report.by_model.is_empty(),
            "expected at least one model row from a populated trace"
        );
        eprintln!(
            "live tokenfuse OK against {}: blocked_spend_microusd={}, {} model rows, {} agent rows",
            traces_dir.display(),
            savings.blocked_spend_microusd,
            report.by_model.len(),
            report.by_agent.len()
        );
    }
}
