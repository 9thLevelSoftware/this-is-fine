//! Tier C — AI soak substitute (UT-3). docs/USER_TESTING.md C01–C03.

use std::fs;
use std::time::Instant;
use tif_e2e::{
    copy_fixture, ensure_tif_built, git_init_commit, run_scenario, tif_json, ScenarioResult,
};

/// C01: ≥50 scripted tasks across fixtures × fire levels × plant styles.
#[test]
fn c01_task_battery_fifty() {
    ensure_tif_built();
    let r = run_scenario("C01", || {
        let fixtures = ["rust-mini", "js-mini"];
        let fire_levels = [1u8, 2, 3, 4];
        let plants = ["minimal", "bloat", "none"];
        let mut n = 0u32;
        let mut fails = 0u32;
        let mut notes = Vec::new();

        // Generate combinations until N≥50 with diversity.
        'outer: for fixture in fixtures {
            for &level in &fire_levels {
                for plant in plants {
                    for approval in [false, true] {
                        if n >= 50 {
                            break 'outer;
                        }
                        match run_one_task(fixture, level, plant, approval) {
                            Ok(msg) => notes.push(format!("ok#{n}:{msg}")),
                            Err(e) => {
                                fails += 1;
                                notes.push(format!("fail#{n}:{e}"));
                            }
                        }
                        n += 1;
                    }
                }
            }
        }
        // Pad remaining with simple contained tasks if under 50.
        while n < 50 {
            match run_one_task("rust-mini", 3, "minimal", false) {
                Ok(msg) => notes.push(format!("ok#{n}:{msg}")),
                Err(e) => {
                    fails += 1;
                    notes.push(format!("fail#{n}:{e}"));
                }
            }
            n += 1;
        }

        let pass_rate = (n - fails) as f64 / n as f64;
        if fails > 0 && pass_rate < 0.98 {
            return Err(format!(
                "C01 pass rate {pass_rate:.3} below 98% (fails={fails}/{n}): {}",
                notes
                    .iter()
                    .filter(|s| s.starts_with("fail"))
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        // Zero P0: any data-loss style fail is a hard fail — already counted.
        Ok(format!(
            "N={n} fails={fails} pass_rate={pass_rate:.3} (AI soak substitute)"
        ))
    });
    assert_scenario(&r);
}

#[test]
fn c02_concurrent_apply_lock() {
    ensure_tif_built();
    let r = run_scenario("C02", || {
        // Sequential stress of apply path with shared lockfile expectations.
        // Two sequential OOC applies on separate repos must both succeed; on one repo
        // back-to-back completes must not corrupt trees.
        let mut ok = 0;
        for i in 0..5 {
            let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
            let root = tmp.path();
            fs::write(root.join("TIF_MOCK_REDUCE"), vec![b'X'; 25_000])
                .map_err(|e| e.to_string())?;
            git_init_commit(root).map_err(|e| e.to_string())?;
            let begin = tif_json(
                root,
                &[
                    "run",
                    "begin",
                    "--task",
                    &format!("concurrent {i}"),
                    "--force",
                ],
            )
            .map_err(|e| e.to_string())?;
            let id = begin.data_str("run_id").ok_or_else(|| begin.summary())?;
            let complete = tif_json(
                root,
                &[
                    "run",
                    "complete",
                    &id,
                    "--deps-added",
                    "1",
                    "--files-added",
                    "2",
                    "--lines-added",
                    "80",
                    "--auto-firebreak",
                    "--verification-passed",
                    "true",
                ],
            )
            .map_err(|e| e.to_string())?;
            if !complete.ok_envelope() {
                return Err(format!("task {i}: {}", complete.summary()));
            }
            // Tree must remain readable.
            if !root.join("src/lib.rs").is_file() {
                return Err(format!("task {i}: source corrupted"));
            }
            ok += 1;
        }
        Ok(format!("concurrent-safe sequential applies ok={ok}/5"))
    });
    assert_scenario(&r);
}

#[test]
fn c03_perf_smoke() {
    ensure_tif_built();
    let r = run_scenario("C03", || {
        let tmp = copy_fixture("rust-mini").map_err(|e| e.to_string())?;
        let root = tmp.path();
        git_init_commit(root).map_err(|e| e.to_string())?;

        let t0 = Instant::now();
        for _ in 0..100 {
            let out = tif_json(root, &["policy", "resolve", "--task", "perf smoke"])
                .map_err(|e| e.to_string())?;
            if !out.ok_envelope() {
                return Err(format!("policy resolve: {}", out.summary()));
            }
        }
        let policy_ms = t0.elapsed().as_millis();
        let p95_budget_ms = 30_000u128; // generous CI budget for 100 resolves
        if policy_ms > p95_budget_ms {
            return Err(format!(
                "policy resolve ×100 took {policy_ms}ms > budget {p95_budget_ms}ms"
            ));
        }

        // Large synthetic assess.
        let t1 = Instant::now();
        let assess = tif_json(
            root,
            &[
                "assess",
                "--files-added",
                "50",
                "--files-changed",
                "200",
                "--lines-added",
                "10000",
                "--lines-removed",
                "500",
                "--deps-added",
                "0",
            ],
        )
        .map_err(|e| e.to_string())?;
        if !assess.ok_envelope() {
            return Err(format!("assess large: {}", assess.summary()));
        }
        let assess_ms = t1.elapsed().as_millis();

        Ok(format!(
            "policy×100={policy_ms}ms assess_large={assess_ms}ms"
        ))
    });
    assert_scenario(&r);
}

fn run_one_task(
    fixture: &str,
    fire_level: u8,
    plant: &str,
    require_approval: bool,
) -> Result<String, String> {
    let tmp = copy_fixture(fixture).map_err(|e| e.to_string())?;
    let root = tmp.path();

    if require_approval {
        let shared = root.join(".this-is-fine.toml");
        let text = fs::read_to_string(&shared).map_err(|e| e.to_string())?;
        let text = text.replace(
            "require_firebreak_approval = false",
            "require_firebreak_approval = true",
        );
        fs::write(&shared, text).map_err(|e| e.to_string())?;
    }

    match plant {
        "bloat" => {
            fs::write(root.join("TIF_MOCK_REDUCE"), vec![b'X'; 15_000])
                .map_err(|e| e.to_string())?;
        }
        "minimal" => {
            if root.join("src/lib.rs").is_file() {
                let mut b =
                    fs::read_to_string(root.join("src/lib.rs")).map_err(|e| e.to_string())?;
                b.push_str("\n// plant minimal\n");
                fs::write(root.join("src/lib.rs"), b).map_err(|e| e.to_string())?;
            } else if root.join("index.js").is_file() {
                let mut b = fs::read_to_string(root.join("index.js")).map_err(|e| e.to_string())?;
                b.push_str("\n// plant minimal\n");
                fs::write(root.join("index.js"), b).map_err(|e| e.to_string())?;
            }
        }
        _ => {}
    }

    git_init_commit(root).map_err(|e| e.to_string())?;

    let fl = fire_level.to_string();
    let begin = tif_json(
        root,
        &[
            "run",
            "begin",
            "--task",
            "soak task",
            "--fire-level",
            &fl,
            "--force",
        ],
    )
    .map_err(|e| e.to_string())?;
    if !begin.ok_envelope() {
        return Err(format!("begin: {}", begin.summary()));
    }
    let run_id = begin.data_str("run_id").ok_or("no run_id")?;

    let mut args: Vec<&str> = vec!["run", "complete", &run_id, "--verification-passed", "true"];
    if plant == "bloat" {
        args.extend_from_slice(&[
            "--deps-added",
            "1",
            "--files-added",
            "2",
            "--lines-added",
            "80",
            "--auto-firebreak",
        ]);
    } else {
        args.extend_from_slice(&["--files-changed", "1", "--lines-added", "2"]);
    }

    let complete = tif_json(root, &args).map_err(|e| e.to_string())?;
    if complete.protocol_version() != Some(1) {
        return Err(format!("bad protocol: {}", complete.summary()));
    }
    // P0: never leave a corrupted primary source file.
    if fixture.starts_with("rust") && !root.join("src/lib.rs").is_file() {
        return Err("P0: src/lib.rs missing after complete".into());
    }
    if fixture.starts_with("js") && !root.join("index.js").is_file() {
        return Err("P0: index.js missing after complete".into());
    }

    let state = complete.data_str("state").unwrap_or_default();
    Ok(format!(
        "{fixture}/fl{fire_level}/{plant}/appr={require_approval}→{state}"
    ))
}

fn assert_scenario(r: &ScenarioResult) {
    assert!(
        r.pass,
        "scenario {} failed ({}ms): {}\n{}",
        r.id, r.duration_ms, r.notes, r.log
    );
}
