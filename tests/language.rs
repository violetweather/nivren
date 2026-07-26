use nivren::runtime::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn eval(source: &str) -> Value {
    nivren::run(source).unwrap()
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").unwrap();
    }
    output
}

fn eval_vm(source: &str) -> Value {
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    nivren::runtime::Interpreter::new()
        .run_bytecode(&chunk)
        .unwrap()
}

fn eval_tree(source: &str) -> Value {
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    nivren::runtime::Interpreter::new().run(&program).unwrap()
}

#[test]
fn arithmetic_and_precedence() {
    assert_eq!(eval("2 + 3 * 4"), Value::Int(14));
}

#[test]
fn immutable_bindings_reject_assignment() {
    let errors = nivren::run("keep answer = 42; answer = 7;").unwrap_err();
    assert!(errors[0].message.contains("immutable"));
}

#[test]
fn mutable_bindings_and_loops_work() {
    assert_eq!(
        eval("change n = 0; repeat (n < 5) { n = n + 1; } n"),
        Value::Int(5)
    );
}

#[test]
fn functions_return_values() {
    assert_eq!(
        eval("define add(a, b) { give a + b; } add(20, 22)"),
        Value::Int(42)
    );
}

#[test]
fn closures_capture_scope() {
    assert_eq!(
        eval(
            "define outer(x) { define inner(y) { give x + y; } give inner; } keep add2 = outer(2); add2(40)"
        ),
        Value::Int(42)
    );
}

#[test]
fn conditions_require_booleans() {
    let errors = nivren::run("when 1 { 2 }").unwrap_err();
    assert!(errors[0].message.contains("expected Bool"));
}

#[test]
fn scanner_reports_locations() {
    let errors = nivren::check("keep x = @;").unwrap_err();
    assert_eq!((errors[0].line, errors[0].column), (1, 10));
}

#[test]
fn prototype_spellings_are_not_part_of_edition_two() {
    for source in [
        "let value = 1",
        "var value = 1",
        "fun value() { return 1 }",
        "if (true) { print(1) } else { print(0) }",
        "while (false) {}",
        "for (value in [1]) {}",
        "record Value { item: Int }",
        "enum Value { One }",
        "match (value) { One => 1 }",
        "import \"value.niv\"",
        "export { value }",
        "null",
    ] {
        assert!(
            nivren::check(source).is_err(),
            "accepted prototype source: {source}"
        );
    }
}

#[test]
fn nested_block_comments_work() {
    assert_eq!(eval("/* one /* two */ three */ 42"), Value::Int(42));
}

#[test]
fn undefined_names_fail_during_check() {
    let errors = nivren::check("missing + 1").unwrap_err();
    assert!(errors[0].message.contains("undefined name"));
}

#[test]
fn operator_misuse_fails_during_check() {
    let errors = nivren::check("yes - 1").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("numeric operands"))
    );
}

#[test]
fn arity_fails_during_check() {
    let errors = nivren::check("define pair(a, b) { give a; } pair(1)").unwrap_err();
    assert!(errors[0].message.contains("expects 2"));
}

#[test]
fn immutable_arrays_support_safe_indexing() {
    assert_eq!(
        eval("keep values = [10, 20, 30]; values[1]"),
        Value::Int(20)
    );
    assert_eq!(
        eval("keep values = append([1, 2], 3); len(values)"),
        Value::Int(3)
    );
}

#[test]
fn array_bounds_are_checked() {
    let errors = nivren::run("[1][2]").unwrap_err();
    assert!(errors[0].message.contains("out of bounds"));
}

#[test]
fn mixed_array_types_fail_check() {
    let errors = nivren::check("[1, yes]").unwrap_err();
    assert!(errors[0].message.contains("one type"));
}

#[test]
fn annotations_check_bindings_arguments_and_returns() {
    let source = "define add(a: Int, b: Int) gives Int { give a + b; } keep answer: Int = add(20, 22); answer";
    assert_eq!(eval(source), Value::Int(42));
    assert!(nivren::check("keep value: String = 42").is_err());
    assert!(nivren::check("define bad() gives Bool { give 1; }").is_err());
    assert!(
        nivren::check("define onlyInt(value: Int) gives Int { give value; } onlyInt(yes)").is_err()
    );
}

#[test]
fn assertions_can_guard_language_tests() {
    assert_eq!(eval("assert(2 + 2 == 4, \"math\")"), Value::Null);
    assert!(
        nivren::run("assert(no, \"broken\")").unwrap_err()[0]
            .message
            .contains("broken")
    );
}

#[test]
fn nullable_types_require_explicit_declaration_and_fallback() {
    assert_eq!(
        eval("keep missing: String? = none; missing ?? \"fallback\""),
        Value::String("fallback".into())
    );
    assert_eq!(
        eval("keep present: String? = \"value\"; present ?? \"fallback\""),
        Value::String("value".into())
    );
    assert!(nivren::check("keep invalid: String = none").is_err());
    assert!(nivren::check("keep plain: Int = 1; plain ?? 2").is_err());
}

