//! Invariant 54: Verify, with stand-in `trailryx-node` and `trailryx-verify`
//! that record how they were run. The seal reading itself is held in
//! `crates/api/src/record/tests.rs`.

use genaryx_api::proof::slots::Slots;
use genaryx_api::record::{RecordConfig, VerifyState, WITNESS_ENV, verify_in};
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use trailryx_index::{Dimension, SegmentManifest};
use trailryx_record::{Algorithms, Hash, SegmentId, ShardIx, Timestamp};
use trailryx_store::evidence::encode_manifest;

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-record-test-{tag}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create_dir_all");
    dir
}

fn manifest(segment: u64) -> SegmentManifest {
    SegmentManifest {
        format_version: 1,
        segment: SegmentId(segment),
        shard: ShardIx(0),
        records: 3,
        history_root: Hash([7u8; 48]),
        index_roots: Dimension::ALL
            .iter()
            .map(|d| (*d, Hash([9u8; 48])))
            .collect(),
        chain_before: Hash::ZERO,
        chain_after: Hash([2u8; 48]),
        first_recorded_at: Timestamp(1_763_900_000_000_000_000),
        last_recorded_at: Timestamp(1_763_900_001_000_000_000),
        algorithms: Algorithms::default(),
    }
}

/// A data directory with one sealed segment of three records.
fn data_dir(tag: &str) -> PathBuf {
    let dir = temp_dir(&format!("{tag}-data"));
    std::fs::write(dir.join("s0-000001.mf"), encode_manifest(&manifest(1))).expect("mf");
    dir
}

/// A `trailryx-node` that records its arguments and environment, prints the
/// summary line the real one prints for the store `data_dir` builds, writes a
/// pack where `--pack` points, and exits `code`. With `plant`, it copies that
/// file into the data directory first, as a node sealing a segment would.
fn fake_node(dir: &Path, code: i32, sleep: u32) -> PathBuf {
    fake_node_with(dir, code, sleep, None, "")
}

