use super::*;

/// What `tokenfuse backtest --budget 0.002` printed over a lab trace (tokenfuse
/// v1.20.3), byte for byte in shape.
const TRACE_REPORT: &str = "TokenFuse backtest
  candidate policy   : budget/run $0.0020
  runs               : 230 (15 affected)
  calls              : 832 (70 would be blocked)
  spend in trace     : $0.256379
  would have saved   : $0.204439 (79.7%)
  projected spend    : $0.051940
";

/// The same candidate over a trace that holds one run only.
const RUN_REPORT: &str = "TokenFuse backtest
  candidate policy   : budget/run $0.0020
  runs               : 1 (1 affected)
  calls              : 35 (18 would be blocked)
  spend in trace     : $0.005674
  would have saved   : $0.003810 (67.1%)
  projected spend    : $0.001864
";

/// All three limits at once, over a run with calls at the same step.
const THREE_LIMITS_REPORT: &str = "TokenFuse backtest
  candidate policy   : budget/run $0.0001, budget/step $0.0001, max_steps 10
  runs               : 1 (1 affected)
  calls              : 155 (145 would be blocked)
  spend in trace     : $0.000116
  would have saved   : $0.000080 (69.0%)
  projected spend    : $0.000036
";

const COPY_TABLE: &str = "+-------+
| count |
+-------+
| 35    |
+-------+
";

const STEPS_TABLE: &str = "+-----+-------+
| n   | steps |
+-----+-------+
| 155 | 29    |
+-----+-------+
";

fn run_budget() -> BudgetCandidate {
    BudgetCandidate::from_parts(Some("0.002"), None, None).expect("a candidate")
}

fn three_limits() -> BudgetCandidate {
    BudgetCandidate::from_parts(Some("0.0001"), Some("0.00005"), Some(10)).expect("a candidate")
}

/// A pseudo-random stream, the same for the same seed.
fn next(x: &mut u64) -> u64 {
    *x = x
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *x >> 33
}

#[test]
fn amounts_are_whole_micro_dollars_or_refused() {
    for (text, micros) in [
        ("2", 2_000_000),
        ("0.35", 350_000),
        ("0.000249", 249),
        ("0.000001", 1),
        ("12.5", 12_500_000),
        ("1000000", MAX_LIMIT_MICROS),
    ] {
        assert_eq!(parse_usd(text), Ok(micros), "{text}");
    }
    for refused in [
        "",
        ".5",
        "5.",
        "-1",
        "+1",
        "1e3",
        "0x10",
        "1.2.3",
        "0.0000001",
        "0",
        "0.000000",
        "1000000.000001",
        "1000001",
        " 1",
        "1 ",
        "1,5",
        "\u{663}",
        "99999999999999999999",
    ] {
        assert!(parse_usd(refused).is_err(), "{refused:?} was accepted");
    }
}

#[test]
fn an_amount_is_handed_to_the_cli_as_text_it_reads_back_exactly() {
    // The CLI reads a flag as `(v * 1e6) as i64`. The plain text of 249
    // micro-dollars is read as 248.
    let plain = (249f64 / 1e6).to_string();
    assert_eq!((plain.parse::<f64>().expect("a number") * 1e6) as i64, 248);

    let text = cli_amount(249).expect("an amount");
    assert_eq!((text.parse::<f64>().expect("a number") * 1e6) as i64, 249);

    for micros in (1..=60_000).chain([MAX_LIMIT_MICROS - 1, MAX_LIMIT_MICROS]) {
        let text = cli_amount(micros).unwrap_or_else(|e| panic!("{micros}: {e}"));
        assert_eq!(
            (text.parse::<f64>().expect("a number") * 1e6) as i64,
            micros,
            "{micros} read back from {text}"
        );
    }
    for seed in 0..200u64 {
        let mut x = seed.wrapping_add(7);
        let micros = 1 + (next(&mut x) * next(&mut x)) as i64 % MAX_LIMIT_MICROS;
        let text = cli_amount(micros).unwrap_or_else(|e| panic!("{micros}: {e}"));
        assert_eq!(
            (text.parse::<f64>().expect("a number") * 1e6) as i64,
            micros,
            "seed {seed}: {micros} read back from {text}"
        );
    }
    for out_of_range in [-5, 0, MAX_LIMIT_MICROS + 1, i64::MAX, i64::MIN] {
        assert!(cli_amount(out_of_range).is_err(), "{out_of_range}");
    }
}