#[test]
fn records_are_nominal_typed_values() {
    let source =
        "shape Person { name: String, age: Int } keep ada: Person = Person(\"Ada\", 37); ada.name";
    assert_eq!(eval(source), Value::String("Ada".into()));
    assert!(nivren::check("shape Person { name: String } keep person = Person(42)").is_err());
    assert!(
        nivren::check(
            "shape Person { name: String } keep person = Person(\"Ada\"); person.missing"
        )
        .is_err()
    );
}

#[test]
fn sealed_enums_require_exhaustive_matches() {
    let source = "choice State { Idle, Running, Done } keep state: State = State.Running; choose (state) { Idle => 0, Running => 1, Done => 2 }";
    assert_eq!(eval(source), Value::Int(1));
    assert!(
        nivren::check(
            "choice State { Idle, Done } keep state = State.Idle; choose (state) { Idle => 0 }"
        )
        .is_err()
    );
    assert!(nivren::check("choice State { Idle } keep state = State.Missing").is_err());
}

#[test]
fn for_iteration_is_typed_and_unicode_safe() {
    assert_eq!(
        eval(
            "change total: Int = 0; each (value within [1, 2, 3]) { total = total + value; } total"
        ),
        Value::Int(6)
    );
    assert_eq!(
        eval(
            "change count: Int = 0; each (character within \"a💡c\") { count = count + 1; } count"
        ),
        Value::Int(3)
    );
    assert!(nivren::check("each (value within 42) { show(value) }").is_err());
}

#[test]
fn integers_and_floats_are_distinct_and_overflow_is_trapped() {
    assert_eq!(eval("1 + 2"), Value::Int(3));
    assert_eq!(eval("1.5 + 2.25"), Value::Float(3.75));
    assert!(nivren::check("1 + 2.0").is_err());
    let errors = nivren::run("9223372036854775807 + 1").unwrap_err();
    assert!(errors[0].message.contains("integer overflow"));
}

#[test]
fn typed_results_require_exhaustive_payload_matching() {
    let source = "define parse(valid: Bool) gives Result<Int, String> { when (valid) { give ok(42); } give err(\"invalid\"); } keep result: Result<Int, String> = parse(yes); choose (result) { Ok(value) => value, Err(message) => 0 }";
    assert_eq!(eval(source), Value::Int(42));
    assert!(
        nivren::check(
            "keep result: Result<Int, String> = ok(1); choose (result) { Ok(value) => value }"
        )
        .is_err()
    );
    assert!(nivren::check("keep result: Result<Int, String> = err(\"bad\"); choose (result) { Ok => 1, Err(error) => 0 }").is_err());
}

