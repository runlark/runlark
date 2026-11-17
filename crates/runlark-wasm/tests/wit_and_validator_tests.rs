use runlark_wasm::{
    CheckEvidence, PackageDigest, PackageKind, PackageManifest, ResourceLimits, ValidationInput,
    ValidationVerdict, VerifiedPackage, WasmError, WasmValidatorHost, WitContract, WorkflowStage,
    compile_wat_component, sha256_hex,
};
use std::path::Path;
use wit_component::{ComponentEncoder, StringEncoding, embed_component_metadata};
use wit_parser::{Resolve, UnresolvedPackageGroup};

const SOLO_VALIDATOR_WAT: &str = include_str!("../../../workflows/solo-default/validator.wat");

fn make_package_from_wat(
    wat: &str,
    capabilities: Vec<String>,
    limits: ResourceLimits,
) -> VerifiedPackage {
    let contract = WitContract::canonical().expect("canonical WIT should parse");
    let component_bytes = compile_wat_component(wat, &contract).expect("WAT should compile");
    let sha256 = sha256_hex(&component_bytes);
    let manifest = PackageManifest {
        schema_version: 1,
        name: "test-validator".to_string(),
        version: "0.1.0".to_string(),
        kind: PackageKind::Validator,
        wit_version: "runlark:workflow@0.1.0".to_string(),
        world: "workflow-validator".to_string(),
        entrypoint: "validator.wasm".to_string(),
        sha256,
        workflow_path: None,
        workflow_sha256: None,
        capabilities,
        limits,
    };
    let package_digest = manifest.canonical_digest();
    VerifiedPackage {
        manifest,
        package_digest,
        component_bytes,
        workflow: None,
        workflow_bytes: None,
    }
}

#[test]
fn canonical_wit_contract_parses_and_matches_disk() {
    let canonical = WitContract::canonical().expect("canonical WIT parses");
    let disk_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("wit")
        .join("runlark-workflow.wit");
    let from_disk = WitContract::from_file(&disk_path).expect("disk WIT parses");
    assert_eq!(canonical.info(), from_disk.info());
    assert_eq!(canonical.info().package_name, "runlark:workflow@0.1.0");
    assert!(canonical.allowed_capabilities().contains("log"));
    assert!(canonical.allowed_stages().contains("human-accept"));
}

#[test]
fn wit_parser_rejects_wrong_package_or_syntax_error() {
    let err = WitContract::parse_str("bad.wit", "package other:pkg@0.1.0; world x {}")
        .expect_err("wrong package should fail");
    assert!(matches!(err, WasmError::WitContract(_)));

    let syntax_err = WitContract::parse_str("syntax.wit", "not valid wit {{{")
        .expect_err("syntax error should fail");
    assert!(matches!(syntax_err, WasmError::WitContract(_)));
}