#[test]
fn a_candidate_names_what_it_limits_and_hands_over_only_what_it_can() {
    for (run, step, steps) in [(None, None, None), (Some("abc"), None, None)] {
        assert!(BudgetCandidate::from_parts(run, step, steps).is_err());
    }
    assert!(BudgetCandidate::from_parts(None, None, Some(0)).is_err());
    assert!(BudgetCandidate::from_parts(None, None, Some(MAX_STEPS_LIMIT + 1)).is_err());
    assert!(BudgetCandidate::from_parts(None, Some("0"), None).is_err());

    let all = BudgetCandidate::from_parts(Some("2"), Some("0.01"), Some(20)).expect("candidate");
    assert_eq!(all.per_run_micros, Some(2_000_000));
    assert_eq!(all.per_step_micros, Some(10_000));
    assert_eq!(all.max_steps, Some(20));
    let args = all.args().expect("args");
    assert_eq!(args.len(), 6);
    assert_eq!(args[0], "--budget");
    assert_eq!(args[2], "--budget-per-step");
    assert_eq!(args[4], "--max-steps");
    assert_eq!(args[5], "20");
    assert_eq!((args[1].parse::<f64>().unwrap() * 1e6) as i64, 2_000_000);
    assert_eq!((args[3].parse::<f64>().unwrap() * 1e6) as i64, 10_000);
    assert_eq!(
        all.describe(),
        "budget/run $2.0000, budget/step $0.0100, max_steps 20"
    );

    let one = BudgetCandidate::from_parts(None, None, Some(3)).expect("candidate");
    assert_eq!(one.args().expect("args"), ["--max-steps", "3"]);
    assert_eq!(one.describe(), "max_steps 3");
}

#[test]
fn a_run_id_is_put_in_a_query_only_when_it_is_a_plain_name() {
    for plain in ["r-incident", "w2-forge-long-d1gw-135654", "run:1.2_3", "A"] {
        assert_eq!(query_safe_run(plain), Ok(()), "{plain}");
    }
    let too_long = "x".repeat(RUN_ID_MAX_BYTES + 1);
    for refused in [
        "",
        "a'b",
        "a b",
        "a;b",
        "a\"b",
        "a\\b",
        "a\nb",
        "a/b",
        "a\0b",
        "r\u{e9}",
        too_long.as_str(),
    ] {
        assert!(query_safe_run(refused).is_err(), "{refused:?}");
    }
    assert_eq!(
        copy_statement("r1", "/tmp/p/part.parquet").expect("a statement"),
        "copy (select * from calls where run_id = 'r1' order by ts_millis, step) \
         to '/tmp/p/part.parquet' stored as parquet"
    );
    for file in ["", "/tmp/a'b", "/tmp/a b", "/tmp/a;b", "/tmp/\u{e9}"] {
        assert!(copy_statement("r1", file).is_err(), "{file:?}");
    }

    // Whatever a run id is made of, a statement that was built holds the id
    // once, between its quotes, and nothing that could end the quotes early.
    let alphabet: Vec<char> = "ab-_.:'\" ;\\/\n\0\u{e9}x1".chars().collect();
    for seed in 0..200u64 {
        let mut x = seed.wrapping_add(11);
        let len = 1 + next(&mut x) as usize % 12;
        let run: String = (0..len)
            .map(|_| alphabet[next(&mut x) as usize % alphabet.len()])
            .collect();
        if let Ok(statement) = copy_statement(&run, "/tmp/p/part.parquet") {
            assert!(!run.contains('\''), "seed {seed}: {run:?}");
            assert_eq!(statement.matches('\'').count(), 4, "seed {seed}: {run:?}");
            assert!(statement.contains(&format!("run_id = '{run}' order")));
        }
    }
}

