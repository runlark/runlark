use runlark_domain::RunId;
use runlark_wasm::{
    DigestPinnedRegistry, PackageKind, PackageManifest, ResourceLimits, StageTransition,
    ValidationInput, ValidationVerdict, VerifiedPackage, WasmError, WasmValidatorHost, WitContract,
    WorkflowBudget, WorkflowDefinition, WorkflowStage, compile_wat_component, sha256_hex,
};
use std::path::Path;
use tempfile::tempdir;

fn solo_default_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("workflows")
        .join("solo-default")
}

#[test]
fn first_party_solo_default_package_verifies_and_matches_wat_source() {
    let contract = WitContract::canonical().expect("canonical WIT parses");
    let pkg_dir = solo_default_dir();
    let pkg = VerifiedPackage::load_from_path(&pkg_dir, &contract)
        .expect("solo-default package must verify cleanly");

    assert_eq!(pkg.manifest.name, "solo-default");
    assert_eq!(pkg.manifest.version, "0.1.0");
    assert_eq!(pkg.manifest.kind, PackageKind::Workflow);
    assert_eq!(pkg.manifest.capabilities, vec!["log"]);

    let workflow = pkg.workflow.as_ref().expect("workflow definition present");
    assert_eq!(
        workflow.stages,
        vec!["clarify", "plan", "implement", "validate", "human-accept"]
    );
    assert!(workflow.budget.human_acceptance_required);

    // Ensure committed validator.wasm is reproducibly assembled from validator.wat + WIT.
    let wat_src = std::fs::read_to_string(pkg_dir.join("validator.wat")).expect("read wat");
    let rebuilt = compile_wat_component(&wat_src, &contract).expect("compile wat");
    assert_eq!(
        pkg.component_bytes, rebuilt,
        "committed validator.wasm must match validator.wat + wit/runlark-workflow.wit"
    );

    let host = WasmValidatorHost::new().expect("engine initializes");
    let report = host
        .validate_run(
            &pkg,
            &ValidationInput {
                task_id: "task-solo-1".to_string(),
                run_id: "run-solo-1".to_string(),
                stage: WorkflowStage::Validate,
                changed_files: vec!["src/main.rs".to_string()],
                checks: vec![runlark_wasm::CheckEvidence {
                    name: "just check".to_string(),
                    passed: true,
                    exit_code: 0,
                }],
                diff_bytes: 2048,
                repair_attempt: 0,
            },
        )
        .expect("solo-default validator executes cleanly");
    assert_eq!(report.verdict, ValidationVerdict::Pass);
}

#[test]
fn tampered_artifacts_and_invalid_manifests_are_rejected() {
    let contract = WitContract::canonical().expect("canonical WIT parses");
    let dir = tempdir().expect("tempdir");
    let pkg_dir = solo_default_dir();

    for file in ["manifest.json", "workflow.json", "validator.wasm"] {
        std::fs::copy(pkg_dir.join(file), dir.path().join(file)).expect("copy fixture");
    }

    // Tamper validator.wasm by 1 byte -> DigestMismatch.
    let mut wasm_bytes = std::fs::read(dir.path().join("validator.wasm")).expect("read wasm");
    let last_idx = wasm_bytes.len() - 1;
    wasm_bytes[last_idx] ^= 0x01;
    std::fs::write(dir.path().join("validator.wasm"), wasm_bytes).expect("write tampered wasm");

    let err = VerifiedPackage::load_from_path(dir.path(), &contract)
        .expect_err("tampered validator.wasm must fail digest check");
    assert!(matches!(err, WasmError::DigestMismatch { .. }));

    // Restore validator.wasm, tamper workflow.json -> DigestMismatch.
    std::fs::copy(
        pkg_dir.join("validator.wasm"),
        dir.path().join("validator.wasm"),
    )
    .expect("restore wasm");
    let mut wf = std::fs::read_to_string(dir.path().join("workflow.json")).expect("read wf");
    wf.push(' ');
    std::fs::write(dir.path().join("workflow.json"), wf).expect("write tampered wf");

    let err = VerifiedPackage::load_from_path(dir.path(), &contract)
        .expect_err("tampered workflow.json must fail digest check");
    assert!(matches!(err, WasmError::DigestMismatch { .. }));

    // Manifest validation rejections: path traversal, bad semver, disallowed capability.
    let mut bad_manifest: PackageManifest = serde_json::from_slice(
        &std::fs::read(pkg_dir.join("manifest.json")).expect("read manifest"),
    )
    .expect("parse manifest");
    bad_manifest.entrypoint = "../escape.wasm".to_string();
    assert!(matches!(
        bad_manifest.validate(&contract),
        Err(WasmError::InvalidManifest(_))
    ));

    bad_manifest.entrypoint = "validator.wasm".to_string();
    bad_manifest.capabilities = vec!["fs:write".to_string()];
    assert!(matches!(
        bad_manifest.validate(&contract),
        Err(WasmError::InvalidManifest(_))
    ));

    bad_manifest.capabilities = vec!["log".to_string()];
    bad_manifest.version = "v1".to_string();
    assert!(matches!(
        bad_manifest.validate(&contract),
        Err(WasmError::InvalidManifest(_))
    ));

    // Workflow validation rejection: human_acceptance_required=false must be rejected.
    let bad_wf = WorkflowDefinition {
        schema_version: 1,
        workflow_id: "bad".to_string(),
        stages: vec!["implement".to_string(), "human-accept".to_string()],
        transitions: vec![StageTransition {
            from_stage: "implement".to_string(),
            to_stage: "human-accept".to_string(),
            max_attempts: 1,
        }],
        validator_stage: "implement".to_string(),
        budget: WorkflowBudget {
            max_repair_attempts: 1,
            max_wall_seconds: 60,
            max_spend_cents: 100,
            human_acceptance_required: false,
        },
    };
    assert!(matches!(
        bad_wf.validate(&contract),
        Err(WasmError::InvalidWorkflow(_))
    ));
}