#[test]
fn wat_validator_executes_pass_fail_and_error_paths_in_wasmtime() {
    let pkg = make_package_from_wat(
        SOLO_VALIDATOR_WAT,
        vec!["log".to_string()],
        ResourceLimits::default(),
    );
    let host = WasmValidatorHost::new().expect("engine initializes");

    let pass_report = host
        .validate_run(
            &pkg,
            &ValidationInput {
                task_id: "task-1".to_string(),
                run_id: "run-1".to_string(),
                stage: WorkflowStage::Validate,
                changed_files: vec!["crates/runlark-wasm/src/lib.rs".to_string()],
                checks: vec![CheckEvidence {
                    name: "cargo test".to_string(),
                    passed: true,
                    exit_code: 0,
                }],
                diff_bytes: 512,
                repair_attempt: 0,
            },
        )
        .expect("pass validation should succeed");

    assert_eq!(pass_report.verdict, ValidationVerdict::Pass);
    assert!(pass_report.findings.is_empty());
    assert_eq!(pass_report.summary, "all validation checks passed");
    assert!(pass_report.fuel_consumed > 0);
    assert_eq!(pass_report.logs.len(), 1);
    assert_eq!(pass_report.logs[0].level, "info");
    assert_eq!(pass_report.logs[0].message, "validating run");

    let fail_report = host
        .validate_run(
            &pkg,
            &ValidationInput {
                task_id: "task-1".to_string(),
                run_id: "run-2".to_string(),
                stage: WorkflowStage::Validate,
                changed_files: vec![],
                checks: vec![CheckEvidence {
                    name: "cargo clippy".to_string(),
                    passed: false,
                    exit_code: 101,
                }],
                diff_bytes: 0,
                repair_attempt: 1,
            },
        )
        .expect("fail validation should return Fail verdict");

    assert_eq!(fail_report.verdict, ValidationVerdict::Fail);
    assert_eq!(fail_report.summary, "validation failed");
    assert_eq!(
        fail_report.findings,
        vec![
            "no changed files recorded",
            "empty diff (0 bytes)",
            "one or more checks failed",
        ]
    );

    let err = host
        .validate_run(
            &pkg,
            &ValidationInput {
                task_id: String::new(),
                run_id: "run-3".to_string(),
                stage: WorkflowStage::Validate,
                changed_files: vec!["a.rs".to_string()],
                checks: vec![CheckEvidence {
                    name: "lint".to_string(),
                    passed: true,
                    exit_code: 0,
                }],
                diff_bytes: 10,
                repair_attempt: 0,
            },
        )
        .expect_err("empty task_id should return ValidatorRejected");
    assert!(matches!(err, WasmError::ValidatorRejected(_)));
}

#[test]
fn wasmtime_enforces_fuel_ceiling_on_infinite_loop() {
    let infinite_wat = r#"
(module
    (import "$root" "log" (func $host_log (param i32 i32 i32 i32)))
    (memory (export "memory") 1)
    (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
        (i32.const 1024)
    )
    (func (export "runlark:workflow/validator@0.1.0#validate-run")
        (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i64 i32) (result i32)
        (loop $spin (br $spin))
        (i32.const 0)
    )
)
"#;
    let limits = ResourceLimits {
        max_fuel: 2_000,
        ..ResourceLimits::default()
    };
    let pkg = make_package_from_wat(infinite_wat, vec!["log".to_string()], limits);
    let host = WasmValidatorHost::new().expect("engine initializes");

    let err = host
        .validate_run(
            &pkg,
            &ValidationInput {
                task_id: "task-spin".to_string(),
                run_id: "run-spin".to_string(),
                stage: WorkflowStage::Validate,
                changed_files: vec!["a.rs".to_string()],
                checks: vec![],
                diff_bytes: 1,
                repair_attempt: 0,
            },
        )
        .expect_err("infinite loop must trap with OutOfFuel");
    match err {
        WasmError::Execution(msg) => assert!(msg.contains("OutOfFuel"), "unexpected msg: {msg}"),
        other => panic!("expected Execution(OutOfFuel), got {other:?}"),
    }
}

#[test]
fn wasmtime_enforces_memory_limit_on_excessive_growth() {
    let oom_wat = r#"
(module
    (import "$root" "log" (func $host_log (param i32 i32 i32 i32)))
    (memory (export "memory") 1)
    (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
        (i32.const 1024)
    )
    (func (export "runlark:workflow/validator@0.1.0#validate-run")
        (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i64 i32) (result i32)
        (drop (memory.grow (i32.const 100)))
        (i32.const 0)
    )
)
"#;
    let limits = ResourceLimits {
        max_memory_bytes: 2 * 65_536,
        ..ResourceLimits::default()
    };
    let pkg = make_package_from_wat(oom_wat, vec!["log".to_string()], limits);
    let host = WasmValidatorHost::new().expect("engine initializes");

    let err = host
        .validate_run(
            &pkg,
            &ValidationInput {
                task_id: "task-oom".to_string(),
                run_id: "run-oom".to_string(),
                stage: WorkflowStage::Validate,
                changed_files: vec![],
                checks: vec![],
                diff_bytes: 0,
                repair_attempt: 0,
            },
        )
        .expect_err("memory.grow past store limit must trap");
    assert!(matches!(err, WasmError::Execution(_)));
}