#[test]
fn a_table_is_read_as_tokenfuse_sql_prints_it() {
    assert_eq!(parse_copy_count(COPY_TABLE), Some(35));
    assert_eq!(parse_step_counts(STEPS_TABLE), Some((155, 29)));
    let zero = COPY_TABLE.replace("| 35    |", "| 0     |");
    assert_eq!(parse_copy_count(&zero), Some(0));

    for not_a_count in [
        "",
        "sql error: Schema error: No field named nope.",
        "+-------+\n| count |\n+-------+\n+-------+\n",
        "+-------+\n| total |\n+-------+\n| 35    |\n+-------+\n",
        "+-------+\n| count |\n+-------+\n| 3x    |\n+-------+\n",
        "+-------+\n| count |\n+-------+\n| -3    |\n+-------+\n",
        "+-------+\n| count |\n+-------+\n| +3    |\n+-------+\n",
        "+-------+\n| count |\n+-------+\n| 35    |\n| 36    |\n+-------+\n",
        "| count |\n| 35    |\n",
    ] {
        assert_eq!(parse_copy_count(not_a_count), None, "{not_a_count:?}");
        assert_eq!(parse_step_counts(not_a_count), None, "{not_a_count:?}");
    }
    assert_eq!(parse_step_counts(COPY_TABLE), None);
    assert_eq!(parse_copy_count(STEPS_TABLE), None);

    let pieces = [
        "+-----+-------+",
        "| n   | steps |",
        "| count |",
        "| 155 | 29    |",
        "| 35    |",
        "|",
        "+",
        "",
    ];
    for seed in 0..200u64 {
        let mut x = seed.wrapping_add(3);
        let mut text = String::new();
        for _ in 0..(seed % 9) {
            text.push_str(pieces[next(&mut x) as usize % pieces.len()]);
            text.push('\n');
        }
        // Whatever it is, it is read or it is not, never a panic.
        let _ = parse_copy_count(&text);
        let _ = parse_step_counts(&text);
    }
}

#[test]
fn the_report_is_read_as_backtest_prints_it() {
    assert_eq!(
        parse_backtest(TRACE_REPORT, &run_budget()),
        Ok(BacktestReport {
            runs_total: 230,
            runs_affected: 15,
            calls_total: 832,
            calls_blocked: 70,
            spent_micros: 256_379,
            saved_micros: 204_439,
        })
    );
    assert_eq!(
        parse_backtest(RUN_REPORT, &run_budget()),
        Ok(BacktestReport {
            runs_total: 1,
            runs_affected: 1,
            calls_total: 35,
            calls_blocked: 18,
            spent_micros: 5_674,
            saved_micros: 3_810,
        })
    );
    let three = parse_backtest(THREE_LIMITS_REPORT, &three_limits()).expect("parses");
    assert_eq!(three.calls_blocked, 145);
    assert_eq!(three.spent_micros, 116);
    assert_eq!(three.saved_micros, 80);
}

#[test]
fn an_empty_trace_is_no_calls_and_not_a_report() {
    assert_eq!(
        parse_backtest("no calls in the trace at /tmp/x\n", &run_budget()),
        Err(BacktestParseError::NoCalls)
    );
    for not_a_report in [
        "",
        "no candidate policy given. Try: tokenfuse backtest --budget 2.0 --max-steps 20",
        "backtest error: sql error",
        "TokenFuse backtest",
        "TokenFuse backtest\n  candidate policy   : budget/run $0.0020\n",
    ] {
        assert_eq!(
            parse_backtest(not_a_report, &run_budget()),
            Err(BacktestParseError::Unrecognised),
            "{not_a_report:?}"
        );
    }
}