fn fake_node_with(
    dir: &Path,
    code: i32,
    sleep: u32,
    plant: Option<&Path>,
    stderr: &str,
) -> PathBuf {
    let d = dir.display();
    std::fs::write(dir.join("node-err.txt"), stderr).expect("stderr");
    let plant_line = match plant {
        Some(p) => format!("cp '{}' \"$data/s0-000002.mf\"\n", p.display()),
        None => String::new(),
    };
    let script = format!(
        "#!/bin/sh\n\
         printf '%s\\n' \"$@\" > '{d}/node-args.txt'\n\
         env | sort > '{d}/node-env.txt'\n\
         prev=''\n\
         for a in \"$@\"; do\n\
           if [ \"$prev\" = '--data' ]; then data=\"$a\"; fi\n\
           if [ \"$prev\" = '--pack' ]; then printf 'pack' > \"$a\"; echo \"$a\" > '{d}/pack.path'; fi\n\
           prev=\"$a\"\n\
         done\n\
         {plant_line}\
         echo \"1 sealed segment(s), 3 record(s), in $data\"\n\
         cat '{d}/node-err.txt' 1>&2\n\
         if [ {sleep} -gt 0 ]; then sleep {sleep}; fi\n\
         exit {code}\n"
    );
    let bin = dir.join("trailryx-node");
    std::fs::write(&bin, script).expect("script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    bin
}

/// A `trailryx-verify` that records how it ran and prints `stdout`.
fn fake_verify(dir: &Path, stdout: &str, code: i32) -> PathBuf {
    std::fs::write(dir.join("verify-out.txt"), stdout).expect("out");
    let d = dir.display();
    let script = format!(
        "#!/bin/sh\n\
         printf '%s\\n' \"$@\" > '{d}/verify-args.txt'\n\
         env | sort > '{d}/verify-env.txt'\n\
         cat '{d}/verify-out.txt'\n\
         exit {code}\n"
    );
    let bin = dir.join("trailryx-verify");
    std::fs::write(&bin, script).expect("script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    bin
}

fn cfg(data: &Path, node: PathBuf, verify: PathBuf, witness: Option<&str>) -> RecordConfig {
    RecordConfig {
        data_dir: Ok(Some(data.to_path_buf())),
        witness_dir: witness.map(OsString::from),
        node_bin: node,
        verify_bin: verify,
    }
}

const VERIFIED_OUT: &str =
    "[note] root-signature: \"es384\" by key 0a1b2c\n3 records in 1 segments\nVERIFIED\n";

#[test]
fn verify_packs_the_store_and_believes_only_an_agreeing_verified() {
    let bins = temp_dir("ok");
    let data = data_dir("ok");
    let c = cfg(
        &data,
        fake_node(&bins, 0, 0),
        fake_verify(&bins, VERIFIED_OUT, 0),
        None,
    );
    match verify_in(&Slots::new(2), &c, Duration::from_secs(20)) {
        VerifyState::Verified { root, findings, .. } => {
            assert_eq!(root.len(), 96);
            assert_eq!(
                findings,
                vec!["[note] root-signature: \"es384\" by key 0a1b2c"]
            );
        }
        other => panic!("expected verified, got {other:?}"),
    }
}

#[test]
fn the_two_steps_run_with_exact_arguments_and_path_alone() {
    let bins = temp_dir("argv");
    let data = data_dir("argv");
    let c = cfg(
        &data,
        fake_node(&bins, 0, 0),
        fake_verify(&bins, VERIFIED_OUT, 0),
        Some("/w/attest"),
    );
    let _ = verify_in(&Slots::new(2), &c, Duration::from_secs(20));
    let node_args = std::fs::read_to_string(bins.join("node-args.txt")).expect("node args");
    let pack = std::fs::read_to_string(bins.join("pack.path")).expect("pack path");
    let pack = pack.trim();
    assert_eq!(
        node_args,
        format!("read\n--data\n{}\n--pack\n{pack}\n", data.display())
    );
    let verify_args = std::fs::read_to_string(bins.join("verify-args.txt")).expect("verify args");
    assert_eq!(verify_args.trim(), pack);
    let node_env = std::fs::read_to_string(bins.join("node-env.txt")).expect("node env");
    let verify_env = std::fs::read_to_string(bins.join("verify-env.txt")).expect("verify env");
    let names = |env: &str| -> Vec<String> {
        env.lines()
            .filter_map(|l| l.split_once('=').map(|(k, _)| k.to_string()))
            .filter(|k| k != "PWD" && k != "SHLVL" && k != "_" && k != "OLDPWD")
            .collect()
    };
    assert_eq!(
        names(&node_env),
        vec!["PATH".to_string(), WITNESS_ENV.to_string()]
    );
    assert_eq!(names(&verify_env), vec!["PATH".to_string()]);
    // The scratch the pack was written in is gone.
    assert!(!Path::new(pack).exists());
}

#[test]
fn a_verified_line_that_is_not_the_last_is_not_a_verdict() {
    let bins = temp_dir("forged");
    let data = data_dir("forged");
    let out = "VERIFIED\n[note] something printed after\n";
    let c = cfg(
        &data,
        fake_node(&bins, 0, 0),
        fake_verify(&bins, out, 0),
        None,
    );
    assert!(matches!(
        verify_in(&Slots::new(2), &c, Duration::from_secs(20)),
        VerifyState::NotChecked { .. }
    ));
}

#[test]
fn not_verified_is_said_with_its_findings() {
    let bins = temp_dir("bad");
    let data = data_dir("bad");
    let out = "[fail] history: segment 1 does not chain\nNOT VERIFIED\n";
    let c = cfg(
        &data,
        fake_node(&bins, 0, 0),
        fake_verify(&bins, out, 1),
        None,
    );
    match verify_in(&Slots::new(2), &c, Duration::from_secs(20)) {
        VerifyState::NotVerified { findings, .. } => assert_eq!(findings.len(), 1),
        other => panic!("expected not verified, got {other:?}"),
    }
}

#[test]
fn an_exit_code_and_a_last_line_that_disagree_could_not_be_checked() {
    let bins = temp_dir("disagree");
    let data = data_dir("disagree");
    let c = cfg(
        &data,
        fake_node(&bins, 0, 0),
        fake_verify(&bins, "NOT VERIFIED\n", 0),
        None,
    );
    assert!(matches!(
        verify_in(&Slots::new(2), &c, Duration::from_secs(20)),
        VerifyState::NotChecked { .. }
    ));
    let bins = temp_dir("exit2");
    let c = cfg(
        &data,
        fake_node(&bins, 0, 0),
        fake_verify(&bins, "BROKEN: x\n", 2),
        None,
    );
    assert!(matches!(
        verify_in(&Slots::new(2), &c, Duration::from_secs(20)),
        VerifyState::NotChecked { .. }
    ));
}

#[test]
fn a_pack_step_that_fails_or_overruns_could_not_be_checked() {
    let bins = temp_dir("packfail");
    let data = data_dir("packfail");
    let c = cfg(
        &data,
        fake_node(&bins, 3, 0),
        fake_verify(&bins, VERIFIED_OUT, 0),
        None,
    );
    assert!(matches!(
        verify_in(&Slots::new(2), &c, Duration::from_secs(20)),
        VerifyState::NotChecked { .. }
    ));
    assert!(
        !bins.join("verify-args.txt").exists(),
        "verify ran after a failed pack"
    );

    let bins = temp_dir("slow");
    let c = cfg(
        &data,
        fake_node(&bins, 0, 5),
        fake_verify(&bins, VERIFIED_OUT, 0),
        None,
    );
    let started = std::time::Instant::now();
    match verify_in(&Slots::new(2), &c, Duration::from_secs(1)) {
        VerifyState::NotChecked { reason, .. } => assert!(reason.contains("Timeout"), "{reason}"),
        other => panic!("expected not checked, got {other:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(4));
}

#[test]
fn a_busy_box_or_no_data_dir_runs_nothing() {
    let bins = temp_dir("busy");
    let data = data_dir("busy");
    let slots = Slots::new(1);
    let _held = slots.try_acquire().expect("one slot");
    let c = cfg(
        &data,
        fake_node(&bins, 0, 0),
        fake_verify(&bins, VERIFIED_OUT, 0),
        None,
    );
    match verify_in(&slots, &c, Duration::from_secs(20)) {
        VerifyState::NotChecked { reason, .. } => assert!(reason.contains("slots"), "{reason}"),
        other => panic!("expected not checked, got {other:?}"),
    }
    assert!(!bins.join("node-args.txt").exists());

    let bins = temp_dir("unset");
    let c = RecordConfig {
        data_dir: Ok(None),
        ..cfg(
            &data,
            fake_node(&bins, 0, 0),
            fake_verify(&bins, VERIFIED_OUT, 0),
            None,
        )
    };
    assert!(matches!(
        verify_in(&Slots::new(2), &c, Duration::from_secs(20)),
        VerifyState::NotChecked { .. }
    ));
    assert!(!bins.join("node-args.txt").exists());
}

#[test]
fn a_record_sealed_further_while_it_was_packed_is_no_verdict() {
    let bins = temp_dir("moved");
    let data = data_dir("moved");
    let spare = bins.join("spare.mf");
    std::fs::write(&spare, encode_manifest(&manifest(2))).expect("spare");
    let node = fake_node_with(&bins, 0, 0, Some(&spare), "");
    let c = cfg(&data, node, fake_verify(&bins, VERIFIED_OUT, 0), None);
    match verify_in(&Slots::new(2), &c, Duration::from_secs(20)) {
        VerifyState::NotChecked { reason, .. } => {
            assert!(reason.contains("sealed further"), "{reason}")
        }
        other => panic!("expected not checked, got {other:?}"),
    }
}

#[test]
fn a_store_the_node_refuses_to_pack_is_refused_with_its_words() {
    let bins = temp_dir("refused");
    let data = data_dir("refused");
    let node = fake_node_with(
        &bins,
        2,
        0,
        None,
        "s0-000001.trlx does not reproduce its manifest",
    );
    let c = cfg(&data, node, fake_verify(&bins, VERIFIED_OUT, 0), None);
    match verify_in(&Slots::new(2), &c, Duration::from_secs(20)) {
        VerifyState::Refused { reason, root, .. } => {
            assert!(reason.contains("does not reproduce"), "{reason}");
            assert_eq!(root.len(), 96);
        }
        other => panic!("expected refused, got {other:?}"),
    }
    assert!(
        !bins.join("verify-args.txt").exists(),
        "verify ran after a refusal"
    );
}