fn module_fixture(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "nivren-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn file_modules_resolve_relative_imports_once() {
    let directory = module_fixture("modules");
    fs::write(
        directory.join("math.niv"),
        "define double(value: Int) gives Int { give value * 2; } keep private = 7; expose { double };",
    )
    .unwrap();
    let entry = directory.join("main.niv");
    fs::write(
        &entry,
        "use \"math.niv\"; use \"math.niv\"; math.double(21)",
    )
    .unwrap();

    let program = nivren::modules::load(&entry).unwrap();
    nivren::typecheck::check(&program).unwrap();
    assert_eq!(
        nivren::runtime::Interpreter::new().run(&program).unwrap(),
        Value::Int(42)
    );

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn module_members_are_private_unless_exported() {
    let directory = module_fixture("private-modules");
    fs::write(
        directory.join("secrets.niv"),
        "keep visible = 1; keep hidden = 2; expose { visible };",
    )
    .unwrap();
    let entry = directory.join("main.niv");
    fs::write(&entry, "use \"secrets.niv\"; secrets.hidden").unwrap();

    let program = nivren::modules::load(&entry).unwrap();
    let errors = nivren::typecheck::check(&program).unwrap_err();
    assert!(errors[0].message.contains("no exposed member 'hidden'"));

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn module_record_types_are_nominally_namespaced() {
    let directory = module_fixture("nominal-modules");
    fs::write(
        directory.join("numbers.niv"),
        "shape Box { value: Int } define read(box: Box) gives Int { give box.value; } expose { Box, read };",
    )
    .unwrap();
    fs::write(
        directory.join("strings.niv"),
        "shape Box { value: String } expose { Box };",
    )
    .unwrap();
    let entry = directory.join("main.niv");
    fs::write(
        &entry,
        "use \"numbers.niv\"; use \"strings.niv\"; numbers.read(strings.Box(\"wrong\"))",
    )
    .unwrap();

    let program = nivren::modules::load(&entry).unwrap();
    let errors = nivren::typecheck::check(&program).unwrap_err();
    assert!(errors[0].message.contains("numbers.Box"));
    assert!(errors[0].message.contains("strings.Box"));

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn file_modules_reject_import_cycles() {
    let directory = module_fixture("cycles");
    let first = directory.join("first.niv");
    fs::write(&first, "use \"second.niv\";").unwrap();
    fs::write(directory.join("second.niv"), "use \"first.niv\";").unwrap();

    let errors = nivren::modules::load(&first).unwrap_err();
    assert!(errors[0].message.contains("use cycle"));

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn project_manifests_are_strict_and_deterministic() {
    let root = PathBuf::from("/tmp/example");
    let source =
        "[package]\nname = \"example-app\"\nversion = \"1.2.3\"\nentry = \"src/main.niv\"\n";
    let manifest = nivren::project::Manifest::parse(source, root.clone()).unwrap();
    assert_eq!(manifest.name, "example-app");
    assert_eq!(manifest.entry_path(), root.join("src/main.niv"));
    assert!(manifest.dependencies.is_empty());
    assert_eq!(manifest.lockfile(), manifest.lockfile());
    assert!(
        nivren::project::Manifest::parse(&source.replace("1.2.3", "01.2.3"), root.clone()).is_err()
    );
    assert!(
        nivren::project::Manifest::parse(&source.replace("src/main.niv", "../main.niv"), root)
            .is_err()
    );
}

#[test]
fn project_dependencies_are_exact_strict_and_canonically_locked() {
    let root = PathBuf::from("/tmp/example");
    let source = "[package]\nname = \"app\"\nversion = \"1.0.0\"\nentry = \"main.niv\"\n\n[dependencies]\nzed = \"2.0.0\"\nalpha = \"1.2.3\"\n";
    let manifest = nivren::project::Manifest::parse(source, root.clone()).unwrap();
    assert_eq!(manifest.dependencies["alpha"], "1.2.3");
    let resolved = BTreeMap::from([
        (("zed".into(), "2.0.0".into()), "b".repeat(64)),
        (("alpha".into(), "1.2.3".into()), "a".repeat(64)),
    ]);
    let lock = manifest.resolved_lockfile(&resolved);
    assert!(lock.find("name = \"alpha\"").unwrap() < lock.find("name = \"zed\"").unwrap());
    assert!(
        nivren::project::Manifest::parse(&source.replace("1.2.3", "^1.2"), root.clone()).is_err()
    );
    assert!(nivren::project::Manifest::parse(&source.replace("alpha", "bad-name"), root).is_err());
}

#[test]
fn registry_dependencies_install_lock_import_and_detect_tampering() {
    let directory = module_fixture("dependencies");
    let dependency_root = directory.join("answerlib");
    fs::create_dir_all(&dependency_root).unwrap();
    fs::write(
        dependency_root.join("niv.toml"),
        "[package]\nname = \"answerlib\"\nversion = \"1.0.0\"\nentry = \"main.niv\"\n",
    )
    .unwrap();
    fs::write(
        dependency_root.join("main.niv"),
        "define answer() gives Int { give 42; } expose { answer };",
    )
    .unwrap();
    let dependency = nivren::project::Manifest::load(&dependency_root).unwrap();
    let archive = nivren::package::Package::build(&dependency)
        .unwrap()
        .encode()
        .unwrap();
    let registry = directory.join("registry");
    nivren::package::publish(&archive, &registry).unwrap();

    let app = directory.join("app");
    fs::create_dir_all(&app).unwrap();
    fs::write(
        app.join("niv.toml"),
        "[package]\nname = \"app\"\nversion = \"1.0.0\"\nentry = \"main.niv\"\n\n[dependencies]\nanswerlib = \"1.0.0\"\n",
    )
    .unwrap();
    fs::write(
        app.join("main.niv"),
        "use \"@answerlib\"; answerlib.answer()",
    )
    .unwrap();
    let app_manifest = nivren::project::Manifest::load(&app).unwrap();
    assert_eq!(
        nivren::package::install_dependencies(&app_manifest, &registry).unwrap(),
        1
    );
    let expected_lock = nivren::package::installed_lockfile(&app_manifest).unwrap();
    assert_eq!(
        fs::read_to_string(app.join("niv.lock")).unwrap(),
        expected_lock
    );
    let program = nivren::modules::load_project(&app, &app.join("main.niv")).unwrap();
    nivren::typecheck::check(&program).unwrap();
    assert_eq!(
        nivren::runtime::Interpreter::new().run(&program).unwrap(),
        Value::Int(42)
    );

    fs::write(
        app.join(".niv/deps/answerlib-1.0.0/main.niv"),
        "expose { answer }; keep answer = 0;",
    )
    .unwrap();
    assert!(nivren::package::installed_lockfile(&app_manifest).is_err());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn project_fingerprints_are_content_addressed_and_ignore_build_outputs() {
    let directory = module_fixture("fingerprint");
    fs::create_dir_all(directory.join("src")).unwrap();
    fs::write(directory.join("src/main.niv"), "42").unwrap();
    let manifest = nivren::project::Manifest::parse(
        "[package]\nname = \"fingerprint\"\nversion = \"0.1.0\"\nentry = \"src/main.niv\"\n",
        directory.canonicalize().unwrap(),
    )
    .unwrap();
    let first = manifest.fingerprint().unwrap();
    assert_eq!(first, manifest.fingerprint().unwrap());
    fs::create_dir_all(directory.join("target")).unwrap();
    fs::write(directory.join("target/noise.niv"), "ignored").unwrap();
    assert_eq!(first, manifest.fingerprint().unwrap());
    fs::write(directory.join("src/main.niv"), "43").unwrap();
    assert_ne!(first, manifest.fingerprint().unwrap());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn project_modules_cannot_escape_the_root() {
    let directory = module_fixture("project-root");
    let outside = directory.parent().unwrap().join(format!(
        "nivren-outside-{}-{:?}.niv",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::write(&outside, "keep secret = 42;").unwrap();
    let entry = directory.join("main.niv");
    fs::write(
        &entry,
        format!(
            "use \"../{}\";",
            outside.file_name().unwrap().to_string_lossy()
        ),
    )
    .unwrap();

    let errors = nivren::modules::load_project(&directory, &entry).unwrap_err();
    assert!(errors[0].message.contains("outside the project root"));

    fs::remove_file(outside).unwrap();
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn formatter_is_comment_safe_and_idempotent() {
    let source = "define main() {\nkeep text = \"{not a block}\" // }\n/* { nested /* } */ ok */\nwhen yes {\nshow(text)\n}\n}\n";
    let formatted = nivren::formatter::format(source);
    assert!(formatted.contains("    keep text = \"{not a block}\" // }"));
    assert!(formatted.contains("    /* { nested /* } */ ok */"));
    assert_eq!(nivren::formatter::format(&formatted), formatted);
}

#[test]
fn documentation_lists_only_explicit_module_exports() {
    let source =
        "define public(value: Int) gives Int { give value; } keep hidden = 1; expose { public };";
    let parsed = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    let module = nivren::ast::Stmt::Module {
        name: "sample".into(),
        body: parsed[..2].to_vec(),
        exports: vec!["public".into()],
        span: nivren::ast::Span { line: 1, column: 1 },
    };
    let docs = nivren::documentation::generate("package", "1.0.0", &[module]);
    assert!(docs.contains("define public(value: Int) gives Int"));
    assert!(!docs.contains("hidden"));
}

#[test]
fn bytecode_is_versioned_verified_and_deterministic() {
    let source = "define sum(limit: Int) gives Int { change total = 0; change index = 0; repeat (index < limit) { total = total + index; index = index + 1; } give total; } sum(5)";
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    nivren::bytecode::verify(&chunk).unwrap();
    let first = nivren::bytecode::disassemble(&chunk);
    assert_eq!(first, nivren::bytecode::disassemble(&chunk));
    assert!(first.starts_with("NIVB 1\n"));

    let mut incompatible = chunk;
    incompatible.version += 1;
    assert!(
        nivren::bytecode::verify(&incompatible)
            .unwrap_err()
            .message
            .contains("unsupported bytecode version")
    );
}

#[test]
fn bytecode_verifier_rejects_stack_underflow() {
    let chunk = nivren::bytecode::Chunk {
        version: nivren::bytecode::BYTECODE_VERSION,
        code: vec![nivren::bytecode::Instruction {
            op: nivren::bytecode::Op::Pop,
            span: nivren::ast::Span { line: 4, column: 2 },
        }],
    };
    let error = nivren::bytecode::verify(&chunk).unwrap_err();
    assert_eq!((error.line, error.column), (4, 2));
    assert!(error.message.contains("underflow"));
}

#[test]
fn bytecode_verifier_rejects_invalid_operands_and_scopes() {
    let span = nivren::ast::Span { line: 2, column: 3 };
    let invalid_operator = nivren::bytecode::Chunk {
        version: nivren::bytecode::BYTECODE_VERSION,
        code: vec![
            nivren::bytecode::Instruction {
                op: nivren::bytecode::Op::Constant(nivren::ast::Literal::Int(1)),
                span,
            },
            nivren::bytecode::Instruction {
                op: nivren::bytecode::Op::Unary(nivren::lexer::TokenKind::Print),
                span,
            },
        ],
    };
    assert!(
        nivren::bytecode::verify(&invalid_operator)
            .unwrap_err()
            .message
            .contains("invalid bytecode operand")
    );

    let open_scope = nivren::bytecode::Chunk {
        version: nivren::bytecode::BYTECODE_VERSION,
        code: vec![
            nivren::bytecode::Instruction {
                op: nivren::bytecode::Op::EnterScope,
                span,
            },
            nivren::bytecode::Instruction {
                op: nivren::bytecode::Op::Constant(nivren::ast::Literal::Null),
                span,
            },
        ],
    };
    assert!(
        nivren::bytecode::verify(&open_scope)
            .unwrap_err()
            .message
            .contains("open scope")
    );
}

#[test]
fn binary_bundles_round_trip_and_execute() {
    let source = "shape Pair { left: Int, right: Int } choice Choice { First, Second } define pick(value: Choice) gives Int { give choose value { First => 1, Second => 2 }; } change total = 0; each value within [10, 20] { total = total + value; } keep pair = Pair(total, pick(Choice.Second)); pair.left + pair.right";
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let compiled = nivren::bytecode::compile(&program).unwrap();
    let bytes = nivren::bundle::encode(&compiled).unwrap();
    let decoded = nivren::bundle::decode(&bytes).unwrap();
    assert_eq!(decoded, compiled);
    assert_eq!(
        nivren::runtime::Interpreter::new()
            .run_bytecode(&decoded)
            .unwrap(),
        Value::Int(32)
    );
}

#[test]
fn binary_bundle_decoder_rejects_hostile_input() {
    let program = nivren::parser::parse(nivren::lexer::scan("42").unwrap()).unwrap();
    let bytes = nivren::bundle::encode(&nivren::bytecode::compile(&program).unwrap()).unwrap();
    for length in 0..bytes.len() {
        assert!(nivren::bundle::decode(&bytes[..length]).is_err());
    }

    let mut unknown_instruction = bytes.clone();
    unknown_instruction[18] = 255;
    assert!(
        nivren::bundle::decode(&unknown_instruction)
            .unwrap_err()
            .message
            .contains("unknown bytecode instruction")
    );

    let mut oversized = bytes.clone();
    oversized[6..10].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(
        nivren::bundle::decode(&oversized)
            .unwrap_err()
            .message
            .contains("allocation limit")
    );

    let mut trailing = bytes;
    trailing.push(0);
    assert!(
        nivren::bundle::decode(&trailing)
            .unwrap_err()
            .message
            .contains("trailing data")
    );
}

#[test]
fn gc_stress_preserves_escaping_closures_and_collects_cycles() {
    let source = "define make(base: Int) { define add(value: Int) gives Int { give base + value; } give add; } keep escaped = make(40); change index = 0; repeat (index < 100) { define temporary() gives Int { give index; } temporary(); index = index + 1; } escaped(2)";
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let mut interpreter = nivren::runtime::Interpreter::new();
    interpreter.set_gc_stress(true);
    assert_eq!(interpreter.run_bytecode(&chunk).unwrap(), Value::Int(42));
    interpreter.collect_garbage();
    let stats = interpreter.heap_stats();
    assert!(stats.collections > 100);
    assert!(stats.minor_collections > 100);
    assert!(stats.major_collections > 0 || stats.concurrent_marking);
    assert_eq!(stats.live_environments, 2);
}

#[test]
fn typed_standard_library_handles_files_paths_time_and_process_errors() {
    let directory = module_fixture("stdlib");
    let file = directory.join("message.txt");
    let path = file.to_string_lossy().replace('\\', "\\\\");
    let source = format!(
        "keep writeResult: Result<Null, String> = std.fs.write(\"{path}\", \"hello\"); assert(choose (writeResult) {{ Ok(value) => yes, Err(error) => no }}, \"write\"); keep readResult: Result<String, String> = std.fs.read(\"{path}\"); keep text = choose (readResult) {{ Ok(value) => value, Err(error) => error }}; assert(std.fs.exists(\"{path}\"), \"exists\"); assert((std.path.basename(\"{path}\") ?? \"\") == \"message.txt\", \"basename\"); std.time.sleep(0.0); text"
    );
    assert_eq!(eval_vm(&source), Value::String("hello".into()));

    let process = "keep result: Result<String, String> = std.process.run(\"nivren-command-that-does-not-exist-4f3d\", []); choose (result) { Ok(output) => no, Err(error) => yes }";
    assert_eq!(eval_vm(process), Value::Bool(true));
    assert!(nivren::check("std.fs.read(42)").is_err());

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn bounded_json_engine_validates_unicode_and_formats_deterministically() {
    let source = " { \"emoji\" : \"\\uD83D\\uDCA1\", \"items\" : [1, true, null] } ";
    assert!(nivren::json::valid(source));
    assert_eq!(
        nivren::json::compact(source).unwrap(),
        "{\"emoji\":\"💡\",\"items\":[1,true,null]}"
    );
    assert!(nivren::json::pretty(source).unwrap().ends_with("}\n"));
    for invalid in [
        "{\"key\": 1, \"key\": 2}",
        "01",
        "\"\\uD800\"",
        "[1,]",
        "true false",
    ] {
        assert!(
            !nivren::json::valid(invalid),
            "accepted invalid JSON: {invalid}"
        );
    }

    let program = "keep result: Result<String, String> = std.json.compact(\"{\\\"ok\\\": true}\"); choose result { Ok(value) => value, Err(error) => error }";
    assert_eq!(eval_vm(program), Value::String("{\"ok\":true}".into()));
}

#[test]
fn typed_tcp_standard_library_uses_bounded_timeouts() {
    use std::io::Write as _;

    let probe_listener = match std::net::TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("cannot bind loopback test listener: {error}"),
    };
    let probe_address = probe_listener.local_addr().unwrap();
    let probe = match std::net::TcpStream::connect(probe_address) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::AddrNotAvailable
            ) =>
        {
            return;
        }
        Err(error) => panic!("cannot connect to loopback test listener: {error}"),
    };
    let (accepted, _) = probe_listener.accept().unwrap();
    drop((probe, accepted, probe_listener));

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.write_all(b"hello").unwrap();
    });
    let source = format!(
        "keep connection = std.net.connect(\"127.0.0.1\", {port}, 2.0); choose (connection) {{ Ok(stream) => choose (std.net.read(stream, 5)) {{ Ok(text) => text, Err(error) => error }}, Err(error) => error }}"
    );
    assert_eq!(eval_vm(&source), Value::String("hello".into()));
    server.join().unwrap();
    assert!(nivren::check("std.net.connect(\"localhost\", \"80\", 1.0)").is_err());
}

#[test]
fn structured_tasks_cancel_and_exchange_channel_values() {
    let source = "keep channel = std.channel.create(1); define producer() gives Int { keep sent = std.channel.send(channel, 42, 2.0); give choose (sent) { Ok(value) => 1, Err(error) => 0 }; } keep task = std.task.spawn(producer); keep received = std.channel.receive(channel, 2.0); keep value = choose (received) { Ok(item) => item, Err(error) => 0 }; keep completed = std.task.await(task); assert(choose (completed) { Ok(code) => code == 1, Err(error) => no }, \"task completion\"); value";
    assert_eq!(eval_vm(source), Value::Int(42));

    let cancellation = "define forever() gives Int { change value = 0; repeat (value < 9223372036854775807) { value = value + 1; } give value; } keep task = std.task.spawn(forever); std.task.cancel(task); keep result = std.task.await(task); choose (result) { Ok(value) => no, Err(error) => yes }";
    assert_eq!(eval_vm(cancellation), Value::Bool(true));
    assert!(nivren::check("std.task.spawn(42)").is_err());
    assert!(nivren::run("std.task.spawn(42)").is_err());
}

#[test]
fn bytecode_vm_matches_the_tree_interpreter() {
    let programs = [
        "2 + 3 * 4",
        "change n = 0; repeat (n < 5) { n = n + 1; } n",
        "when yes and !no { 42 } otherwise { 0 }",
        "define outer(x: Int) { define inner(y: Int) { give x + y; } give inner; } outer(2)(40)",
        "keep values = append([1, 2], 3); values[2]",
        "keep missing: String? = none; missing ?? \"fallback\"",
        "shape Person { name: String, age: Int } keep person = Person(\"Ada\", 37); person.age",
        "choice State { Idle, Ready } keep state = State.Ready; choose (state) { Idle => 0, Ready => 42 }",
        "keep result: Result<Int, String> = ok(42); choose (result) { Ok(value) => value, Err(message) => 0 }",
        "change total = 0; each (value within [10, 20, 12]) { total = total + value; } total",
        "define first() gives Int { each (value within [42, 0]) { give value; } give 0; } first()",
    ];
    for source in programs {
        assert_eq!(
            eval_vm(source),
            eval_tree(source),
            "differential failure: {source}"
        );
    }
}

#[test]
fn bytecode_vm_executes_namespaced_modules() {
    let directory = module_fixture("bytecode-modules");
    fs::write(
        directory.join("answer.niv"),
        "define value() gives Int { give 42; } expose { value };",
    )
    .unwrap();
    let entry = directory.join("main.niv");
    fs::write(&entry, "use \"answer.niv\"; answer.value()").unwrap();
    let program = nivren::modules::load(&entry).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    assert_eq!(
        nivren::runtime::Interpreter::new()
            .run_bytecode(&chunk)
            .unwrap(),
        Value::Int(42)
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn bytecode_runtime_errors_include_call_frames() {
    let errors = nivren::run(
        "define inner() gives Int { give 1 / 0; } define outer() gives Int { give inner(); } outer()",
    )
    .unwrap_err();
    let functions: Vec<&str> = errors[0]
        .trace
        .iter()
        .map(|frame| frame.function.as_str())
        .collect();
    assert_eq!(functions, vec!["inner", "outer"]);
}

#[test]
fn runtime_metrics_cover_nested_bytecode_and_operations() {
    let source =
        "define twice(value: Int) gives Int { give value * 2; }\nkeep answer = twice(21);\nanswer";
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let mut interpreter = nivren::runtime::Interpreter::new();
    interpreter.enable_metrics();
    assert_eq!(interpreter.run_bytecode(&chunk).unwrap(), Value::Int(42));
    let metrics = interpreter.execution_metrics().unwrap();
    assert!(metrics.instructions > 0);
    assert!(metrics.line_hits.contains_key(&1));
    assert!(metrics.line_hits.contains_key(&2));
    assert!(
        metrics
            .operation_hits
            .get("call")
            .is_some_and(|hits| *hits == 1)
    );
    assert!(
        metrics
            .operation_hits
            .get("binary")
            .is_some_and(|hits| *hits == 1)
    );
}

#[test]
fn debugger_hook_steps_source_and_exposes_user_variables() {
    let source = "keep answer = 42;\nanswer + 1";
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let events = Arc::new(Mutex::new(vec![]));
    let captured = events.clone();
    let mut interpreter = nivren::runtime::Interpreter::new();
    interpreter.set_debug_hook(move |event| {
        captured.lock().unwrap().push(event.clone());
        nivren::runtime::DebugControl::Continue
    });
    assert_eq!(interpreter.run_bytecode(&chunk).unwrap(), Value::Int(43));
    let events = events.lock().unwrap();
    assert!(events.iter().any(|event| event.line == 2));
    assert!(events.iter().any(|event| {
        event
            .variables
            .get("answer")
            .is_some_and(|value| value == "42")
    }));
}

#[test]
fn packages_are_deterministic_traversal_safe_and_registry_verified() {
    let project = module_fixture("package-project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
        project.join("niv.toml"),
        "[package]\nname = \"sample\"\nversion = \"1.2.3\"\nentry = \"src/main.niv\"\n",
    )
    .unwrap();
    fs::write(project.join("src/main.niv"), "keep answer = 42; answer").unwrap();
    let manifest = nivren::project::Manifest::load(&project).unwrap();
    let package = nivren::package::Package::build(&manifest).unwrap();
    let first = package.encode().unwrap();
    let second = package.encode().unwrap();
    assert_eq!(first, second);
    let decoded = nivren::package::Package::decode(&first).unwrap();
    assert_eq!(
        (decoded.name.as_str(), decoded.version.as_str()),
        ("sample", "1.2.3")
    );

    let registry = module_fixture("package-registry");
    nivren::package::publish(&first, &registry).unwrap();
    let fetched = nivren::package::fetch("sample", "1.2.3", &registry).unwrap();
    assert_eq!(fetched, first);
    let destination = project.with_file_name(format!(
        "nivren-package-extracted-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    decoded.extract(&destination).unwrap();
    let extracted = nivren::project::Manifest::load(&destination).unwrap();
    assert_eq!(extracted.name, "sample");

    let mut changed = package.clone();
    changed
        .files
        .insert("src/main.niv".into(), b"keep answer = 7; answer".to_vec());
    assert!(nivren::package::publish(&changed.encode().unwrap(), &registry).is_err());
    let mut unsafe_package = package;
    unsafe_package
        .files
        .insert("../escape.niv".into(), b"42".to_vec());
    assert!(unsafe_package.encode().is_err());

    fs::remove_dir_all(project).unwrap();
    fs::remove_dir_all(registry).unwrap();
    fs::remove_dir_all(destination).unwrap();
}

#[test]
fn hot_integer_functions_tier_to_native_code_with_checked_overflow() {
    let source = "define twice_sum(a: Int, b: Int) gives Int { keep sum = a + b; give sum * 2; } twice_sum(1, 2); twice_sum(20, 1)";
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let mut interpreter = nivren::runtime::Interpreter::new();
    interpreter.set_jit_threshold(2);
    assert_eq!(interpreter.run_bytecode(&chunk).unwrap(), Value::Int(42));
    assert_eq!(
        interpreter.jit_stats(),
        nivren::runtime::JitStats {
            compilations: 1,
            executions: 1,
        }
    );

    let overflow =
        "define add(a: Int, b: Int) gives Int { give a + b; } add(9223372036854775807, 1)";
    let program = nivren::parser::parse(nivren::lexer::scan(overflow).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let mut interpreter = nivren::runtime::Interpreter::new();
    interpreter.set_jit_threshold(1);
    let error = interpreter.run_bytecode(&chunk).unwrap_err();
    assert!(error.message.contains("integer overflow"));
}

#[test]
fn public_registry_provenance_revocation_and_advisories_are_enforced() {
    let project = module_fixture("trusted-package");
    fs::write(
        project.join("niv.toml"),
        "[package]\nname = \"trusted\"\nversion = \"1.0.0\"\nentry = \"main.niv\"\n",
    )
    .unwrap();
    fs::write(project.join("main.niv"), "42").unwrap();
    let manifest = nivren::project::Manifest::load(&project).unwrap();
    let package = nivren::package::Package::build(&manifest)
        .unwrap()
        .encode()
        .unwrap();
    let root_secret = [7u8; 32];
    let publisher_secret = [9u8; 32];
    let publisher_key = hex(&nivren::trust::public_key(publisher_secret));
    let authorization = nivren::trust::authorize_publisher(
        root_secret,
        "team".into(),
        publisher_key.clone(),
        "example/trusted".into(),
        ".github/workflows/release.yml".into(),
        2_000,
    )
    .unwrap();
    let provenance = nivren::trust::attest_release(
        publisher_secret,
        &package,
        "team".into(),
        "example/trusted".into(),
        ".github/workflows/release.yml".into(),
        "0123456789abcdef".into(),
        1_000,
    )
    .unwrap();
    let status = nivren::trust::sign_status(
        root_secret,
        nivren::trust::RegistryStatus {
            generation: 1,
            issued_at: 1_000,
            expires_at: 2_000,
            revoked_keys: BTreeSet::new(),
            frozen_packages: BTreeMap::new(),
            signature: String::new(),
        },
    );
    let root_public = nivren::trust::public_key(root_secret);
    assert!(
        nivren::trust::verify_release(
            &package,
            &provenance,
            &authorization,
            &status,
            &[],
            root_public,
            1_100,
            0,
        )
        .is_ok()
    );

    let unsafe_authorization = nivren::trust::authorize_publisher(
        root_secret,
        "../escape".into(),
        publisher_key.clone(),
        "example/trusted".into(),
        ".github/workflows/release.yml".into(),
        2_000,
    )
    .unwrap();
    let unsafe_provenance = nivren::trust::attest_release(
        publisher_secret,
        &package,
        "../escape".into(),
        "example/trusted".into(),
        ".github/workflows/release.yml".into(),
        "0123456789abcdef".into(),
        1_000,
    )
    .unwrap();
    assert!(
        nivren::trust::verify_release(
            &package,
            &unsafe_provenance,
            &unsafe_authorization,
            &status,
            &[],
            root_public,
            1_100,
            0,
        )
        .unwrap_err()
        .message
        .contains("safe registry identifier")
    );

    let registry = module_fixture("public-registry-server");
    let trust = registry.join("v1/trust");
    fs::create_dir_all(&trust).unwrap();
    fs::write(trust.join("root.pub"), hex(&root_public)).unwrap();
    fs::write(
        trust.join("status.json"),
        serde_json::to_vec_pretty(&status).unwrap(),
    )
    .unwrap();
    fs::write(trust.join("advisories.json"), b"[]").unwrap();
    let envelope = nivren::trust::PublishEnvelope {
        package: package.clone(),
        provenance: provenance.clone(),
        authorization: authorization.clone(),
    }
    .encode()
    .unwrap();
    let mut request = format!(
        "POST /v1/publish HTTP/1.1\r\nHost: registry.test\r\nContent-Type: application/vnd.nivren.publish-v1\r\nContent-Length: {}\r\n\r\n",
        envelope.len()
    )
    .into_bytes();
    request.extend_from_slice(&envelope);
    let response = nivren::registry_server::handle_request_for_test(&request, &registry, 1_100, 1);
    assert!(response.starts_with(b"HTTP/1.1 201 Created\r\n"));
    let response = nivren::registry_server::handle_request_for_test(
        b"GET /v1/packages/trusted/1.0.0.nivpkg HTTP/1.1\r\nHost: registry.test\r\n\r\n",
        &registry,
        1_100,
        1,
    );
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
    assert!(response.ends_with(&package));
    let response = nivren::registry_server::handle_request_for_test(
        b"GET /v1/packages/../root.pub HTTP/1.1\r\nHost: registry.test\r\n\r\n",
        &registry,
        1_100,
        1,
    );
    assert!(response.starts_with(b"HTTP/1.1 404 Not Found\r\n"));

    let advisory = nivren::trust::sign_advisory(
        root_secret,
        nivren::trust::Advisory {
            id: "NIV-2026-0001".into(),
            package: "trusted".into(),
            affected_versions: BTreeSet::from(["1.0.0".into()]),
            severity: "high".into(),
            summary: "test incident".into(),
            withdrawn: false,
            signature: String::new(),
        },
    );
    assert!(
        nivren::trust::verify_release(
            &package,
            &provenance,
            &authorization,
            &status,
            &[advisory],
            root_public,
            1_100,
            1,
        )
        .unwrap_err()
        .message
        .contains("advisory")
    );

    let revoked = nivren::trust::sign_status(
        root_secret,
        nivren::trust::RegistryStatus {
            generation: 2,
            issued_at: 1_100,
            expires_at: 2_000,
            revoked_keys: BTreeSet::from([publisher_key]),
            frozen_packages: BTreeMap::new(),
            signature: String::new(),
        },
    );
    assert!(
        nivren::trust::verify_release(
            &package,
            &provenance,
            &authorization,
            &revoked,
            &[],
            root_public,
            1_100,
            1,
        )
        .unwrap_err()
        .message
        .contains("revoked")
    );
    fs::remove_dir_all(registry).unwrap();
    fs::remove_dir_all(project).unwrap();
}