#[test]
fn rejects_undeclared_capabilities_and_ambient_wasi_imports() {
    let host = WasmValidatorHost::new().expect("engine initializes");

    // 1. Missing `log` capability in manifest for a component that imports `log`.
    let pkg = make_package_from_wat(SOLO_VALIDATOR_WAT, vec![], ResourceLimits::default());
    let err = host
        .inspect_component(&pkg.component_bytes, &pkg.manifest.capabilities)
        .expect_err("missing `log` capability must be rejected");
    assert!(matches!(err, WasmError::IncompatibleComponent(_)));

    // 2. Valid Component Model binary with an undeclared/ambient import (e.g. `wasi-fs`) must be rejected pre-instantiation.
    let rogue_contract_wit = r"
package runlark:workflow@0.1.0;

interface types {
    enum workflow-stage { clarify, plan, implement, validate, human-accept }
    record check-evidence { name: string, passed: bool, exit-code: s32 }
    record validation-input {
        task-id: string,
        run-id: string,
        stage: workflow-stage,
        changed-files: list<string>,
        checks: list<check-evidence>,
        diff-bytes: u64,
        repair-attempt: u32,
    }
    enum verdict { pass, fail }
    record validation-result { verdict: verdict, summary: string, findings: list<string> }
}

interface validator {
    use types.{validation-input, validation-result};
    validate-run: func(input: validation-input) -> result<validation-result, string>;
}

world rogue-validator {
    import wasi-fs-open: func(path: string);
    export validator;
}
";
    let rogue_module_wat = r#"
(module
    (import "$root" "wasi-fs-open" (func (param i32 i32)))
    (memory (export "memory") 1)
    (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
        (i32.const 1024)
    )
    (func (export "runlark:workflow/validator@0.1.0#validate-run")
        (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i64 i32) (result i32)
        (i32.const 0)
    )
)
"#;
    let mut resolve = Resolve::default();
    let group =
        UnresolvedPackageGroup::parse("rogue.wit", rogue_contract_wit).expect("rogue WIT parses");
    let pkg_id = resolve.push_group(group).expect("rogue WIT resolves");
    let world_id = resolve
        .select_world(pkg_id, Some("rogue-validator"))
        .expect("rogue world exists");
    let mut core = wat::parse_str(rogue_module_wat).expect("rogue WAT parses");
    embed_component_metadata(&mut core, &resolve, world_id, StringEncoding::UTF8)
        .expect("embed metadata");
    let component_bytes = ComponentEncoder::default()
        .module(&core)
        .expect("module")
        .validate(true)
        .encode()
        .expect("encode");

    let err = host
        .inspect_component(&component_bytes, &["log".to_string()])
        .expect_err("undeclared ambient import must be rejected");
    match err {
        WasmError::IncompatibleComponent(msg) => {
            assert!(msg.contains("wasi-fs-open"), "msg: {msg}");
        }
        other => panic!("expected IncompatibleComponent, got {other:?}"),
    }

    // 3. Digest mismatch in memory is rejected before compilation/execution.
    let mut tampered = pkg;
    tampered.package_digest = PackageDigest("0".repeat(64));
    tampered.component_bytes[0] ^= 0xff;
    let err = host
        .validate_run(
            &tampered,
            &ValidationInput {
                task_id: "t".to_string(),
                run_id: "r".to_string(),
                stage: WorkflowStage::Validate,
                changed_files: vec![],
                checks: vec![],
                diff_bytes: 0,
                repair_attempt: 0,
            },
        )
        .expect_err("tampered bytes must fail digest verification");
    assert!(matches!(err, WasmError::DigestMismatch { .. }));
}