#[test]
fn a_backtest_report_for_another_candidate_is_refused() {
    // The CLI drops a flag it cannot read and runs without it, so what it says
    // it ran is what is compared with what was asked.
    let other = BudgetCandidate::from_parts(Some("0.003"), None, None).expect("candidate");
    assert!(matches!(
        parse_backtest(TRACE_REPORT, &other),
        Err(BacktestParseError::Inconsistent(_))
    ));
    let with_a_step_limit = BudgetCandidate::from_parts(Some("0.002"), None, Some(5)).unwrap();
    assert!(matches!(
        parse_backtest(TRACE_REPORT, &with_a_step_limit),
        Err(BacktestParseError::Inconsistent(_))
    ));
}

#[test]
fn a_backtest_report_that_contradicts_itself_is_refused() {
    for (what, from, to) in [
        (
            "more runs affected than there are",
            "230 (15 affected)",
            "230 (300 affected)",
        ),
        (
            "more calls blocked than there are",
            "832 (70 would be blocked)",
            "832 (900 would be blocked)",
        ),
        (
            "more runs affected than calls blocked",
            "832 (70 would be blocked)",
            "832 (10 would be blocked)",
        ),
        (
            "calls blocked in no run",
            "230 (15 affected)",
            "230 (0 affected)",
        ),
        (
            "a projection that is not the spend less the saving",
            "$0.051940",
            "$0.051941",
        ),
    ] {
        let text = TRACE_REPORT.replace(from, to);
        assert!(
            matches!(
                parse_backtest(&text, &run_budget()),
                Err(BacktestParseError::Inconsistent(_))
            ),
            "{what}: {:?}",
            parse_backtest(&text, &run_budget())
        );
    }
}

#[test]
fn text_that_is_not_a_backtest_report_is_refused_and_a_200_seed_sweep_never_panics() {
    let pieces = [
        "TokenFuse backtest",
        "  candidate policy   : budget/run $0.0020",
        "  runs               : 230 (15 affected)",
        "  calls              : 832 (70 would be blocked)",
        "  spend in trace     : $0.256379",
        "  would have saved   : $0.204439 (79.7%)",
        "  projected spend    : $0.051940",
        "  runs               : 1 (1 affected)",
        "  spend in trace     : $-1.000000",
        "no calls in the trace at x",
        "\0",
        "",
    ];
    for seed in 0..200u64 {
        let mut x = seed.wrapping_add(5);
        let mut text = String::new();
        for _ in 0..(seed % 9) {
            text.push_str(pieces[next(&mut x) as usize % pieces.len()]);
            text.push('\n');
        }
        // Read or refused, never a panic, and a report that was read holds
        // together.
        if let Ok(report) = parse_backtest(&text, &run_budget()) {
            assert!(report.runs_affected <= report.runs_total, "seed {seed}");
            assert!(report.calls_blocked <= report.calls_total, "seed {seed}");
            assert!(report.runs_affected <= report.calls_blocked, "seed {seed}");
        }
    }
}

fn report(runs: (u64, u64), calls: (u64, u64), spent: i64, saved: i64) -> BacktestReport {
    BacktestReport {
        runs_total: runs.0,
        runs_affected: runs.1,
        calls_total: calls.0,
        calls_blocked: calls.1,
        spent_micros: spent,
        saved_micros: saved,
    }
}

fn the_run() -> BacktestReport {
    parse_backtest(RUN_REPORT, &run_budget()).expect("parses")
}

fn the_trace() -> BacktestReport {
    parse_backtest(TRACE_REPORT, &run_budget()).expect("parses")
}

