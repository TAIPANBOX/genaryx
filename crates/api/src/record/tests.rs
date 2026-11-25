//! Invariant 54: the seal read from a data directory built with Trailryx's own
//! `encode_manifest`, the signing files, and the strict reading of a verdict.

use super::*;
use trailryx_index::{Dimension, SegmentManifest};
use trailryx_record::{Algorithms, Hash, Timestamp};
use trailryx_store::evidence::encode_manifest;

fn scratch(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-record-{tag}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn manifest(segment: u64, records: u64) -> SegmentManifest {
    SegmentManifest {
        format_version: 1,
        segment: SegmentId(segment),
        shard: ShardIx(0),
        records,
        history_root: Hash([segment as u8; 48]),
        index_roots: Dimension::ALL
            .iter()
            .map(|d| (*d, Hash([9u8; 48])))
            .collect(),
        chain_before: Hash([segment.wrapping_sub(1) as u8; 48]),
        chain_after: Hash([segment as u8; 48]),
        first_recorded_at: Timestamp(1_763_900_000_000_000_000 + segment * 1_000_000_000),
        last_recorded_at: Timestamp(
            1_763_900_000_000_000_000 + segment * 1_000_000_000 + 500_000_000,
        ),
        algorithms: Algorithms::default(),
    }
}

fn write_manifest(dir: &Path, m: &SegmentManifest) {
    std::fs::write(
        dir.join(format!("s0-{:06}.mf", m.segment.0)),
        encode_manifest(m),
    )
    .expect("write mf");
}

fn store(tag: &str, n: u64) -> PathBuf {
    let dir = scratch(tag);
    for s in 1..=n {
        write_manifest(&dir, &manifest(s, 10 + s));
    }
    dir
}

fn expected_root(n: u64) -> String {
    let mut tree = ShardTree::new(ShardIx(0));
    for s in 1..=n {
        tree.push(manifest(s, 10 + s));
    }
    StoreTree::from_shards(&[tree]).root().to_hex()
}

fn cfg(dir: &Path) -> RecordConfig {
    RecordConfig {
        data_dir: Ok(Some(dir.to_path_buf())),
        witness_dir: None,
        node_bin: PathBuf::from("trailryx-node"),
        verify_bin: PathBuf::from("trailryx-verify"),
    }
}

/// An uncompressed P-384 point: 0x04 then 96 bytes, 97 in all.
fn public_key() -> String {
    format!("04{}", "11".repeat(96))
}

fn signed_text(at: u64) -> String {
    format!(
        "generated_at={at}\nalgorithm=es384\npublic_key={}\nsignature={}\n",
        public_key(),
        "ab".repeat(96)
    )
}

#[test]
fn the_data_dir_setting_is_an_absolute_path_with_no_parent_steps() {
    assert_eq!(DATA_ENV, "GENARYX_TRAILRYX_DATA_DIR");
    assert_eq!(data_dir_from(" "), Ok(None));
    assert_eq!(
        data_dir_from("/var/lib/trailryx\n"),
        Ok(Some(PathBuf::from("/var/lib/trailryx")))
    );
    assert!(data_dir_from("trailryx-data").is_err());
    assert!(data_dir_from("/var/lib/../etc").is_err());
}

#[test]
fn a_box_with_no_data_dir_named_says_so_and_names_the_setting() {
    let c = RecordConfig {
        data_dir: Ok(None),
        ..cfg(Path::new("/x"))
    };
    assert_eq!(
        seal_state(&c),
        SealState::NotConfigured {
            variable: DATA_ENV.to_string()
        }
    );
}

#[test]
fn a_named_directory_that_is_not_there_is_unreadable_not_empty() {
    let missing = scratch("missing").join("nope");
    assert!(matches!(
        seal_state(&cfg(&missing)),
        SealState::Unreadable { .. }
    ));
}

#[test]
fn an_empty_directory_is_nothing_sealed() {
    let dir = scratch("empty");
    assert!(matches!(
        seal_state(&cfg(&dir)),
        SealState::NothingSealed { .. }
    ));
}

#[test]
fn the_root_shown_is_the_root_trailryx_computes_for_one_two_and_fifty_segments() {
    for n in [1u64, 2, 50] {
        let dir = store(&format!("root{n}"), n);
        match seal_state(&cfg(&dir)) {
            SealState::Sealed {
                root,
                segments,
                records,
                last,
                ..
            } => {
                assert_eq!(root, expected_root(n), "{n} segments");
                assert_eq!(root.len(), 96);
                assert_eq!(segments as u64, n);
                assert_eq!(records, (1..=n).map(|s| 10 + s).sum::<u64>());
                assert_eq!(last.segment, n);
                assert_eq!(last.records, 10 + n);
                assert_eq!(last.first_recorded_at, 1_763_900_000 + n);
            }
            other => panic!("expected sealed, got {other:?}"),
        }
    }
}

#[cfg(unix)]
#[test]
fn a_linked_manifest_or_a_directory_named_like_one_makes_the_store_unreadable() {
    let dir = store("link", 1);
    let elsewhere = scratch("link-target").join("x.mf");
    std::fs::write(&elsewhere, encode_manifest(&manifest(2, 1))).expect("target");
    std::os::unix::fs::symlink(&elsewhere, dir.join("s0-000002.mf")).expect("symlink");
    assert!(matches!(
        seal_state(&cfg(&dir)),
        SealState::Unreadable { .. }
    ));

    let dir = store("dirmf", 1);
    std::fs::create_dir(dir.join("s0-000002.mf")).expect("dir");
    assert!(matches!(
        seal_state(&cfg(&dir)),
        SealState::Unreadable { .. }
    ));
}

#[test]
fn a_manifest_under_another_segments_name_makes_the_store_unreadable() {
    let dir = store("moved", 2);
    std::fs::rename(dir.join("s0-000002.mf"), dir.join("s0-000007.mf")).expect("rename");
    match seal_state(&cfg(&dir)) {
        SealState::Unreadable { reason, .. } => {
            assert!(reason.contains("another segment"), "{reason}")
        }
        other => panic!("expected unreadable, got {other:?}"),
    }
}

#[test]
fn an_oversized_or_undecodable_manifest_makes_the_store_unreadable() {
    let dir = store("big", 1);
    std::fs::write(
        dir.join("s0-000002.mf"),
        vec![0u8; (MAX_MANIFEST_BYTES + 1) as usize],
    )
    .expect("big");
    assert!(matches!(
        seal_state(&cfg(&dir)),
        SealState::Unreadable { .. }
    ));
    let dir = store("junk", 1);
    std::fs::write(dir.join("s0-000002.mf"), b"not a manifest").expect("junk");
    assert!(matches!(
        seal_state(&cfg(&dir)),
        SealState::Unreadable { .. }
    ));
}

#[test]
fn other_shards_are_counted_and_not_read() {
    let dir = store("shards", 1);
    std::fs::write(dir.join("s1-000001.mf"), b"whatever").expect("other shard");
    std::fs::write(dir.join("s0-000001.trlx"), b"journal").expect("journal");
    match seal_state(&cfg(&dir)) {
        SealState::Sealed {
            other_shards,
            segments,
            ..
        } => {
            assert_eq!((other_shards, segments), (1, 1));
        }
        other => panic!("expected sealed, got {other:?}"),
    }
}

fn signing_file(dir: &Path, sub: &str, root: &str, body: &str) {
    let d = dir.join("signing").join(sub);
    std::fs::create_dir_all(&d).expect("signing dir");
    std::fs::write(d.join(root), body).expect("signing file");
}

#[test]
fn a_signature_on_file_for_the_current_root_reads_as_signed_with_its_key_and_time() {
    let dir = store("signed", 2);
    signing_file(
        &dir,
        "signed",
        &expected_root(2),
        &signed_text(1_763_950_000_123_456_789),
    );
    match seal_state(&cfg(&dir)) {
        SealState::Sealed {
            signature: Signature::Signed { key, algorithm, at },
            ..
        } => {
            assert_eq!(key.len(), 16);
            assert_eq!(algorithm, "es384");
            assert_eq!(at, 1_763_950_000);
        }
        other => panic!("expected signed, got {other:?}"),
    }
}

#[test]
fn a_root_waiting_for_its_signature_reads_as_queued_never_as_signed() {
    let dir = store("pending", 1);
    signing_file(
        &dir,
        "pending",
        &expected_root(1),
        "tenant=acme\ngenerated_at=1763950000000000000\n",
    );
    match seal_state(&cfg(&dir)) {
        SealState::Sealed { signature, .. } => assert_eq!(
            signature,
            Signature::Queued {
                since: 1_763_950_000
            }
        ),
        other => panic!("expected sealed, got {other:?}"),
    }
}

#[test]
fn a_signature_for_another_root_does_not_sign_this_one() {
    let dir = store("otherroot", 2);
    signing_file(&dir, "signed", &expected_root(1), &signed_text(1));
    match seal_state(&cfg(&dir)) {
        SealState::Sealed { signature, .. } => assert_eq!(signature, Signature::None),
        other => panic!("expected sealed, got {other:?}"),
    }
}

#[test]
fn a_signing_file_this_console_cannot_read_is_unreadable_never_unsigned() {
    let dir = store("badsig", 1);
    let bad = signed_text(1).replace("algorithm=es384", "algorithm=es256");
    signing_file(&dir, "signed", &expected_root(1), &bad);
    match seal_state(&cfg(&dir)) {
        SealState::Sealed {
            signature: Signature::Unreadable { .. },
            ..
        } => {}
        other => panic!("expected an unreadable signature, got {other:?}"),
    }
    for bad in [
        signed_text(1).replace("public_key=04", "public_key=02"),
        signed_text(1).replace(&"ab".repeat(96), &"ab".repeat(95)),
        format!("{}extra=1\n", signed_text(1)),
        signed_text(1).replace("\nalgorithm", "\n\nalgorithm"),
        signed_text(1).replace("generated_at=1", "generated_at=x"),
        signed_text(1).to_uppercase(),
    ] {
        assert!(parse_signed(&bad).is_err(), "{bad}");
    }
}

#[test]
fn a_verdict_needs_the_exit_code_and_the_last_line_to_agree() {
    assert_eq!(
        judge_verify(Some(0), "[note] x: y\n1 records in 1 segments\nVERIFIED\n"),
        Ok(true)
    );
    assert_eq!(
        judge_verify(Some(1), "[fail] x: y\nNOT VERIFIED\n"),
        Ok(false)
    );
    assert!(judge_verify(Some(0), "NOT VERIFIED\n").is_err());
    assert!(judge_verify(Some(1), "VERIFIED\n").is_err());
    assert!(judge_verify(Some(2), "BROKEN: short\n").is_err());
    assert_eq!(
        judge_verify(Some(1), "BROKEN: the history does not chain\n"),
        Ok(false)
    );
    assert!(judge_verify(None, "VERIFIED\n").is_err());
    assert!(judge_verify(Some(0), "VERIFIED\n[note] a line after\n").is_err());
}

#[test]
fn findings_are_bounded_and_only_the_bracketed_lines() {
    let mut out = String::new();
    for i in 0..300 {
        out.push_str(&format!("[note] f{i}: {}\n", "x".repeat(1000)));
    }
    out.push_str("VERIFIED\n");
    let f = findings_of(&out);
    assert_eq!(f.len(), MAX_FINDINGS);
    assert!(f[0].chars().count() <= MAX_FINDING_CHARS + 3);
    assert!(f.iter().all(|l| l.starts_with('[')));
}

#[test]
fn two_hundred_seeds_of_mutated_manifests_never_panic_and_never_read_under_another_segment() {
    let good = encode_manifest(&manifest(1, 11));
    for seed in 0..200u64 {
        let mut x = seed.wrapping_add(5);
        let mut next = || {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 33) as usize
        };
        let mut bytes = good.clone();
        match next() % 3 {
            0 => bytes.truncate(next() % good.len()),
            1 => {
                let i = next() % bytes.len();
                bytes[i] ^= 1 + (next() % 255) as u8;
            }
            _ => bytes.extend(std::iter::repeat_n(0u8, 1 + next() % 8)),
        }
        let dir = scratch(&format!("sweep{seed}"));
        std::fs::write(dir.join("s0-000001.mf"), &bytes).expect("write");
        match seal_state(&cfg(&dir)) {
            SealState::Unreadable { .. } => {}
            SealState::Sealed { segments, last, .. } => {
                // A flip that still decodes names the same segment, or the
                // name check would have refused it.
                assert_eq!((segments, last.segment), (1, 1), "seed {seed}");
            }
            other => panic!("seed {seed}: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn two_hundred_seeds_of_hostile_signing_files_never_read_as_signed() {
    let alphabet: Vec<char> =
        "=\n0123456789abcdefABCDEF generated_atalgorithmes384public_keysignature"
            .chars()
            .collect();
    for seed in 0..200u64 {
        let mut x = seed.wrapping_add(11);
        let mut next = || {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 33) as usize
        };
        let n = next() % 400;
        let text: String = (0..n).map(|_| alphabet[next() % alphabet.len()]).collect();
        assert!(
            !matches!(parse_signed(&text), Ok(Signature::Signed { .. })),
            "seed {seed}: {text:?}"
        );
        let _ = parse_pending(&text);
    }
}

#[test]
fn a_verdict_is_stale_when_the_root_shown_is_another_or_none() {
    let verified = VerifyState::Verified {
        root: expected_root(1),
        at: 10,
        millis: 1,
        findings: vec![],
    };
    let dir = store("stale1", 1);
    assert!(!panel(seal_state(&cfg(&dir)), verified.clone()).verify_stale);
    let dir = store("stale2", 2);
    assert!(panel(seal_state(&cfg(&dir)), verified.clone()).verify_stale);
    let gone = scratch("stale-gone").join("nope");
    assert!(panel(seal_state(&cfg(&gone)), verified.clone()).verify_stale);
    assert!(!panel(seal_state(&cfg(&gone)), VerifyState::NotRun).verify_stale);
}

#[test]
fn a_check_that_made_no_verdict_or_an_older_one_does_not_replace_the_kept_verdict() {
    let red = VerifyState::NotVerified {
        root: "r".into(),
        at: 20,
        millis: 1,
        findings: vec![],
    };
    let mut kept = VerifyState::NotRun;
    keep_verdict(&mut kept, &red);
    assert_eq!(kept, red);
    keep_verdict(
        &mut kept,
        &VerifyState::NotChecked {
            at: 30,
            reason: "busy".into(),
        },
    );
    assert_eq!(kept, red);
    let older = VerifyState::Verified {
        root: "r".into(),
        at: 10,
        millis: 1,
        findings: vec![],
    };
    keep_verdict(&mut kept, &older);
    assert_eq!(kept, red);
    let newer = VerifyState::Verified {
        root: "r".into(),
        at: 40,
        millis: 1,
        findings: vec![],
    };
    keep_verdict(&mut kept, &newer);
    assert_eq!(kept, newer);
}
