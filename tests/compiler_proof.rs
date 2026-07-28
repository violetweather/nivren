use nivren::runtime::{Interpreter, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

fn compile(source: &str) -> nivren::bytecode::Chunk {
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    nivren::bytecode::compile(&program).unwrap()
}

fn assert_vm_native_equivalent(source: &str) -> Value {
    let chunk = compile(source);
    let vm = Interpreter::new().run_bytecode(&chunk);
    let mut native = Interpreter::new();
    let native_result = native.run_native(&chunk);
    assert_eq!(
        native_result.as_ref().map_err(ToString::to_string),
        vm.as_ref().map_err(ToString::to_string)
    );
    let stats = native.native_stats();
    assert!(stats.compilations > 0);
    assert!(stats.executions > 0);
    assert_eq!(stats.fallbacks, 0);
    native_result.unwrap()
}

#[test]
fn edition_four_proof_programs_have_vm_native_equivalence() {
    for path in [
        "proofs/edition4/intent_snapshot.niv",
        "proofs/edition4/database_service.niv",
        "proofs/edition4/native_binding.niv",
        "proofs/edition4/concurrent_pipeline.niv",
    ] {
        let source = std::fs::read_to_string(path).unwrap();
        assert_vm_native_equivalent(&source);
    }
}

#[test]
fn native_tier_covers_choices_collections_closures_and_protocols() {
    let source = r#"
protocol Named {
    define name(value: Self) gives String
}

shape Person { name: String }
define person_name(value: Person) gives String {
    give value.name
}
adopt Named for Person { name = person_name }

define present<Value: Named>(value: Value) gives String {
    give Named.name(value)
}
define outer(value: Int) {
    define multiply(item: Int) gives Int { give value * item }
    give multiply
}

keep values = [1, 2, 3]
keep times_twenty = outer(20)
keep label = present(Person("Nivren"))
times_twenty(values[1])
"#;
    assert_eq!(assert_vm_native_equivalent(source), Value::Int(40));
}

#[test]
fn checked_failures_are_identical_without_native_fallback() {
    for source in ["9223372036854775807 + 1", "[1][2]"] {
        let chunk = compile(source);
        let vm = Interpreter::new()
            .run_bytecode(&chunk)
            .unwrap_err()
            .to_string();
        let mut native = Interpreter::new();
        let native_error = native.run_native(&chunk).unwrap_err().to_string();
        assert_eq!(native_error, vm);
        assert_eq!(native.native_stats().fallbacks, 0);
    }
}

#[test]
fn native_capabilities_limits_and_cancellation_match_the_vm() {
    let denied = compile(
        "define effect() gives Result<String, String> needs Native { give std.host.invoke(\"device\", \"read\") } effect()",
    );
    let vm = Interpreter::new()
        .with_capabilities(Vec::<String>::new())
        .run_bytecode(&denied)
        .unwrap_err()
        .to_string();
    let native = Interpreter::new()
        .with_capabilities(Vec::<String>::new())
        .run_native(&denied)
        .unwrap_err()
        .to_string();
    assert_eq!(native, vm);

    let runaway = compile("repeat yes { none }");
    let vm = Interpreter::new()
        .with_instruction_limit(64)
        .run_bytecode(&runaway)
        .unwrap_err()
        .to_string();
    let native = Interpreter::new()
        .with_instruction_limit(64)
        .run_native(&runaway)
        .unwrap_err()
        .to_string();
    assert_eq!(native, vm);

    let cancelled = Arc::new(AtomicBool::new(true));
    let vm = Interpreter::new()
        .with_cancellation(cancelled.clone())
        .run_bytecode(&compile("40 + 2"))
        .unwrap_err()
        .to_string();
    cancelled.store(true, Ordering::Release);
    let native = Interpreter::new()
        .with_cancellation(cancelled)
        .run_native(&compile("40 + 2"))
        .unwrap_err()
        .to_string();
    assert_eq!(native, vm);
}

#[test]
fn native_using_cleanup_closes_owned_foreign_handles_once() {
    let chunk = compile(
        r#"
define query() gives Result<String, String> needs Native {
    keep opened = std.host.open("database", "configuration") or give
    using handle = opened {
        give std.host.call(handle, "query", "select 42")
    }
}
query()
"#,
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let mut interpreter = Interpreter::new().with_host_callback(move |operation, _| {
        captured.lock().unwrap().push(operation.to_string());
        match operation {
            "nivren.handle.open:database" => Ok("handle-42".into()),
            "nivren.handle.call:query" => Ok("row:42".into()),
            "nivren.handle.close" => Ok("closed".into()),
            _ => Err("unexpected operation".into()),
        }
    });
    assert_eq!(
        interpreter.run_native(&chunk).unwrap(),
        Value::Ok(Arc::new(Value::String("row:42".into())))
    );
    assert_eq!(
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.as_str() == "nivren.handle.close")
            .count(),
        1
    );
    assert_eq!(interpreter.native_stats().fallbacks, 0);
}

#[test]
fn malformed_artifacts_are_rejected_before_native_compilation() {
    let malformed = nivren::bytecode::Chunk {
        version: nivren::bytecode::BYTECODE_VERSION,
        code: vec![nivren::bytecode::Instruction {
            op: nivren::bytecode::Op::Jump(99),
            span: nivren::ast::Span { line: 1, column: 1 },
        }],
    };
    let mut interpreter = Interpreter::new();
    assert!(interpreter.run_native(&malformed).is_err());
    assert_eq!(interpreter.native_stats().compilations, 0);
    assert_eq!(interpreter.native_stats().executions, 0);
}

#[test]
fn unsafe_system_modules_are_individually_declared_and_fingerprinted() {
    let root = std::path::PathBuf::from("/tmp/nivren-unsafe-manifest-proof");
    let source = r#"
[package]
name = "systems-proof"
version = "1.0.0"
entry = "main.niv"

[capabilities]
Native = "allow"

[unsafe]
memory = "allow"
layouts = "allow"
allocators = "allow"
atomics = "allow"
threads = "allow"
simd = "allow"
devices = "allow"
ffi = "allow"
"#;
    let manifest = nivren::project::Manifest::parse(source, root.clone()).unwrap();
    assert_eq!(manifest.unsafe_modules.len(), 8);
    let reparsed = nivren::project::Manifest::parse(&manifest.source(), root.clone()).unwrap();
    assert_eq!(reparsed.unsafe_modules, manifest.unsafe_modules);
    assert!(
        nivren::project::Manifest::parse(
            &source.replace("[capabilities]\nNative = \"allow\"\n", ""),
            root.clone(),
        )
        .is_err()
    );
    assert!(
        nivren::project::Manifest::parse(
            &source.replace("memory = \"allow\"", "pointers = \"allow\""),
            root,
        )
        .is_err()
    );
}