#[test]
fn the_incident_is_stopped_only_when_its_own_run_has_a_call_blocked() {
    let stopped = judge_budget("r1", &run_budget(), &the_run(), &the_trace(), 35, false)
        .expect("a proof");
    assert!(stopped.stops_this_incident);

    // The candidate blocks calls elsewhere in the trace and none in this run.
    let untouched = report((1, 0), (35, 0), 5_674, 0);
    let elsewhere_only = judge_budget("r1", &run_budget(), &untouched, &the_trace(), 35, false)
        .expect("a proof");
    assert!(!elsewhere_only.stops_this_incident);
    assert_eq!(elsewhere_only.incident.calls_blocked, 0);
    assert_eq!(elsewhere_only.trace.calls_blocked, 70);
    assert_eq!(elsewhere_only.incident.spent_after_micros, 5_674);
}

#[test]
fn the_rest_of_the_trace_is_the_whole_trace_less_the_incident() {
    let proof = judge_budget("r1", &run_budget(), &the_run(), &the_trace(), 35, false)
        .expect("a proof");
    assert_eq!(proof.run, Data::bounded("r1"));
    assert_eq!(proof.candidate, run_budget());
    assert_eq!(proof.incident.calls, 35);
    assert_eq!(proof.incident.calls_blocked, 18);
    assert_eq!(proof.incident.spent_micros, 5_674);
    assert_eq!(proof.incident.saved_micros, 3_810);
    assert_eq!(proof.incident.spent_after_micros, 1_864);
    assert_eq!(proof.elsewhere.runs, 229);
    assert_eq!(proof.elsewhere.runs_affected, 14);
    assert_eq!(proof.elsewhere.calls, 797);
    assert_eq!(proof.elsewhere.calls_blocked, 52);
    assert_eq!(proof.elsewhere.saved_micros, 200_629);
    assert_eq!(proof.trace, the_trace());
    assert_eq!(proof.covers, BUDGET_COVERS);
    assert!(proof.elsewhere_exact);
}

#[test]
fn a_run_with_calls_at_the_same_step_marks_the_split_approximate() {
    let tied = judge_budget("r1", &run_budget(), &the_run(), &the_trace(), 35, true)
        .expect("a proof");
    assert!(!tied.elsewhere_exact);
    // The numbers are still given; only their claim to be exact is withdrawn.
    assert_eq!(tied.elsewhere.runs_affected, 14);
}

#[test]
fn reports_that_cannot_describe_the_same_trace_are_refused() {
    let candidate = run_budget();
    let trace = the_trace();
    let run = the_run();

    let two_runs = report((2, 1), (35, 18), 5_674, 3_810);
    let copy_disagrees = judge_budget("r1", &candidate, &run, &trace, 34, false);
    let with_other_runs = judge_budget("r1", &candidate, &two_runs, &trace, 35, false);
    let no_runs = report((0, 0), (0, 0), 0, 0);
    let empty_trace = judge_budget("r1", &candidate, &run, &no_runs, 35, false);
    let small = report((1, 1), (10, 5), 100, 50);
    let fewer_calls = judge_budget("r1", &candidate, &run, &small, 35, false);
    let few_blocks = report((20, 15), (832, 10), 256_379, 1);
    let blocks_less = judge_budget("r1", &candidate, &run, &few_blocks, 35, false);
    let few_runs = report((20, 0), (832, 70), 256_379, 1);
    let affects_less = judge_budget("r1", &candidate, &run, &few_runs, 35, false);

    for (what, result) in [
        ("a copy that disagrees with its replay", copy_disagrees),
        ("a replay of the run with other runs", with_other_runs),
        ("an empty whole trace", empty_trace),
        ("fewer calls in the trace than in the run", fewer_calls),
        ("fewer blocked calls in the trace than in the run", blocks_less),
        ("fewer affected runs in the trace than in the run", affects_less),
    ] {
        assert!(result.is_err(), "{what} was believed");
    }
}