#[test]
fn digest_pinned_registry_retains_older_digests_for_inflight_runs() {
    let contract = WitContract::canonical().expect("canonical WIT parses");
    let cas_dir = tempdir().expect("cas dir");
    let mut registry = DigestPinnedRegistry::with_cas_dir(cas_dir.path()).expect("init CAS");

    let pkg_v1 =
        VerifiedPackage::load_from_path(&solo_default_dir(), &contract).expect("load v0.1.0");
    let pin_v1 = registry
        .install_package(pkg_v1.clone())
        .expect("install v0.1.0");

    let run_1 = RunId::from("run-inflight-1");
    registry
        .pin_run(&run_1, "solo-default")
        .expect("pin run_1 to v0.1.0");

    // Build and install v0.2.0 with updated version and limits.
    let mut pkg_v2 = pkg_v1.clone();
    pkg_v2.manifest.version = "0.2.0".to_string();
    pkg_v2.manifest.limits = ResourceLimits {
        max_fuel: 2_000_000,
        ..ResourceLimits::default()
    };
    pkg_v2.package_digest = pkg_v2.manifest.canonical_digest();
    let pin_v2 = registry.install_package(pkg_v2).expect("install v0.2.0");
    assert_ne!(pin_v1.package_digest, pin_v2.package_digest);

    let run_2 = RunId::from("run-inflight-2");
    registry
        .pin_run(&run_2, "solo-default")
        .expect("pin run_2 to v0.2.0");

    // Latest points to v0.2.0, while run_1 still resolves to v0.1.0.
    assert_eq!(
        registry
            .latest_for("solo-default")
            .map(|p| p.manifest.version.as_str()),
        Some("0.2.0")
    );
    assert_eq!(
        registry
            .get_for_run(&run_1)
            .map(|p| p.manifest.version.as_str()),
        Some("0.1.0")
    );
    assert_eq!(
        registry
            .get_for_run(&run_2)
            .map(|p| p.manifest.version.as_str()),
        Some("0.2.0")
    );

    // GC must not evict v0.1.0 while run_1 is still in-flight.
    assert_eq!(registry.gc_unreferenced(), 0);
    assert!(registry.get_by_digest(&pin_v1.package_digest).is_some());

    // CAS directory can reload v0.1.0 from disk in a fresh registry instance.
    let mut fresh_registry =
        DigestPinnedRegistry::with_cas_dir(cas_dir.path()).expect("reopen CAS");
    let reloaded = fresh_registry
        .load_by_digest(&pin_v1.package_digest, &contract)
        .expect("CAS reloads pinned v0.1.0 digest");
    assert_eq!(reloaded.manifest.version, "0.1.0");
    assert_eq!(
        sha256_hex(&reloaded.component_bytes),
        pkg_v1.manifest.sha256
    );

    // Releasing run_1 allows GC to evict v0.1.0 while preserving v0.2.0 (latest + run_2).
    registry.release_run(&run_1);
    assert_eq!(registry.gc_unreferenced(), 1);
    assert!(registry.get_by_digest(&pin_v1.package_digest).is_none());
    assert!(registry.get_by_digest(&pin_v2.package_digest).is_some());
}
