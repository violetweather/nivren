use nivren::runtime::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
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

fn nivren_string_contents(value: impl AsRef<str>) -> String {
    value
        .as_ref()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\r', "\\r")
        .replace('\n', "\\n")
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
fn edition_four_intent_grammar_checks_and_runs_in_both_engines() {
    let source = r#"
type UserId is Int

shape User holds {
    id is UserId
    name is String
    email is maybe String
} derives Json, Compare, Display, Validate

choice LookupProblem holds {
    case Missing
    case Invalid carries String
}

define add
takes {
    left is Int
    right is Int
}
gives Int
{
    give left + right
}

define checked_add
takes {
    left is Int
    right is Int
}
gives Int or String
needs Network within "example.test"
{
    give ok(add with {
        left set left
        right set right
    })
}

shape AddPlan holds {
    left is Int
    right is Int
}

prepare addition as AddPlan with {
    left set 20
    right set 22
}

change total is Int set perform addition.left
change total to total + addition.right

keep answer set choose checked_add with {
    left set total
    right set 0
} {
    case Ok carries value => value
    case Err carries problem => -1
}

when answer == 42 {
    show("intent")
} otherwise {
    show("wrong")
}

change visited is Int set 0
each value in [1, 2, 3] {
    change visited to visited + value
}
repeat while visited < 10 {
    change visited to visited + 1
}

answer + visited
"#;
    assert_eq!(eval_tree(source), Value::Int(52));
    assert_eq!(eval_vm(source), Value::Int(52));
}

#[test]
fn edition_four_diagnostics_name_the_intended_forms() {
    let errors = nivren::check("keep answer is Int = 42").unwrap_err();
    assert!(errors[0].message.contains("set"));

    let errors = nivren::check("prepare request as Request { value set 1 }").unwrap_err();
    assert!(errors[0].message.contains("with"));

    let errors = nivren::check("shape User holds { name is String } derives Magic").unwrap_err();
    assert!(errors[0].message.contains("unknown derive 'Magic'"));

    let errors = nivren::check(
        "define add takes { left is Int right is Int } gives Int { give left + right }\n\
         add with { right set 2 left set 1 }",
    )
    .unwrap_err();
    assert!(errors[0].message.contains("canonical order"));

    for (source, expected) in [
        ("type UserId U64", "is"),
        ("shape User name is String }", "holds"),
        ("choice State holds { case Failed carries }", "type"),
        ("prepare request Request with {}", "as"),
        ("5 through 2", "function"),
        (
            "define fetch needs Network within \"https://api.example.test/v1\" { give null }",
            "names a host",
        ),
        (
            "protocol Named { name(value: Self) gives String }",
            "define",
        ),
        (
            "shape User holds { name is String } adopt Named for User { name user_name }",
            "set",
        ),
        (
            "shape User holds { name is String } derives Json, Json",
            "more than once",
        ),
    ] {
        let errors = nivren::check(source).unwrap_err();
        assert!(
            errors.iter().any(|error| error.message.contains(expected)),
            "{source:?} did not explain {expected:?}: {errors:?}"
        );
    }
}

#[test]
fn edition_four_preserves_scoped_needs_as_checked_metadata() {
    let source = "define fetch gives String or String needs Network within \"api.example.test\" { give err(\"offline\") } expose { fetch }";
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    let nivren::ast::Stmt::Function {
        capability_needs, ..
    } = &program[0]
    else {
        panic!("expected function");
    };
    assert_eq!(capability_needs[0].capability, "Network");
    assert_eq!(
        capability_needs[0].boundary.as_deref(),
        Some("api.example.test")
    );
    let docs = nivren::documentation::generate("proof", "4.0.0-beta", &program);
    assert!(docs.contains("Network within \"api.example.test\""));

    let invalid = nivren::parser::parse(
        nivren::lexer::scan(
            "define fetch needs Network within \"https://api.example.test/v1\" { give null }",
        )
        .unwrap(),
    )
    .unwrap_err();
    assert!(invalid[0].message.contains("names a host"));
}

#[test]
fn edition_four_protocol_clauses_dispatch_in_both_engines() {
    let source = r#"
protocol Named {
    define name
    takes {
        value is Self
    }
    gives String
}

shape User holds {
    name is String
}

define user_name
takes {
    value is User
}
gives String {
    give value.name
}

adopt Named for User {
    name set user_name
}

keep user set User with {
    name set "Mira"
}
Named.name(user)
"#;
    assert_eq!(eval_tree(source), Value::String("Mira".into()));
    assert_eq!(eval_vm(source), Value::String("Mira".into()));
}

#[test]
fn edition_four_labeled_calls_preserve_names_and_validate_module_exports() {
    let parsed = nivren::parser::parse(
        nivren::lexer::scan(
            r#"define greet takes { name is String } gives String { give name }
               greet with { name set "Mira" }"#,
        )
        .unwrap(),
    )
    .unwrap();
    let nivren::ast::Stmt::Expression(nivren::ast::Expr::Call(_, _, labels, _)) = &parsed[1] else {
        panic!("expected labeled call");
    };
    assert_eq!(labels.as_deref(), Some(["name".to_string()].as_slice()));

    let body = nivren::parser::parse(
        nivren::lexer::scan("define greet takes { name is String } gives String { give name }")
            .unwrap(),
    )
    .unwrap();
    let span = nivren::ast::Span { line: 1, column: 1 };
    let module = nivren::ast::Stmt::Module {
        name: "people".into(),
        body,
        exports: vec!["greet".into()],
        span,
    };
    let call = |label: &str| {
        nivren::parser::parse(
            nivren::lexer::scan(&format!("people.greet with {{ {label} set \"Mira\" }}")).unwrap(),
        )
        .unwrap()
        .remove(0)
    };
    assert!(nivren::typecheck::check(&[module.clone(), call("name")]).is_ok());
    let errors = nivren::typecheck::check(&[module, call("person")]).unwrap_err();
    assert!(errors[0].message.contains("expects labeled values [name]"));
}

#[test]
fn edition_four_json_schema_labels_are_parseable_and_checked() {
    let source = r#"
shape Event holds {
    id is Int
} derives Json

std.json.decode with {
    schema set Event
    source set "{\"id\":42}"
}
"#;
    assert!(nivren::check(source).is_ok());
}

#[test]
fn edition_four_labels_may_reuse_contextual_keywords() {
    // `std.iter.range` declares a canonical first label named `start`, which is
    // also the structured-concurrency keyword. The labeled-call form must still
    // reach it, and `start` must keep spawning tasks in expression position.
    let ranged = r#"
keep counted set std.iter.range with { start set 0 end set 10 step set 2 }
show(counted)
"#;
    assert!(nivren::check(ranged).is_ok());

    let spawned = r#"
define first
gives Int
{
    give 20
}

keep value set wait start first
show(value)
"#;
    assert!(nivren::check(spawned).is_ok());

    let named = r#"
shape Window holds {
    start is Int
}

keep frame set Window with { start set 3 }
show(frame.start)
"#;
    assert!(nivren::check(named).is_ok());
}

#[test]
fn edition_four_derives_are_checked_and_gate_generated_operations() {
    let complete = r#"
shape Release holds {
    name is String
    build is Int
} derives Json, Compare, Display, Key, Validate, Binary, DatabaseRow, Arguments

keep first set Release with { name set "beta" build set 4 }
keep second set Release with { name set "beta" build set 4 }
assert(first == second, "Compare derive")
std.json.encode(first)
"#;
    assert!(nivren::check(complete).is_ok());
    assert_eq!(eval_tree(complete), eval_vm(complete));

    for derive in ["Json", "Compare", "Display", "Validate", "Binary"] {
        let source = format!("shape Unsafe holds {{ handle is NativeHandle }} derives {derive}");
        let errors = nivren::check(&source).unwrap_err();
        assert!(
            errors[0]
                .message
                .contains(&format!("derive {derive} does not support"))
        );
    }
    assert!(nivren::check("shape Row holds { values is [Int] } derives DatabaseRow").is_err());
    assert!(nivren::check("shape Cli holds { bytes is Bytes } derives Arguments").is_err());
    let key_errors = nivren::check("shape Id holds { value is Int } derives Key").unwrap_err();
    assert!(key_errors[0].message.contains("must also derive Compare"));

    let missing_json = r#"
shape Visible holds { value is Int } derives Display
std.json.encode(Visible with { value set 1 })
"#;
    let errors = nivren::check(missing_json).unwrap_err();
    assert!(errors[0].message.contains("must derive Json"));
}

#[test]
fn edition_four_generated_derive_methods_run_in_both_engines() {
    let source = r#"
shape Release holds {
    name is String
    build is Int
} derives Json, Compare, Display, Key, Validate, Binary, DatabaseRow, Arguments

define verify
gives String or String
{
    keep release set Release with { name set "beta" build set 4 }
    keep json set Release.to_json with { value set release } or give
    keep decoded set Release.from_json with { source set json } or give
    assert(Release.compare with { left set release right set decoded }, "compare")
    keep shown set Release.display with { value set decoded }
    keep key set Release.key with { value set decoded } or give
    keep checked set Release.validate with { value set decoded } or give
    keep bytes set Release.to_binary with { value set decoded } or give
    keep binary set Release.from_binary with { bytes set bytes } or give
    keep row set Release.from_row with { source set json } or give
    keep cli set Release.from_arguments with {
        arguments set ["--name=beta", "--build=4"]
    } or give
    assert(Release.compare with { left set binary right set row }, "binary and row")
    assert(Release.compare with { left set row right set cli }, "arguments")
    give ok(shown + ":" + key)
}

verify with {}
"#;
    assert_eq!(eval_tree(source), eval_vm(source));
    assert!(matches!(eval_vm(source), Value::Ok(_)));
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let restored = nivren::bundle::decode(&nivren::bundle::encode(&chunk).unwrap()).unwrap();
    assert_eq!(
        nivren::runtime::Interpreter::new()
            .run_bytecode(&restored)
            .unwrap(),
        eval_tree(source)
    );
}

#[test]
fn through_pipelines_values_into_readable_stages() {
    assert_eq!(
        eval(
            "define add takes { value is Int, amount is Int } gives Int { give value + amount }\n\
             define double takes { value is Int } gives Int { give value * 2 }\n\
             5 through add(3) through double"
        ),
        Value::Int(16)
    );
}

fn public_example_sources() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut paths: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "niv"))
        .collect();
    paths.sort();
    assert!(
        paths.len() >= 29,
        "expected the published example corpus, found {} files",
        paths.len()
    );
    paths
}

#[test]
fn public_edition_four_examples_all_type_check() {
    for path in public_example_sources() {
        let source = fs::read_to_string(&path).unwrap();
        nivren::check(&source)
            .unwrap_or_else(|errors| panic!("{} failed: {errors:?}", path.display()));
        let formatted = nivren::formatter::format(&source);
        assert_eq!(nivren::formatter::format(&formatted), formatted);
        nivren::check(&formatted)
            .unwrap_or_else(|errors| panic!("formatted {} failed: {errors:?}", path.display()));
    }
}

#[test]
fn public_examples_contain_no_edition_three_residue() {
    for path in public_example_sources() {
        let source = fs::read_to_string(&path).unwrap();
        for (index, line) in source.lines().enumerate() {
            let trimmed = line.trim_start();
            let residue = if trimmed.starts_with("keep ") || trimmed.starts_with("change ") {
                // Edition 4 binds with `set` and reassigns with `to`; a bare `=`
                // is Edition 3 spelling. `==` and `=>` are ordinary operators.
                trimmed.match_indices('=').any(|(at, _)| {
                    !trimmed[at..].starts_with("==")
                        && !trimmed[at..].starts_with("=>")
                        && !trimmed[..at].ends_with(['=', '!', '<', '>'])
                })
            } else {
                trimmed.contains("gives Result<")
            };
            assert!(
                !residue,
                "{}:{} still uses Edition 3 spelling: {trimmed}",
                path.display(),
                index + 1
            );
        }
    }
}

#[test]
fn edition_four_language_proof_programs_all_type_check() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proofs/edition4");
    for name in [
        "cli_automation.niv",
        "http_service.niv",
        "database_service.niv",
        "realtime_bot.niv",
        "concurrent_pipeline.niv",
        "native_binding.niv",
    ] {
        let path = root.join(name);
        let source = fs::read_to_string(&path).unwrap();
        nivren::check(&source)
            .unwrap_or_else(|errors| panic!("{} failed: {errors:?}", path.display()));
        let formatted = nivren::formatter::format(&source);
        assert_eq!(nivren::formatter::format(&formatted), formatted);
        nivren::check(&formatted)
            .unwrap_or_else(|errors| panic!("formatted {} failed: {errors:?}", path.display()));
    }
}

#[test]
fn edition_four_usability_corpus_stays_within_the_language_proof_budget() {
    let tasks = [
        (
            "keep answer is Int set 42\nanswer",
            "keep answer is Int set 42\nanswer",
        ),
        (
            "change count is Int set 0\ncount = count + 1\ncount",
            "change count is Int set 0\nchange count to count + 1\ncount",
        ),
        (
            "define add takes { left is Int, right is Int } gives Int { give left + right }\nadd(20, 22)",
            "define add takes { left is Int right is Int } gives Int { give left + right }\nadd with { left set 20 right set 22 }",
        ),
        (
            "shape User { name is String, active is Bool }\nUser(\"Mira\", yes).name",
            "shape User holds { name is String active is Bool }\nUser with { name set \"Mira\" active set yes }.name",
        ),
        (
            "choice State { Ready, Failed(String) }\nchoose State.Ready { case Ready => 1, case Failed carries problem => 0 }",
            "choice State holds { case Ready case Failed carries String }\nchoose State.Ready { case Ready => 1 case Failed carries problem => 0 }",
        ),
        (
            "keep name is String? set none\nname ?? \"guest\"",
            "keep name is maybe String set none\nname ?? \"guest\"",
        ),
        (
            "change total set 0\neach value within [1, 2, 3] { total = total + value }\ntotal",
            "change total set 0\neach value in [1, 2, 3] { change total to total + value }\ntotal",
        ),
        (
            "change count set 0\nrepeat count < 3 { count = count + 1 }\ncount",
            "change count set 0\nrepeat while count < 3 { change count to count + 1 }\ncount",
        ),
        (
            "define answer takes { } gives Result<Int, String> { give ok(42) }\nanswer()",
            "define answer gives Int or String { give ok(42) }\nanswer with {}",
        ),
        (
            "define double takes { value is Int } gives Int { give value * 2 }\n21 through double",
            "define double takes { value is Int } gives Int { give value * 2 }\n21 through double",
        ),
        (
            "shape Request { path is String, timeout is Float }\nkeep request set Request(\"/\", 5.0)\nrequest.path",
            "shape Request holds { path is String timeout is Float }\nprepare request as Request with { path set \"/\" timeout set 5.0 }\n(perform request).path",
        ),
        (
            "define greet takes { name is String } gives String { give \"hello \" + name }\ngreet(\"Nivren\")",
            "define greet takes { name is String } gives String { give \"hello \" + name }\ngreet with { name set \"Nivren\" }",
        ),
    ];
    let mut ratios = Vec::new();
    for (edition_three, edition_four) in tasks {
        nivren::check(edition_four).unwrap_or_else(|errors| panic!("{edition_four}: {errors:?}"));
        let old = nivren::lexer::scan(edition_three).unwrap().len() - 1;
        let new = nivren::lexer::scan(edition_four).unwrap().len() - 1;
        ratios.push(new as f64 / old as f64);
    }
    ratios.sort_by(f64::total_cmp);
    let median = (ratios[5] + ratios[6]) / 2.0;
    eprintln!("edition_four_median_token_ratio {median:.3}");
    assert!(
        median <= 1.15,
        "Edition 4 median token ratio {median:.3} exceeds the 1.15 language-proof budget"
    );
}

#[test]
fn edition_four_maintenance_corpus_reduces_ambiguous_choices() {
    #[derive(serde::Deserialize)]
    struct MaintenanceTask {
        task: String,
        edition3_steps: usize,
        edition4_steps: usize,
        edition3_ambiguous_choices: usize,
        edition4_ambiguous_choices: usize,
        evidence: String,
    }

    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proofs/edition4/maintenance-corpus.json");
    let tasks: Vec<MaintenanceTask> =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert!(tasks.len() >= 6);
    let mut old_choices = 0;
    let mut new_choices = 0;
    for task in tasks {
        assert!(!task.task.is_empty() && !task.evidence.is_empty());
        assert!(
            task.edition4_steps <= task.edition3_steps,
            "{} adds conceptual maintenance steps",
            task.task
        );
        assert!(
            task.edition4_ambiguous_choices <= task.edition3_ambiguous_choices,
            "{} adds ambiguous maintenance choices",
            task.task
        );
        old_choices += task.edition3_ambiguous_choices;
        new_choices += task.edition4_ambiguous_choices;
    }
    assert!(new_choices < old_choices);
}

#[test]
fn intent_concurrency_starts_waits_joins_and_races_scoped_tasks() {
    let source = r#"
define one takes { } gives Int { give 1 }
define two takes { } gives Int { give 2 }
keep joined set together [start one, start two]
keep first set race [start one, start two]
choose joined {
    case Ok carries values => choose first {
        case Ok carries value => values[0] + values[1] + value,
        case Err carries problem => 0,
    },
    case Err carries problem => 0
}
"#;
    let tree = eval_tree(source);
    let vm = eval_vm(source);
    assert!(matches!(tree, Value::Int(4) | Value::Int(5)));
    assert!(matches!(vm, Value::Int(4) | Value::Int(5)));
}

#[test]
fn scoped_locks_serialize_shared_updates_in_both_engines() {
    let source = r#"
define count takes { } gives Result<Int, String> needs Task {
    keep counter set std.locks.create(0)
    define increment takes { } gives Result<Null, String> needs Task {
        keep acquired set std.locks.acquire(counter, 2.0) or give
        using guard set acquired {
            keep current set std.locks.read(guard) or give
            keep written set std.locks.write(guard, current + 1) or give
            give ok(none)
        }
    }
    keep completed set together [start increment, start increment] or give
    keep acquired set std.locks.acquire(counter, 2.0) or give
    using guard set acquired {
        give std.locks.read(guard)
    }
}

choose count() { case Ok carries value => value, case Err carries problem => -1 }
"#;
    assert_eq!(eval_tree(source), Value::Int(2));
    assert_eq!(eval_vm(source), Value::Int(2));

    let closed = r#"
define inspect takes { } gives Result<Bool, String> needs Task {
    keep lock set std.locks.create("safe")
    keep guard set std.locks.acquire(lock, 1.0) or give
    keep first set std.locks.close(guard) or give
    keep second set std.locks.close(guard) or give
    give choose std.locks.read(guard) { case Ok carries value => ok(no), case Err carries problem => ok(yes) }
}
choose inspect() { case Ok carries value => value, case Err carries problem => no }
"#;
    assert_eq!(eval_tree(closed), Value::Bool(true));
    assert_eq!(eval_vm(closed), Value::Bool(true));

    let timeout = r#"
define blocked takes { } gives Result<Bool, String> needs Task {
    keep lock set std.locks.create(1)
    keep first set std.locks.acquire(lock, 1.0) or give
    keep second set std.locks.acquire(lock, 0.01)
    keep closed set std.locks.close(first) or give
    give choose second { case Ok carries guard => ok(no), case Err carries problem => ok(yes) }
}
choose blocked() { case Ok carries value => value, case Err carries problem => no }
"#;
    assert_eq!(eval_tree(timeout), Value::Bool(true));
    assert_eq!(eval_vm(timeout), Value::Bool(true));
}

#[test]
fn atomic_integers_are_linearizable_transferable_and_checked_in_both_engines() {
    let source = r#"
define count takes { } gives Result<Int, String> needs Task {
    keep counter set std.atomics.create(0)
    define increment takes { } gives Result<Null, String> {
        change index set 0
        repeat index < 250 {
            keep previous set std.atomics.add(counter, 1) or give
            index = index + 1
        }
        give ok(none)
    }
    keep completed set together [start increment, start increment, start increment, start increment] or give
    give ok(std.atomics.load(counter))
}

count()
"#;
    let expected = Value::Ok(Arc::new(Value::Int(1000)));
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);

    let operations = r#"
keep value set std.atomics.create(4)
keep old set std.atomics.swap(value, 5)
keep success set std.atomics.compare_exchange(value, 5, 9)
keep failure set std.atomics.compare_exchange(value, 5, 10)
choose failure { case Ok carries previous => -1, case Err carries observed => old + observed + std.atomics.load(value) }
"#;
    assert_eq!(eval_tree(operations), Value::Int(22));
    assert_eq!(eval_vm(operations), Value::Int(22));

    assert_eq!(
        eval_vm(
            "keep value set std.atomics.create(9223372036854775807) choose std.atomics.add(value, 1) { case Ok carries previous => no, case Err carries problem => yes }"
        ),
        Value::Bool(true)
    );
    assert!(nivren::check("std.atomics.store(std.atomics.create(0), 1.0)").is_err());
}

#[test]
fn transactions_commit_or_rollback_and_close_deterministically() {
    let commit = r#"
define update takes { } gives Result<Int, String> {
    keep original set std.map.of("count", 1)
    keep transaction is Transaction<String, Int> set std.transactions.create(original)
    keep changed set std.transactions.set(transaction, "count", 2) or give
    keep committed set std.transactions.commit(transaction) or give
    give ok(std.map.get(committed, "count") ?? 0)
}
choose update() { case Ok carries value => value, case Err carries problem => 0 }
"#;
    assert_eq!(eval_tree(commit), Value::Int(2));
    assert_eq!(eval_vm(commit), Value::Int(2));

    let rollback = r#"
define update takes { } gives Result<Int, String> {
    keep original set std.map.of("count", 1)
    keep transaction set std.transactions.create(original)
    keep changed set std.transactions.set(transaction, "count", 9) or give
    keep restored set std.transactions.rollback(transaction) or give
    give ok(std.map.get(restored, "count") ?? 0)
}
choose update() { case Ok carries value => value, case Err carries problem => 0 }
"#;
    assert_eq!(eval_tree(rollback), Value::Int(1));
    assert_eq!(eval_vm(rollback), Value::Int(1));

    let scoped = r#"
define abandon takes { transaction is Transaction<String, Int> } gives Result<Null, String> {
    using active set transaction {
        keep changed set std.transactions.set(active, "count", 7) or give
        give err("abandoned")
    }
}
keep transaction set std.transactions.create(std.map.of("count", 1))
keep abandoned set abandon(transaction)
keep closed set std.transactions.get(transaction, "count")
keep first_close set std.transactions.close(transaction)
keep second_close set std.transactions.close(transaction)
choose closed { case Ok carries value => no, case Err carries problem => yes }
"#;
    assert_eq!(eval_tree(scoped), Value::Bool(true));
    assert_eq!(eval_vm(scoped), Value::Bool(true));

    assert!(
        nivren::check(
            "std.transactions.set(std.transactions.create(std.map.of(1, \"a\")), \"wrong\", \"b\")"
        )
        .is_err()
    );
}

#[test]
fn native_handles_are_opaque_scoped_and_released_once_in_both_engines() {
    let source = r#"
define operate takes { } gives Result<String, String> needs Native {
    keep opened set std.host.open("database", "configuration") or give
    using handle set opened {
        give std.host.call(handle, "query", "select 42")
    }
}

operate()
"#;

    fn exercise_dynamic_libraries() {
        let directory = std::env::temp_dir().join(format!(
            "nivren-native-library-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("fixture.c");
        fs::write(
            &source,
            "#include <stdint.h>\n#include <stddef.h>\n\
         #ifdef _WIN32\n#define EXPORT __declspec(dllexport)\n#else\n#define EXPORT\n#endif\n\
         EXPORT int64_t nivren_add(int64_t left, int64_t right) { return left + right; }\n\
         EXPORT double nivren_mean(double left, double right) { return (left + right) / 2.0; }\n\
         EXPORT int64_t nivren_upper(const unsigned char *input, size_t length, unsigned char *output, size_t capacity) { if (capacity < length) return -2; for (size_t i = 0; i < length; i++) output[i] = input[i] >= 'a' && input[i] <= 'z' ? (unsigned char)(input[i] - 32) : input[i]; return (int64_t)length; }\n",
        )
        .unwrap();
        let library = if cfg!(windows) {
            let library = directory.join("fixture.dll");
            let output = Command::new("cl")
                .args(["/nologo", "/LD"])
                .arg(&source)
                .arg("/link")
                .arg(format!("/OUT:{}", library.display()))
                .current_dir(&directory)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "could not build native fixture: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            library
        } else {
            let library = directory.join(if cfg!(target_os = "macos") {
                "libfixture.dylib"
            } else {
                "libfixture.so"
            });
            let mut command = Command::new("cc");
            if cfg!(target_os = "macos") {
                command.arg("-dynamiclib");
            } else {
                command.args(["-shared", "-fPIC"]);
            }
            let output = command
                .arg(&source)
                .arg("-o")
                .arg(&library)
                .current_dir(&directory)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "could not build native fixture: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            library
        };
        let path = library
            .display()
            .to_string()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let integer = format!(
            r#"
define calculate takes {{ }} gives Result<Int, String> needs Native {{
    keep opened set std.native.open("{path}") or give
    using library set opened {{
        give std.native.call_int(library, "nivren_add", [20, 22])
    }}
}}
choose calculate() {{ case Ok carries value => value, case Err carries problem => -1 }}
"#
        );
        let float = format!(
            r#"
define calculate takes {{ }} gives Result<Float, String> needs Native {{
    keep opened set std.native.open("{path}") or give
    using library set opened {{
        give std.native.call_float(library, "nivren_mean", [1.5, 2.5])
    }}
}}
choose calculate() {{ case Ok carries value => value, case Err carries problem => -1.0 }}
"#
        );
        let closed = format!(
            r#"
define verify takes {{ }} gives Result<Bool, String> needs Native {{
    keep library set std.native.open("{path}") or give
    keep closed set std.native.close(library) or give
    give choose std.native.call_int(library, "nivren_add", [1, 2]) {{
        case Ok carries value => ok(no),
        case Err carries problem => ok(yes),
    }}
}}
choose verify() {{ case Ok carries value => value, case Err carries problem => no }}
"#
        );
        let buffer = format!(
            r#"
define transform takes {{ }} gives Result<String, String> needs Native {{
    keep opened set std.native.open("{path}") or give
    using library set opened {{
        keep output set std.native.call_buffer(library, "nivren_upper", std.bytes.from_string("Nivren"), 64) or give
        give std.bytes.to_string(output)
    }}
}}
choose transform() {{ case Ok carries value => value, case Err carries problem => problem }}
"#
        );
        for source in [&integer, &float, &buffer, &closed] {
            let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
            nivren::typecheck::check(&program).unwrap();
            let chunk = nivren::bytecode::compile(&program).unwrap();
            let tree = nivren::runtime::Interpreter::new().run(&program).unwrap();
            let bytecode = nivren::runtime::Interpreter::new()
                .run_bytecode(&chunk)
                .unwrap();
            assert_eq!(tree, bytecode);
        }
        assert_eq!(eval_tree(&integer), Value::Int(42));
        assert_eq!(eval_tree(&float), Value::Float(2.0));
        assert_eq!(eval_tree(&buffer), Value::String("NIVREN".into()));
        assert_eq!(eval_tree(&closed), Value::Bool(true));
        let program = nivren::parser::parse(nivren::lexer::scan(&integer).unwrap()).unwrap();
        let allowed = nivren::runtime::Interpreter::new()
            .with_capability_scopes([(
                String::from("Native"),
                format!("path:{}", directory.display()),
            )])
            .run(&program)
            .unwrap();
        assert_eq!(allowed, Value::Int(42));
        let denied = nivren::runtime::Interpreter::new()
            .with_capability_scopes([(
                String::from("Native"),
                String::from("path:/definitely/not/the/fixture"),
            )])
            .run(&program)
            .unwrap_err();
        assert!(denied.message.contains("outside the project grant"));
        fs::remove_dir_all(directory).unwrap();
    }
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    for bytecode in [false, true] {
        let events = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let captured = events.clone();
        let mut interpreter =
            nivren::runtime::Interpreter::new().with_host_callback(move |operation, request| {
                captured
                    .lock()
                    .unwrap()
                    .push((operation.to_string(), request.to_string()));
                match operation {
                    "nivren.handle.open:database" => Ok("handle-42".into()),
                    "nivren.handle.call:query" => Ok("row:42".into()),
                    "nivren.handle.close" => Ok("closed".into()),
                    _ => Err("unexpected operation".into()),
                }
            });
        let result = if bytecode {
            interpreter.run_bytecode(&chunk).unwrap()
        } else {
            interpreter.run(&program).unwrap()
        };
        assert_eq!(result, Value::Ok(Arc::new(Value::String("row:42".into()))));
        let events = events.lock().unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|(name, _)| name == "nivren.handle.close")
                .count(),
            1
        );
        let call = events
            .iter()
            .find(|(name, _)| name == "nivren.handle.call:query")
            .unwrap();
        let envelope: serde_json::Value = serde_json::from_str(&call.1).unwrap();
        assert_eq!(envelope["handle"], "handle-42");
        assert_eq!(envelope["request"], "select 42");
    }
    exercise_dynamic_libraries();
}

#[test]
fn native_handle_cleanup_retries_failures_and_survives_stress() {
    let retry = r#"
define close_retry takes { } gives Result<Bool, String> needs Native {
    keep handle set std.host.open("device", "configuration") or give
    keep first set std.host.close(handle)
    keep second set std.host.close(handle)
    give ok(choose first { case Ok carries value => no, case Err carries problem => choose second { case Ok carries value => yes, case Err carries problem => no } })
}
close_retry()
"#;
    let stress = r#"
define exercise takes { } gives Result<Int, String> needs Native {
    change index set 0
    repeat index < 1000 {
        keep opened set std.host.open("device", "configuration") or give
        using handle set opened { index = index + 1 }
    }
    give ok(index)
}
exercise()
"#;
    for bytecode in [false, true] {
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let captured = attempts.clone();
        let mut interpreter =
            nivren::runtime::Interpreter::new().with_host_callback(move |operation, _| {
                match operation {
                    "nivren.handle.open:device" => Ok("retry-handle".into()),
                    "nivren.handle.close"
                        if captured.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 =>
                    {
                        Err("injected close failure".into())
                    }
                    "nivren.handle.close" => Ok("closed".into()),
                    _ => Err("unexpected operation".into()),
                }
            });
        let program = nivren::parser::parse(nivren::lexer::scan(retry).unwrap()).unwrap();
        nivren::typecheck::check(&program).unwrap();
        let result = if bytecode {
            interpreter
                .run_bytecode(&nivren::bytecode::compile(&program).unwrap())
                .unwrap()
        } else {
            interpreter.run(&program).unwrap()
        };
        assert_eq!(result, Value::Ok(Arc::new(Value::Bool(true))));
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);

        let closes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let captured = closes.clone();
        let mut interpreter =
            nivren::runtime::Interpreter::new().with_host_callback(move |operation, _| {
                match operation {
                    "nivren.handle.open:device" => Ok("stress-handle".into()),
                    "nivren.handle.close" => {
                        captured.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok("closed".into())
                    }
                    _ => Err("unexpected operation".into()),
                }
            });
        let program = nivren::parser::parse(nivren::lexer::scan(stress).unwrap()).unwrap();
        nivren::typecheck::check(&program).unwrap();
        let result = if bytecode {
            interpreter
                .run_bytecode(&nivren::bytecode::compile(&program).unwrap())
                .unwrap()
        } else {
            interpreter.run(&program).unwrap()
        };
        assert_eq!(result, Value::Ok(Arc::new(Value::Int(1000))));
        assert_eq!(closes.load(std::sync::atomic::Ordering::SeqCst), 1000);
    }
}

#[test]
fn native_host_operations_join_as_bounded_structured_tasks_in_both_engines() {
    let source = r#"
define query takes { } gives Result<String, String> needs Native, Task {
    keep queued set std.host.invoke_async("device.read", "{\"port\":7}") or give
    give wait queued
}

query()
"#;
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    for bytecode in [false, true] {
        let mut interpreter =
            nivren::runtime::Interpreter::new().with_host_callback(|operation, request| {
                assert_eq!(operation, "device.read");
                assert_eq!(request, "{\"port\":7}");
                Ok("sample:42".into())
            });
        let value = if bytecode {
            interpreter.run_bytecode(&chunk).unwrap()
        } else {
            interpreter.run(&program).unwrap()
        };
        assert_eq!(
            value,
            Value::Ok(Arc::new(Value::String("sample:42".into())))
        );
    }

    assert!(
        nivren::check(r#"define missing takes { } { std.host.invoke_async("x", "y") }"#).is_err()
    );
    let no_host = nivren::run(
        r#"define missing takes { } gives Result<Task, String> needs Native, Task { give std.host.invoke_async("x", "y") } missing()"#,
    )
    .unwrap();
    assert!(matches!(no_host, Value::Err(_)));
}

#[test]
fn datetime_values_preserve_instants_and_iana_zones_in_both_engines() {
    let source = r#"
define render takes { } gives Result<String, String> {
    keep epoch set std.time.from_unix(0, "UTC") or give
    keep new_york set std.time.in_zone(epoch, "America/New_York") or give
    keep later set std.time.add_seconds(epoch, 3600) or give
    assert(later > epoch, "DateTime ordering follows the instant")
    keep parsed set std.time.parse("1970-01-01T09:00:00+09:00[Asia/Tokyo]") or give
    assert(parsed == epoch, "equal instants compare equally")
    assert(std.time.unix(later) == 3600, "Unix conversion preserves seconds")
    give ok(std.time.format(new_york))
}

choose render() { case Ok carries value => value, case Err carries problem => problem }
"#;
    let expected = Value::String("1969-12-31T19:00:00-05:00[America/New_York]".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);

    assert_eq!(
        eval_vm(
            "choose std.time.from_unix(0, \"Not/A_Zone\") { case Ok carries value => no, case Err carries problem => yes }",
        ),
        Value::Bool(true)
    );
}

#[test]
fn bigint_and_decimal_arithmetic_is_exact_checked_and_typed() {
    let source = r#"
define calculate takes { } gives Result<String, String> {
    keep huge set std.bigint.parse("1000000000000000000000000000000") or give
    keep two set std.bigint.from_int(2)
    keep exact set std.decimal.parse("0.1") or give
    keep more set std.decimal.parse("0.2") or give
    keep total set exact + more
    assert(std.decimal.format(total) == "0.3", "decimal addition stays exact")
    assert((huge + two) > huge, "BigInt ordering is exact")
    give ok(std.bigint.format(huge + two))
}

choose calculate() { case Ok carries value => value, case Err carries problem => problem }
"#;
    let expected = Value::String("1000000000000000000000000000002".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);

    assert!(
        nivren::run(
            "keep value set std.decimal.from_int(1) keep zero set std.decimal.from_int(0) value / zero"
        )
        .is_err()
    );
    let outside = r#"
define inspect takes { } gives Result<Bool, String> {
    keep huge set std.bigint.parse("999999999999999999999999") or give
    give choose std.bigint.to_int(huge) { case Ok carries value => ok(no), case Err carries problem => ok(yes) }
}
choose inspect() { case Ok carries value => value, case Err carries problem => no }
"#;
    assert_eq!(eval_vm(outside), Value::Bool(true));
}

#[test]
fn fixed_width_signed_and_unsigned_numbers_are_distinct_and_checked() {
    let source = r#"
define render takes { } gives Result<String, String> {
    keep first is U8 set std.u8.from_int(250) or give
    keep second is U8 set std.u8.from_int(5) or give
    keep maximum is U8 set first + second
    keep signed is I16 set std.i16.parse("-32000") or give
    keep zero is I16 set std.i16.from_int(0) or give
    keep wide is U64 set std.u64.parse("18446744073709551615") or give
    assert(signed < zero, "signed ordering")
    assert(choose std.u64.to_int(wide) { case Ok carries value => no, case Err carries problem => yes }, "U64 conversion is checked")
    give ok(std.u8.format(maximum) + ":" + std.u64.format(wide))
}
choose render() { case Ok carries value => value, case Err carries problem => problem }
"#;
    let expected = Value::String("255:18446744073709551615".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);

    let overflow = r#"
define overflow takes { } gives Result<U8, String> {
    keep left set std.u8.from_int(250) or give
    keep right set std.u8.from_int(6) or give
    give ok(left + right)
}
overflow()
"#;
    assert!(nivren::run(overflow).is_err());
    let program = nivren::parser::parse(nivren::lexer::scan(overflow).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    assert!(
        nivren::runtime::Interpreter::new()
            .run_bytecode(&chunk)
            .is_err()
    );
    assert!(nivren::check(
        "define mixed takes { } gives Result<U8, String> { keep left set std.u8.from_int(1) or give keep right set std.i8.from_int(1) or give give ok(left + right) }"
    )
    .is_err());
    assert!(nivren::check(
        "define negative takes { } gives Result<U8, String> { keep value set std.u8.from_int(1) or give give ok(-value) }"
    )
    .is_err());
}

#[test]
fn capability_needs_are_explicit_and_transitive() {
    let direct = nivren::check(
        "define load takes { path is String } gives Result<String, String> { give std.files.read(path) }",
    )
    .unwrap_err();
    assert!(direct[0].message.contains("needs FileRead"));

    nivren::check(
        "define load takes { path is String } gives Result<String, String> needs FileRead { give std.files.read(path) }",
    )
    .unwrap();

    let transitive = nivren::check(
        "define load takes { path is String } gives Result<String, String> needs FileRead { give std.files.read(path) }\n\
         define config takes { } gives Result<String, String> { give load(\"app.json\") }",
    )
    .unwrap_err();
    assert!(transitive[0].message.contains("needs FileRead"));

    let spawned = nivren::check(
        "define worker takes { } gives Null needs Channel { give none }\n\
         define launch takes { } gives Task needs Task { give start worker }",
    )
    .unwrap_err();
    assert!(
        spawned
            .iter()
            .any(|error| error.message.contains("needs Channel")),
        "{spawned:?}"
    );
}

#[test]
fn intent_wait_awaits_a_started_task() {
    let source = "define answer takes { } gives Int { give 42 } keep task set start answer keep result set wait task choose result { case Ok carries value => value, case Err carries problem => 0 }";
    assert_eq!(eval_tree(source), Value::Int(42));
    assert_eq!(eval_vm(source), Value::Int(42));
}

#[test]
fn immutable_bindings_reject_assignment() {
    let errors = nivren::run("keep answer set 42; answer = 7;").unwrap_err();
    assert!(errors[0].message.contains("immutable"));
}

#[test]
fn mutable_bindings_and_loops_work() {
    assert_eq!(
        eval("change n set 0; repeat (n < 5) { n = n + 1; } n"),
        Value::Int(5)
    );
}

#[test]
fn functions_return_values() {
    assert_eq!(
        eval("define add takes { a is Int , b is Int } { give a + b; } add(20, 22)"),
        Value::Int(42)
    );
}

#[test]
fn generic_functions_infer_reuse_and_check_type_parameters() {
    let source = "define identity<Value> takes { value is Value } gives Value { give value } keep number is Int set identity(42) keep text is String set identity(\"nivren\") number";
    assert_eq!(eval_tree(source), Value::Int(42));
    assert_eq!(eval_vm(source), Value::Int(42));

    let arrays = "define first<Element> takes { values is [Element] } gives Element { give values[0] } keep answer is Int set first([42, 7]) answer";
    assert_eq!(eval_vm(arrays), Value::Int(42));

    let mixed = nivren::check(
        "define same<Value> takes { left is Value, right is Value } gives Value { give left } same(1, \"two\")",
    )
    .unwrap_err();
    assert!(
        mixed
            .iter()
            .any(|error| error.message.contains("generic argument"))
    );
}

#[test]
fn generic_protocols_make_constraints_visible_and_checkable() {
    let source = "define add<Value is Number> takes { left is Value, right is Value } gives Value { give left + right } keep integer is Int set add(20, 22) keep decimal is Float set add(1.5, 2.5) integer";
    assert_eq!(eval_vm(source), Value::Int(42));

    let rejected = nivren::check(
        "define add<Value is Number> takes { left is Value, right is Value } gives Value { give left + right } add(\"a\", \"b\")",
    )
    .unwrap_err();
    assert!(
        rejected
            .iter()
            .any(|error| error.message.contains("does not satisfy Number"))
    );

    nivren::check(
        "define entry<Key is Comparable, Value> takes { key is Key, value is Value } gives Map<Key, Value> { give std.map.of(key, value) } keep item is Map<String, Int> set entry(\"answer\", 42)",
    )
    .unwrap();
    assert!(nivren::check(
        "define entry<Key, Value> takes { key is Key, value is Value } gives Map<Key, Value> { give std.map.of(key, value) }",
    )
    .is_err());
    assert!(
        nivren::check(
            "define bad<Value is Magical> takes { value is Value } gives Value { give value }",
        )
        .is_err()
    );
}

#[test]
fn user_marker_protocols_are_explicit_coherent_and_dual_engine() {
    let source = r#"
protocol Identified
shape User { id is Int, name is String }
adopt Identified for User

define preserve<Value is Identified> takes { value is Value } gives Value {
    give value
}

keep user set User(7, "Mira")
preserve(user)
"#;
    let expected = eval_tree("shape User { id is Int, name is String } User(7, \"Mira\")");
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);

    let missing = nivren::check(
        r#"
protocol Identified
shape User { id is Int }
shape Project { id is Int }
adopt Identified for User
define preserve<Value is Identified> takes { value is Value } gives Value { give value }
preserve(Project(2))
"#,
    )
    .unwrap_err();
    assert!(
        missing
            .iter()
            .any(|error| error.message.contains("does not satisfy Identified"))
    );

    let duplicate = nivren::check(
        "protocol Tagged shape Item { id is Int } adopt Tagged for Item adopt Tagged for Item",
    )
    .unwrap_err();
    assert!(
        duplicate
            .iter()
            .any(|error| error.message.contains("already adopted"))
    );

    let sealed = nivren::check("adopt Comparable for String").unwrap_err();
    assert!(
        sealed
            .iter()
            .any(|error| error.message.contains("sealed protocol"))
    );
}

#[test]
fn protocol_members_are_required_and_dispatch_coherently_in_both_engines() {
    let source = r#"
protocol Rendered {
    define render takes { value is Self } gives String
}
shape User { name is String }
define render_user takes { value is User } gives String { give value.name }
adopt Rendered for User { render set render_user }

define present<Value is Rendered> takes { value is Value } gives String {
    give Rendered.render(value)
}
present(User("Mira"))
"#;
    assert_eq!(eval_tree(source), Value::String("Mira".into()));
    assert_eq!(eval_vm(source), Value::String("Mira".into()));
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    let bundle = nivren::bundle::encode(&nivren::bytecode::compile(&program).unwrap()).unwrap();
    let decoded = nivren::bundle::decode(&bundle).unwrap();
    assert_eq!(
        nivren::runtime::Interpreter::new()
            .run_bytecode(&decoded)
            .unwrap(),
        Value::String("Mira".into())
    );

    assert!(nivren::check(
        "protocol Named { define name takes { value is Self } gives String } shape User { name is String } adopt Named for User"
    )
    .is_err());
    assert!(nivren::check(
        "protocol Named { define name takes { value is Self } gives String } shape User { name is String } define wrong takes { value is User } gives Int { give 1 } adopt Named for User { name set wrong }"
    )
    .is_err());
    assert!(
        nivren::check("protocol Named { define name takes { value is Int } gives String }")
            .is_err()
    );
    assert!(nivren::check(
        "protocol Logged { define emit takes { value is Self } gives Null needs Log } shape Event { text is String } define emit_event takes { value is Event } gives Null needs Log { std.log.info(value.text) } adopt Logged for Event { emit set emit_event } define hidden<Value is Logged> takes { value is Value } { Logged.emit(value) }"
    )
    .is_err());
    assert!(nivren::check(
        "protocol Tagged shape Box<Value> { value is Value } adopt Tagged for Box<Int> adopt Tagged for Box<String>"
    )
    .is_err());
}

#[test]
fn closures_capture_scope() {
    assert_eq!(
        eval(
            "define outer takes { x is Int } { define inner takes { y is Int } { give x + y; } give inner; } keep add2 set outer(2); add2(40)"
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
    let errors = nivren::check("keep x set @;").unwrap_err();
    assert_eq!((errors[0].line, errors[0].column), (1, 12));
}

#[test]
fn parser_rejects_excessive_nesting_without_exhausting_the_stack() {
    let source = format!("{}1", "-".repeat(4_096));
    let errors = nivren::parser::parse(nivren::lexer::scan(&source).unwrap()).unwrap_err();
    assert!(errors[0].message.contains("nesting"));
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
        "match (value) { case One => 1 }",
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
    let errors =
        nivren::check("define pair takes { a is Int , b is Int } { give a; } pair(1)").unwrap_err();
    assert!(errors[0].message.contains("expects 2"));
}

#[test]
fn immutable_arrays_support_safe_indexing() {
    assert_eq!(
        eval("keep values set [10, 20, 30]; values[1]"),
        Value::Int(20)
    );
    assert_eq!(
        eval("keep values set append([1, 2], 3); len(values)"),
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
    let source = "define add takes { a is Int, b is Int } gives Int { give a + b; } keep answer is Int set add(20, 22); answer";
    assert_eq!(eval(source), Value::Int(42));
    assert!(nivren::check("keep value is String set 42").is_err());
    assert!(nivren::check("define bad takes { } gives Bool { give 1; }").is_err());
    assert!(
        nivren::check(
            "define onlyInt takes { value is Int } gives Int { give value; } onlyInt(yes)"
        )
        .is_err()
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
        eval("keep missing is String? set none; missing ?? \"fallback\""),
        Value::String("fallback".into())
    );
    assert_eq!(
        eval("keep present is String? set \"value\"; present ?? \"fallback\""),
        Value::String("value".into())
    );
    assert!(nivren::check("keep invalid is String set none").is_err());
    assert!(nivren::check("keep plain is Int set 1; plain ?? 2").is_err());
}

#[test]
fn records_are_nominal_typed_values() {
    let source = "shape Person { name is String, age is Int } keep ada is Person set Person(\"Ada\", 37); ada.name";
    assert_eq!(eval(source), Value::String("Ada".into()));
    assert!(nivren::check("shape Person { name is String } keep person set Person(42)").is_err());
    assert!(
        nivren::check(
            "shape Person { name is String } keep person set Person(\"Ada\"); person.missing"
        )
        .is_err()
    );
}

#[test]
fn safe_reflection_inspects_shape_values_without_vm_internals() {
    let source = r#"
choice Role { Admin, Member }
shape User { name is String, active is Bool }
define inspect takes { } gives Result<String, String> {
    keep user set User("Mira", yes)
    keep fields set std.reflect.fields(user) or give
    keep shape_schema set std.reflect.schema(User) or give
    keep choice_schema set std.reflect.schema(Role) or give
    give ok(
        (std.map.get(fields, "name") ?? "")
        + ":" + (std.map.get(fields, "active") ?? "")
        + ":" + std.reflect.kind(user)
        + ":" + (std.map.get(shape_schema, "$kind") ?? "")
        + ":" + (std.map.get(shape_schema, "name") ?? "")
        + ":" + (std.map.get(choice_schema, "$kind") ?? "")
        + ":" + (std.map.get(choice_schema, "Member") ?? "")
    )
}
choose inspect() {
    case Ok carries value => value,
    case Err carries problem => problem
}
"#;
    let expected = Value::String("String:Bool:User:shape:String:choice:1".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
    assert!(matches!(eval("std.reflect.fields(42)"), Value::Err(_)));
    assert!(matches!(eval("std.reflect.schema(42)"), Value::Err(_)));
}

#[test]
fn sealed_enums_require_exhaustive_matches() {
    let source = "choice State { Idle, Running, Done } keep state is State set State.Running; choose (state) { case Idle => 0, case Running => 1, case Done => 2 }";
    assert_eq!(eval(source), Value::Int(1));
    assert!(
        nivren::check(
            "choice State { Idle, Done } keep state set State.Idle; choose (state) { case Idle => 0 }"
        )
        .is_err()
    );
    assert!(nivren::check("choice State { Idle } keep state set State.Missing").is_err());
}

#[test]
fn choices_carry_typed_recursive_payloads_in_both_engines() {
    let source = r#"
choice Response {
    Text(String),
    Number(Int),
    Array([Response]),
    Nil
}
define score takes { value is Response } gives Int {
    give choose value {
        case Text carries text => len(text),
        case Number carries number => number,
        case Array carries items => len(items),
        case Nil => 0
    }
}
score(Response.Array([Response.Text("ok"), Response.Number(5)]))
"#;
    assert_eq!(eval_tree(source), Value::Int(2));
    assert_eq!(eval_vm(source), Value::Int(2));
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    let compiled = nivren::bytecode::compile(&program).unwrap();
    let encoded = nivren::bundle::encode(&compiled).unwrap();
    let decoded = nivren::bundle::decode(&encoded).unwrap();
    assert_eq!(
        nivren::runtime::Interpreter::new()
            .run_bytecode(&decoded)
            .unwrap(),
        Value::Int(2)
    );
    let json = r#"choice Value { Text(String), Nil } std.json.encode(Value.Text("ok"))"#;
    let expected_json = Value::Ok(Arc::new(Value::String(
        r#"{"$value":"ok","$variant":"Text"}"#.into(),
    )));
    assert_eq!(eval_tree(json), expected_json);
    assert_eq!(eval_vm(json), expected_json);

    assert!(
        nivren::check("choice Value { Text(String) } keep value is Value set Value.Text").is_err()
    );
    assert!(nivren::check("choice Value { Text(String) } Value.Text(42)").is_err());
    assert!(nivren::check("choice Value { Nil } Value.Nil(1)").is_err());
    assert!(nivren::check(
        "choice Value { Text(String) } keep value set Value.Text(\"ok\") choose value { case Text => 1 }"
    )
    .is_err());
}

#[test]
fn shapes_and_choices_are_generic_nominal_and_inferred() {
    let source = r#"
shape Pair<Left, Right> { left is Left, right is Right }
choice Maybe<Value> { Some(Value), None }

define unwrap takes { value is Maybe<Int> } gives Int {
    give choose value { case Some carries item => item, case None => 0 }
}

keep pair is Pair<String, Int> set Pair("age", 42)
keep present is Maybe<Int> set Maybe.Some(pair.right)
keep absent is Maybe<Int> set Maybe.None
unwrap(present) + unwrap(absent)
"#;
    assert_eq!(eval_tree(source), Value::Int(42));
    assert_eq!(eval_vm(source), Value::Int(42));

    assert!(nivren::check(
        "shape Pair<Left, Right> { left is Left, right is Right } keep pair is Pair<Int, String> set Pair(1, 2)"
    )
    .is_err());
    assert!(
        nivren::check(
            "choice Maybe<Value> { Some(Value), None } keep value is Maybe<String> set Maybe.Some(1)"
        )
        .is_err()
    );
    assert!(
        nivren::check("shape Box<Value> { value is Value } keep value is Box set Box(1)").is_err()
    );
    assert!(
        nivren::check("shape Keyed<Key is Comparable> { key is Key } Keyed(std.iter.from([1]))")
            .is_err()
    );
    assert!(nivren::check(
        "shape Keyed<Key is Comparable> { key is Key } define invalid takes { value is Keyed<Iterator<Int>> } { show(value) }"
    )
    .is_err());
}

#[test]
fn for_iteration_is_typed_and_unicode_safe() {
    assert_eq!(
        eval(
            "change total is Int set 0; each (value within [1, 2, 3]) { total = total + value; } total"
        ),
        Value::Int(6)
    );
    assert_eq!(
        eval(
            "change count is Int set 0; each (character within \"a💡c\") { count = count + 1; } count"
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
    let source = "define parse takes { valid is Bool } gives Result<Int, String> { when (valid) { give ok(42); } give err(\"invalid\"); } keep result is Result<Int, String> set parse(yes); choose (result) { case Ok carries value => value, case Err carries message => 0 }";
    assert_eq!(eval(source), Value::Int(42));
    assert!(
        nivren::check(
            "keep result is Result<Int, String> set ok(1); choose (result) { case Ok carries value => value }"
        )
        .is_err()
    );
    assert!(nivren::check("keep result is Result<Int, String> set err(\"bad\"); choose (result) { case Ok => 1, case Err carries error => 0 }").is_err());
}

#[test]
fn or_give_propagates_typed_failures_through_nested_expressions() {
    let program = r#"
define parse takes { valid is Bool } gives Result<Int, String> {
    when valid { give ok(41) }
    give err("invalid")
}
define answer takes { valid is Bool } gives Result<Int, String> {
    keep value is Int set parse(valid) or give
    give ok(value + 1)
}
answer(yes)
"#;
    assert_eq!(eval_tree(program), Value::Ok(Arc::new(Value::Int(42))));
    assert_eq!(eval_vm(program), Value::Ok(Arc::new(Value::Int(42))));
    let parsed = nivren::parser::parse(nivren::lexer::scan(program).unwrap()).unwrap();
    nivren::typecheck::check(&parsed).unwrap();
    let chunk = nivren::bytecode::compile(&parsed).unwrap();
    let encoded = nivren::bundle::encode(&chunk).unwrap();
    let decoded = nivren::bundle::decode(&encoded).unwrap();
    assert_eq!(
        nivren::runtime::Interpreter::new()
            .run_bytecode(&decoded)
            .unwrap(),
        Value::Ok(Arc::new(Value::Int(42)))
    );

    let failure = program.replace("answer(yes)", "answer(no)");
    assert_eq!(
        eval_tree(&failure),
        Value::Err(Arc::new(Value::String("invalid".into())))
    );
    assert_eq!(eval_vm(&failure), eval_tree(&failure));

    let nested = "define value takes { } gives Result<Int, String> { give ok(20) } define answer takes { } gives Result<Int, String> { give ok((value() or give) * 2 + 2) } answer()";
    assert_eq!(eval_tree(nested), Value::Ok(Arc::new(Value::Int(42))));
    assert_eq!(eval_vm(nested), Value::Ok(Arc::new(Value::Int(42))));

    assert!(nivren::check("ok(1) or give").is_err());
    assert!(nivren::check("define bad takes { } gives Int { give ok(1) or give }").is_err());
    assert!(nivren::check(
        "define bad takes { } gives Result<Int, Int> { keep value set err(\"wrong\") or give give ok(value) }",
    )
    .is_err());
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
        "define double takes { value is Int } gives Int { give value * 2; } keep private set 7; expose { double };",
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
        nivren::runtime::Interpreter::new()
            .with_instruction_limit(10_000_000)
            .run(&program)
            .unwrap(),
        Value::Int(42)
    );

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn module_members_are_private_unless_exported() {
    let directory = module_fixture("private-modules");
    fs::write(
        directory.join("secrets.niv"),
        "keep visible set 1; keep hidden set 2; expose { visible };",
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
        "shape Box { value is Int } define read takes { box is Box } gives Int { give box.value; } expose { Box, read };",
    )
    .unwrap();
    fs::write(
        directory.join("strings.niv"),
        "shape Box { value is String } expose { Box };",
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
fn module_protocol_adoptions_follow_qualified_types_without_collisions() {
    let directory = module_fixture("protocol-modules");
    fs::write(
        directory.join("people.niv"),
        "protocol Identified shape User { id is Int } adopt Identified for User define preserve<Value is Identified> takes { value is Value } gives Value { give value } expose { User, preserve }",
    )
    .unwrap();
    let entry = directory.join("main.niv");
    fs::write(
        &entry,
        "use \"people.niv\" people.preserve(people.User(9)).id",
    )
    .unwrap();

    let program = nivren::modules::load(&entry).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let expected = Value::Int(9);
    assert_eq!(
        nivren::runtime::Interpreter::new().run(&program).unwrap(),
        expected
    );
    let chunk = nivren::bytecode::compile(&program).unwrap();
    assert_eq!(
        nivren::runtime::Interpreter::new()
            .with_instruction_limit(10_000_000)
            .run_bytecode(&chunk)
            .unwrap(),
        expected
    );

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
    assert!(manifest.capabilities.is_empty());
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
fn project_capabilities_are_explicit_validated_and_runtime_enforced() {
    let root = PathBuf::from("/tmp/capability-example");
    let source = "[package]\nname = \"capability-app\"\nversion = \"1.0.0\"\nentry = \"main.niv\"\n\n[capabilities]\nFileRead = \"allow\"\n";
    let manifest = nivren::project::Manifest::parse(source, root.clone()).unwrap();
    assert!(manifest.capabilities.contains("FileRead"));
    assert!(manifest.capability_scopes.is_empty());
    assert!(
        nivren::project::Manifest::parse(&source.replace("FileRead", "Everything"), root.clone())
            .is_err()
    );
    assert!(
        nivren::project::Manifest::parse(&source.replace("\"allow\"", "\"deny\""), root).is_err()
    );

    let program = nivren::parser::parse(
        nivren::lexer::scan("std.files.exists(\"definitely-missing\")").unwrap(),
    )
    .unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let denied = nivren::runtime::Interpreter::new()
        .with_capabilities(Vec::<String>::new())
        .run_bytecode(&chunk)
        .unwrap_err();
    assert!(denied.message.contains("does not allow FileRead"));
    let allowed = nivren::runtime::Interpreter::new()
        .with_capabilities(vec!["FileRead".to_string()])
        .run_bytecode(&chunk)
        .unwrap();
    assert_eq!(allowed, Value::Ok(Value::Bool(false).into()));

    let directory = module_fixture("scoped-capabilities");
    let allowed_file = directory.join("allowed.txt");
    fs::write(&allowed_file, "ok").unwrap();
    let scoped_source = source.replace(
        "FileRead = \"allow\"",
        &format!("FileRead = \"path:{}\"", directory.display()),
    );
    let scoped = nivren::project::Manifest::parse(&scoped_source, directory.clone()).unwrap();
    assert_eq!(
        scoped.capability_scopes.get("FileRead"),
        Some(&format!("path:{}", directory.display()))
    );
    assert_eq!(
        nivren::project::Manifest::parse(&scoped.source(), directory.clone())
            .unwrap()
            .capability_scopes,
        scoped.capability_scopes
    );

    let allowed_path = nivren_string_contents(allowed_file.to_string_lossy());
    let source = format!("std.files.exists(\"{allowed_path}\")");
    let program = nivren::parser::parse(nivren::lexer::scan(&source).unwrap()).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let inside = nivren::runtime::Interpreter::new()
        .with_capabilities(vec!["FileRead".into()])
        .with_capability_scopes(scoped.capability_scopes.clone())
        .run_bytecode(&chunk)
        .unwrap();
    assert_eq!(inside, Value::Ok(Value::Bool(true).into()));

    let outside_source = "std.files.exists(\"/definitely-outside-nivren-scope\")";
    let outside_program =
        nivren::parser::parse(nivren::lexer::scan(outside_source).unwrap()).unwrap();
    let outside = nivren::runtime::Interpreter::new()
        .with_capabilities(vec!["FileRead".into()])
        .with_capability_scopes(scoped.capability_scopes)
        .run(&outside_program)
        .unwrap_err();
    assert!(outside.message.contains("outside the project grant"));

    let network = nivren::parser::parse(
        nivren::lexer::scan("std.net.connect(\"example.com\", 80, 0.01)").unwrap(),
    )
    .unwrap();
    let denied_host = nivren::runtime::Interpreter::new()
        .with_capabilities(vec!["Network".into()])
        .with_capability_scopes([(String::from("Network"), String::from("host:localhost"))])
        .run(&network)
        .unwrap_err();
    assert!(denied_host.message.contains("outside the project grant"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn advanced_capability_scopes_limit_environment_process_and_native_kinds() {
    let root = module_fixture("advanced-scopes");
    let manifest = nivren::project::Manifest::parse(
        "[package]\nname = \"scopes\"\nversion = \"1.0.0\"\nentry = \"src/main.niv\"\n\n[capabilities]\nEnvironment = \"prefix:NIVREN_\"\nProcess = \"command:nivren-command-that-does-not-exist\"\nNative = \"kind:database\"\n",
        root.clone(),
    )
    .unwrap();
    assert_eq!(manifest.capability_scopes["Environment"], "prefix:NIVREN_");
    assert_eq!(
        manifest.capability_scopes["Process"],
        "command:nivren-command-that-does-not-exist"
    );
    assert_eq!(manifest.capability_scopes["Native"], "kind:database");

    let allowed_process = "std.process.run(\"nivren-command-that-does-not-exist\", [])";
    let program = nivren::parser::parse(nivren::lexer::scan(allowed_process).unwrap()).unwrap();
    let value = nivren::runtime::Interpreter::new()
        .with_capability_scopes(manifest.capability_scopes.clone())
        .run(&program)
        .unwrap();
    assert!(matches!(value, Value::Err(_)));
    let denied_process = nivren::parser::parse(
        nivren::lexer::scan("std.process.run(\"different-command\", [])").unwrap(),
    )
    .unwrap();
    assert!(
        nivren::runtime::Interpreter::new()
            .with_capability_scopes(manifest.capability_scopes.clone())
            .run(&denied_process)
            .unwrap_err()
            .message
            .contains("outside the project grant")
    );

    let environment =
        nivren::parser::parse(nivren::lexer::scan("std.env.get(\"PATH\")").unwrap()).unwrap();
    assert!(
        nivren::runtime::Interpreter::new()
            .with_capability_scopes(manifest.capability_scopes.clone())
            .run(&environment)
            .unwrap_err()
            .message
            .contains("outside the project grant")
    );

    let native = nivren::parser::parse(
        nivren::lexer::scan("std.host.open(\"database\", \"configuration\")").unwrap(),
    )
    .unwrap();
    let value = nivren::runtime::Interpreter::new()
        .with_capability_scopes(manifest.capability_scopes)
        .with_host_callback(|operation, _| match operation {
            "nivren.handle.open:database" => Ok("handle".into()),
            "nivren.handle.close" => Ok("closed".into()),
            _ => Err("unexpected operation".into()),
        })
        .run(&native)
        .unwrap();
    assert!(matches!(value, Value::Ok(_)));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn composed_capability_scopes_constrain_hosts_methods_commands_and_arguments() {
    let root = module_fixture("composed-scopes");
    let manifest = nivren::project::Manifest::parse(
        "[package]\nname = \"composed\"\nversion = \"1.0.0\"\nentry = \"src/main.niv\"\n\n[capabilities]\nNetwork = \"host:127.0.0.1,localhost;method:GET\"\nProcess = \"command:nivren-command-that-does-not-exist;arg0:status\"\n",
        root.clone(),
    )
    .unwrap();

    let allowed_web = nivren::parser::parse(
        nivren::lexer::scan(
            "std.web.request(\"GET\", \"http://127.0.0.1:1/status\", std.web.headers(), \"\", 0.01, 64)",
        )
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        nivren::runtime::Interpreter::new()
            .with_capability_scopes(manifest.capability_scopes.clone())
            .run(&allowed_web)
            .unwrap(),
        Value::Err(_)
    ));
    let denied_method = nivren::parser::parse(
        nivren::lexer::scan(
            "std.web.request(\"POST\", \"http://127.0.0.1:1/status\", std.web.headers(), \"\", 0.01, 64)",
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        nivren::runtime::Interpreter::new()
            .with_capability_scopes(manifest.capability_scopes.clone())
            .run(&denied_method)
            .unwrap_err()
            .message
            .contains("outside the project grant")
    );

    let allowed_process = nivren::parser::parse(
        nivren::lexer::scan(
            "std.process.run(\"nivren-command-that-does-not-exist\", [\"status\"])",
        )
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        nivren::runtime::Interpreter::new()
            .with_capability_scopes(manifest.capability_scopes.clone())
            .run(&allowed_process)
            .unwrap(),
        Value::Err(_)
    ));
    let denied_argument = nivren::parser::parse(
        nivren::lexer::scan(
            "std.process.run(\"nivren-command-that-does-not-exist\", [\"version\"])",
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        nivren::runtime::Interpreter::new()
            .with_capability_scopes(manifest.capability_scopes)
            .run(&denied_argument)
            .unwrap_err()
            .message
            .contains("outside the project grant")
    );
    assert!(nivren::project::Manifest::parse(
        "[package]\nname = \"bad\"\nversion = \"1.0.0\"\nentry = \"main.niv\"\n\n[capabilities]\nNetwork = \"method:GET\"\n",
        root.clone(),
    )
    .is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn instruction_limits_stop_runaway_code_and_round_trip_in_projects() {
    let root = PathBuf::from("/tmp/limited-example");
    let source = "[package]\nname = \"limited\"\nversion = \"1.0.0\"\nentry = \"main.niv\"\n\n[limits]\ninstructions = \"1000\"\nmemory_bytes = \"4096\"\n";
    let manifest = nivren::project::Manifest::parse(source, root.clone()).unwrap();
    assert_eq!(manifest.instruction_limit, Some(1000));
    assert_eq!(manifest.memory_limit, Some(4096));
    let reparsed = nivren::project::Manifest::parse(&manifest.source(), root.clone()).unwrap();
    assert_eq!(reparsed.instruction_limit, Some(1000));
    assert_eq!(reparsed.memory_limit, Some(4096));
    assert!(
        nivren::project::Manifest::parse(&source.replace("\"1000\"", "\"0\""), root.clone())
            .is_err()
    );
    assert!(
        nivren::project::Manifest::parse(&source.replace("instructions", "seconds"), root).is_err()
    );

    let program =
        nivren::parser::parse(nivren::lexer::scan("repeat yes { none }").unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let tree_error = nivren::runtime::Interpreter::new()
        .with_instruction_limit(100)
        .run(&program)
        .unwrap_err();
    assert!(tree_error.message.contains("instruction limit"));
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let vm_error = nivren::runtime::Interpreter::new()
        .with_instruction_limit(100)
        .run_bytecode(&chunk)
        .unwrap_err();
    assert!(vm_error.message.contains("instruction limit"));

    let recursive = nivren::parser::parse(
        nivren::lexer::scan("define recurse takes { } gives Int { give recurse() } recurse()")
            .unwrap(),
    )
    .unwrap();
    nivren::typecheck::check(&recursive).unwrap();
    let tree_depth = nivren::runtime::Interpreter::new()
        .with_call_depth_limit(32)
        .run(&recursive)
        .unwrap_err();
    assert!(tree_depth.message.contains("call depth limit"));
    let recursive_chunk = nivren::bytecode::compile(&recursive).unwrap();
    let vm_depth = nivren::runtime::Interpreter::new()
        .with_call_depth_limit(32)
        .run_bytecode(&recursive_chunk)
        .unwrap_err();
    assert!(vm_depth.message.contains("call depth limit"));

    let allocating = nivren::parser::parse(
        nivren::lexer::scan("\"abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz\"").unwrap(),
    )
    .unwrap();
    let tree_memory = nivren::runtime::Interpreter::new()
        .with_memory_limit(32)
        .run(&allocating)
        .unwrap_err();
    assert!(tree_memory.message.contains("memory limit"));
    let allocating_chunk = nivren::bytecode::compile(&allocating).unwrap();
    let vm_memory = nivren::runtime::Interpreter::new()
        .with_memory_limit(32)
        .run_bytecode(&allocating_chunk)
        .unwrap_err();
    assert!(vm_memory.message.contains("memory limit"));
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
    assert!(
        nivren::project::Manifest::parse(&source.replace("alpha", "bad-name"), root.clone())
            .is_err()
    );

    let mut updated = manifest.clone();
    updated.add_dependency("middle", "3.4.5").unwrap();
    let reparsed = nivren::project::Manifest::parse(&updated.source(), root.clone()).unwrap();
    assert_eq!(reparsed.dependencies["middle"], "3.4.5");
    assert!(updated.add_dependency("bad-name", "1.0.0").is_err());
}

#[test]
fn unified_project_commands_create_develop_test_ship_and_add() {
    let parent = module_fixture("project-commands");
    let project = parent.join("sample-app");
    let niv = env!("CARGO_BIN_EXE_niv");

    for arguments in [
        vec!["new".to_string(), project.display().to_string()],
        vec!["dev".to_string(), project.display().to_string()],
        vec!["bench".to_string(), project.display().to_string()],
        vec![
            "test".to_string(),
            project.join("tests/niv").display().to_string(),
        ],
        vec!["ship".to_string(), project.display().to_string()],
    ] {
        let output = std::process::Command::new(niv)
            .args(&arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "command {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(project.join("target/sample-app-0.1.0.nivpkg").is_file());
    assert!(project.join("target/doc/api.md").is_file());
    let standalone = project.join(if cfg!(windows) {
        "target/sample-app.exe"
    } else {
        "target/sample-app"
    });
    assert!(standalone.is_file());
    let executed = std::process::Command::new(&standalone).output().unwrap();
    assert!(
        executed.status.success(),
        "standalone failed: {}",
        String::from_utf8_lossy(&executed.stderr)
    );
    assert!(String::from_utf8_lossy(&executed.stdout).contains("Welcome to Nivren"));

    let native_build = std::process::Command::new(niv)
        .args([
            "build",
            "--standalone",
            "--native",
            project.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        native_build.status.success(),
        "native standalone failed: {}",
        String::from_utf8_lossy(&native_build.stderr)
    );
    let embedded = nivren::standalone::extract(&fs::read(&standalone).unwrap()).unwrap();
    assert!(
        embedded
            .manifest
            .starts_with("# nivren-standalone-engine = native\n")
    );
    let executed = std::process::Command::new(&standalone).output().unwrap();
    assert!(executed.status.success());
    assert!(String::from_utf8_lossy(&executed.stdout).contains("Welcome to Nivren"));

    let project_text = project.display().to_string();
    let added = std::process::Command::new(niv)
        .args(["add", "answerlib", "1.2.3", &project_text])
        .output()
        .unwrap();
    assert!(added.status.success());
    let manifest = nivren::project::Manifest::load(&project).unwrap();
    assert_eq!(manifest.dependencies["answerlib"], "1.2.3");

    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn cli_test_profiles_and_deterministic_time_are_explicit() {
    let project = module_fixture("test-profiles");
    let tests = project.join("tests/niv");
    fs::create_dir_all(&tests).unwrap();
    fs::write(
        project.join("niv.toml"),
        "[package]\nname = \"test-profiles\"\nversion = \"1.0.0\"\nentry = \"main.niv\"\n\n[capabilities]\nTime = \"allow\"\n",
    )
    .unwrap();
    fs::write(project.join("main.niv"), "42").unwrap();
    fs::write(
        tests.join("clock_test.niv"),
        "assert with { condition set perform std.time.monotonic with { } == 1700000000.25 message set \"fixed test time\" }",
    )
    .unwrap();
    let niv = env!("CARGO_BIN_EXE_niv");
    let timed = Command::new(niv)
        .args(["test", "--time", "1700000000.25", tests.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        timed.status.success(),
        "deterministic test failed: {}",
        String::from_utf8_lossy(&timed.stderr)
    );
    fs::write(
        tests.join("clock_test.niv"),
        "assert with { condition set yes message set \"profile dispatch\" }",
    )
    .unwrap();
    for profile in ["--property", "--compat", "--fuzz-smoke"] {
        let output = Command::new(niv)
            .args(["test", profile, tests.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{profile} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::remove_dir_all(project).unwrap();
}

#[test]
fn vscode_extension_registers_the_nivren_debug_adapter() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(repository.join("editors/vscode/package.json")).unwrap())
            .unwrap();
    let debugger = &manifest["contributes"]["debuggers"][0];
    assert_eq!(debugger["type"], "nivren");
    assert_eq!(debugger["languages"][0], "nivren");
    assert_eq!(debugger["initialConfigurations"][0]["request"], "launch");
    let extension = fs::read_to_string(repository.join("editors/vscode/src/extension.ts")).unwrap();
    assert!(extension.contains("registerDebugAdapterDescriptorFactory"));
    assert!(extension.contains("new vscode.DebugAdapterExecutable(this.executable, [\"dap\"])"));
}

#[test]
fn workspace_commands_build_test_and_reuse_incremental_members() {
    let root = module_fixture("workspace-commands");
    for name in ["core", "app"] {
        let member = root.join(name);
        fs::create_dir_all(member.join("src")).unwrap();
        fs::create_dir_all(member.join("tests/niv")).unwrap();
        fs::write(
            member.join("niv.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"src/main.niv\"\n"
            ),
        )
        .unwrap();
        fs::write(member.join("src/main.niv"), "42").unwrap();
        fs::write(
            member.join("tests/niv/value_test.niv"),
            "assert(6 * 7 == 42, \"workspace member\")",
        )
        .unwrap();
    }
    fs::write(
        root.join("niv-workspace.toml"),
        "[workspace]\nmembers = \"core, app\"\n",
    )
    .unwrap();
    let niv = env!("CARGO_BIN_EXE_niv");
    for action in ["build", "build", "test"] {
        let output = Command::new(niv)
            .args(["workspace", action, root.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "workspace {action} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if action == "build" {
            assert!(String::from_utf8_lossy(&output.stdout).contains(
                if root.join("core/target/.nivren-fingerprint").exists() {
                    "core"
                } else {
                    "built"
                }
            ));
        }
    }
    assert!(root.join("core/target/core.nivb").is_file());
    assert!(root.join("app/target/app.nivb").is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_projects_receive_the_bounded_builtin_sqlite_host() {
    let root = module_fixture("sqlite-cli-host");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("niv.toml"),
        "[package]\nname = \"sqlite-app\"\nversion = \"1.0.0\"\nentry = \"src/main.niv\"\n\n[capabilities]\nNative = \"kind:database\"\n",
    )
    .unwrap();
    fs::write(
        root.join("src/main.niv"),
        r#"define run
gives String or String
needs Native within "database"
{
    keep opened set perform std.host.open with { kind set "database" request set "memory://cli-proof" } or give
    using handle set opened {
        keep created set perform std.host.call with { handle set handle name set "execute" request set "{\"operation\":\"execute\",\"statement\":\"CREATE TABLE users (name TEXT NOT NULL)\",\"parameters\":[],\"maximum_rows\":0,\"timeout\":5.0}" } or give
        keep inserted set perform std.host.call with { handle set handle name set "execute" request set "{\"operation\":\"execute\",\"statement\":\"INSERT INTO users (name) VALUES (?)\",\"parameters\":[\"Ada\"],\"maximum_rows\":0,\"timeout\":5.0}" } or give
        give perform std.host.call with { handle set handle name set "query" request set "{\"operation\":\"query\",\"statement\":\"SELECT name FROM users\",\"parameters\":[],\"maximum_rows\":10,\"timeout\":5.0}" }
    }
}

show(choose perform run with {} {
    case Ok carries response => response
    case Err carries problem => problem
})
"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_niv"))
        .args(["run", root.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Ada"));
    fs::remove_dir_all(root).unwrap();
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
        "define answer takes { } gives Int { give 42; } expose { answer };",
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
    fs::remove_file(app.join("niv.lock")).unwrap();
    fs::remove_dir_all(&registry).unwrap();
    assert_eq!(
        nivren::package::install_offline_dependencies(&app_manifest).unwrap(),
        1
    );
    assert_eq!(
        fs::read_to_string(app.join("niv.lock")).unwrap(),
        expected_lock
    );
    assert!(app.join("niv.authority.lock").is_file());
    let authority_check = Command::new(env!("CARGO_BIN_EXE_niv"))
        .args(["authority", "check", app.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(authority_check.status.success());
    let changed_manifest =
        fs::read_to_string(app.join("niv.toml")).unwrap() + "\n[capabilities]\nTime = \"allow\"\n";
    fs::write(app.join("niv.toml"), changed_manifest).unwrap();
    let stale_authority = Command::new(env!("CARGO_BIN_EXE_niv"))
        .args(["authority", "check", app.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!stale_authority.status.success());
    let report = Command::new(env!("CARGO_BIN_EXE_niv"))
        .args(["authority", "report", app.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(report.status.success());
    assert!(String::from_utf8_lossy(&report.stdout).contains("capability = \"Time\""));
    let relocked = Command::new(env!("CARGO_BIN_EXE_niv"))
        .args(["authority", "lock", app.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(relocked.status.success());
    assert!(
        Command::new(env!("CARGO_BIN_EXE_niv"))
            .args(["authority", "check", app.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
    let program = nivren::modules::load_project(&app, &app.join("main.niv")).unwrap();
    nivren::typecheck::check(&program).unwrap();
    assert_eq!(
        nivren::runtime::Interpreter::new().run(&program).unwrap(),
        Value::Int(42)
    );

    let unused = nivren::package::Package {
        name: "unused".into(),
        version: "1.0.0".into(),
        files: BTreeMap::from([
            (
                "niv.toml".into(),
                b"[package]\nname = \"unused\"\nversion = \"1.0.0\"\nentry = \"main.niv\"\n"
                    .to_vec(),
            ),
            ("main.niv".into(), b"42".to_vec()),
        ]),
    };
    let unused_archive = unused.encode().unwrap();
    let unused_root = app.join(".niv/deps/unused-1.0.0");
    unused.extract(&unused_root).unwrap();
    fs::write(unused_root.join(".niv-package"), &unused_archive).unwrap();
    fs::write(
        unused_root.join(".niv-package-sha256"),
        hex(&Sha256::digest(&unused_archive)),
    )
    .unwrap();
    let entries = nivren::package::cache_entries(&app_manifest).unwrap();
    assert_eq!(entries.len(), 2);
    assert!(
        entries
            .iter()
            .any(|entry| entry.name == "answerlib" && entry.reachable)
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.name == "unused" && !entry.reachable)
    );
    let listed = Command::new(env!("CARGO_BIN_EXE_niv"))
        .args(["cache", "list", app.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(listed.status.success());
    assert!(String::from_utf8_lossy(&listed.stdout).contains("unused"));
    let pruned = Command::new(env!("CARGO_BIN_EXE_niv"))
        .args(["cache", "prune", app.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(pruned.status.success());
    assert!(!unused_root.exists());
    assert!(app.join(".niv/deps/answerlib-1.0.0").exists());

    fs::write(
        app.join(".niv/deps/answerlib-1.0.0/main.niv"),
        "expose { answer }; keep answer set 0;",
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
    fs::write(&outside, "keep secret set 42;").unwrap();
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
    let source = "define main takes { } {\nkeep text set \"{not a block}\" // }\n/* { nested /* } */ ok */\nwhen yes {\nshow(text)\n}\n}\n";
    let formatted = nivren::formatter::format(source);
    assert!(formatted.contains("    keep text set \"{not a block}\" // }"));
    assert!(formatted.contains("    /* { nested /* } */ ok */"));
    assert_eq!(nivren::formatter::format(&formatted), formatted);
}

#[test]
fn edition_four_formatter_canonicalizes_spacing_without_moving_comments() {
    let source = r#"shape   User   holds {
name   is   String // public label
}
define   greet
takes {
user   is   User
}
gives   String
{
keep   text is String set user.name
give   text
}
greet   with { user   set   User with { name set "Mira" } }
"#;
    let expected = r#"shape User holds {
    name is String // public label
}
define greet
takes {
    user is User
}
gives String {
    keep text is String set user.name
    give text
}
greet with {
    user set User with {
        name set "Mira"
    }
}
"#;
    let formatted = nivren::formatter::format(source);
    assert_eq!(formatted, expected);
    assert_eq!(nivren::formatter::format(&formatted), formatted);
    assert!(nivren::check(&formatted).is_ok());
}

#[test]
fn edition_four_formatter_erases_equivalent_line_layout_choices() {
    let compact = r#"shape User holds { name is String } derives Json define greet takes { user is User } gives String { give user.name } greet with { user set User with { name set "Mira" } }"#;
    let vertical = r#"
shape User holds {
    name is String
} derives Json

define greet
takes {
    user is User
}
gives String
{
    give user.name
}

greet with {
    user set User with {
        name set "Mira"
    }
}
"#;
    let compact = nivren::formatter::format(compact);
    let vertical = nivren::formatter::format(vertical);
    assert_eq!(compact, vertical);
    assert_eq!(nivren::formatter::format(&compact), compact);
    assert!(nivren::check(&compact).is_ok());
}

#[test]
fn documentation_lists_only_explicit_module_exports() {
    let source = "define public takes { value is Int } gives Int { give value; } keep hidden set 1; expose { public };";
    let parsed = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    let module = nivren::ast::Stmt::Module {
        name: "sample".into(),
        body: parsed[..2].to_vec(),
        exports: vec!["public".into()],
        span: nivren::ast::Span { line: 1, column: 1 },
    };
    let docs = nivren::documentation::generate("package", "1.0.0", &[module]);
    assert!(docs.contains("define public takes { value is Int } gives Int"));
    assert!(!docs.contains("hidden"));
}

#[test]
fn documentation_lists_entry_module_public_api() {
    let source = "define public takes { value is Int } gives Int { give value } keep hidden set 1 expose { public }";
    let parsed = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    let docs = nivren::documentation::generate("entry", "1.0.0", &parsed);
    assert!(docs.contains("## Public API"));
    assert!(docs.contains("define public takes { value is Int } gives Int"));
    assert!(!docs.contains("hidden"));
}

#[test]
fn documentation_includes_declared_capabilities() {
    let source = "define read takes { path is String } gives Result<String, String> needs FileRead { give std.files.read(path) } expose { read }";
    let parsed = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    let module = nivren::ast::Stmt::Module {
        name: "files".into(),
        body: parsed[..1].to_vec(),
        exports: vec!["read".into()],
        span: nivren::ast::Span { line: 1, column: 1 },
    };
    let docs = nivren::documentation::generate("package", "1.0.0", &[module]);
    assert!(docs.contains("needs FileRead"));
}

#[test]
fn documentation_preserves_generic_function_signatures() {
    let source = "define identity<Value is Comparable> takes { value is Value } gives Value { give value } expose { identity }";
    let parsed = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    let module = nivren::ast::Stmt::Module {
        name: "generic".into(),
        body: parsed[..1].to_vec(),
        exports: vec!["identity".into()],
        span: nivren::ast::Span { line: 1, column: 1 },
    };
    let docs = nivren::documentation::generate("package", "1.0.0", &[module]);
    assert!(
        docs.contains("define identity<Value is Comparable> takes { value is Value } gives Value")
    );
}

#[test]
fn bytecode_is_versioned_verified_and_deterministic() {
    let source = "define sum takes { limit is Int } gives Int { change total set 0; change index set 0; repeat (index < limit) { total = total + index; index = index + 1; } give total; } sum(5)";
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    nivren::bytecode::verify(&chunk).unwrap();
    let first = nivren::bytecode::disassemble(&chunk);
    assert_eq!(first, nivren::bytecode::disassemble(&chunk));
    assert!(first.starts_with("NIVB 8\n"));

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
fn source_maps_are_stable_nested_and_exportable() {
    let source = "define answer takes { } gives Int { give 42 }\nanswer()";
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let first = nivren::bytecode::source_map(&chunk, "main.niv");
    let second = nivren::bytecode::source_map(&chunk, "main.niv");
    assert_eq!(first, second);
    let parsed: serde_json::Value = serde_json::from_str(&first).unwrap();
    assert_eq!(parsed["schema"], "org.nivren.sourcemap.v1");
    assert_eq!(parsed["bytecodeVersion"], 8);
    assert!(
        parsed["mappings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|mapping| mapping["path"]
                .as_str()
                .is_some_and(|path| path.contains('.')))
    );

    let directory = module_fixture("source-map-cli");
    let source_path = directory.join("main.niv");
    let output_path = directory.join("main.niv.map.json");
    fs::write(&source_path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_niv"))
        .args([
            "sourcemap",
            source_path.to_str().unwrap(),
            output_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exported: serde_json::Value =
        serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
    assert_eq!(exported["source"], source_path.to_str().unwrap());
    fs::remove_dir_all(directory).unwrap();
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
    let source = "shape Pair { left is Int, right is Int } choice Choice { First, Second } define pick takes { value is Choice } gives Int { give choose value { case First => 1, case Second => 2 }; } change total set 0; each value within [10, 20] { total = total + value; } keep pair set Pair(total, pick(Choice.Second)); pair.left + pair.right";
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
fn cli_live_inspection_streams_privacy_safe_versioned_json_lines() {
    let directory = module_fixture("live-inspection");
    let source = directory.join("inspect.niv");
    let output = directory.join("events.jsonl");
    fs::write(
        &source,
        "keep secret set \"hidden-value\"\nkeep answer set 40 + 2\nanswer\n",
    )
    .unwrap();
    let inspected = Command::new(env!("CARGO_BIN_EXE_niv"))
        .arg("inspect")
        .arg(&source)
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        inspected.status.success(),
        "inspection failed: {}",
        String::from_utf8_lossy(&inspected.stderr)
    );
    let jsonl = fs::read_to_string(&output).unwrap();
    assert!(!jsonl.contains("hidden-value"));
    let events = jsonl
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(events.len() >= 3);
    assert_eq!(events[0]["schema"], "org.nivren.inspect.v1");
    assert_eq!(events[0]["kind"], "started");
    assert_eq!(events.last().unwrap()["kind"], "finished");
    assert_eq!(events.last().unwrap()["status"], "ok");
    assert!(events.last().unwrap()["instructions"].as_u64().unwrap() > 0);
    let steps = events
        .iter()
        .filter(|event| event["kind"] == "step")
        .collect::<Vec<_>>();
    assert!(!steps.is_empty());
    assert!(steps.iter().all(|event| event.get("variables").is_none()));
    assert!(steps.iter().any(|event| {
        event["variable_names"]
            .as_array()
            .is_some_and(|names| names.iter().any(|name| name == "secret"))
    }));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn schema_bindgen_cli_emits_c_and_cpp_compatible_views() {
    let directory = std::env::temp_dir().join(format!(
        "nivren-bindgen-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::create_dir_all(&directory).unwrap();
    let schema = directory.join("messages.niv");
    let header = directory.join("messages.h");
    fs::write(
        &schema,
        "choice Role { Admin, Member }\n\
         shape Address { city is String, postal is U32 }\n\
         shape User { name is String, role is Role, address is Address?, tags is [String] }\n",
    )
    .unwrap();
    let generated = Command::new(env!("CARGO_BIN_EXE_niv"))
        .args(["bindgen", "c"])
        .arg(&schema)
        .arg(&header)
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "bindgen failed: {}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let consumer = directory.join("consumer.c");
    let ffi_include = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("crates/nivren-ffi/include");
    fs::write(
        &consumer,
        "#include \"messages.h\"\n\
         #include \"nivren.h\"\n\
         int main(void) { Nivren_User user = {0}; NivrenAsyncRun *run = 0; nivren_async_run_cancel(run); return (int)user.role; }\n",
    )
    .unwrap();
    let cpp_consumer = directory.join("consumer.cpp");
    fs::write(
        &cpp_consumer,
        "#include \"messages.h\"\n\
         #include \"nivren.h\"\n\
         int main() { Nivren_User user{}; NivrenAsyncRun *run = nullptr; nivren_async_run_cancel(run); return static_cast<int>(user.role); }\n",
    )
    .unwrap();
    if cfg!(windows) {
        for (source, language) in [(&consumer, "/TC"), (&cpp_consumer, "/TP")] {
            let include = format!("/I{}", ffi_include.display());
            let output = Command::new("cl")
                .args(["/nologo", "/W4", "/WX", "/Zs", language])
                .arg(include)
                .arg(source)
                .current_dir(&directory)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "MSVC rejected generated bindings: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    } else {
        for (compiler, standard, source) in [
            ("cc", "-std=c11", &consumer),
            ("c++", "-std=c++17", &cpp_consumer),
        ] {
            let output = Command::new(compiler)
                .args([standard, "-Wall", "-Wextra", "-Werror", "-fsyntax-only"])
                .arg("-I")
                .arg(&ffi_include)
                .arg(source)
                .current_dir(&directory)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{compiler} rejected generated bindings: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    fs::remove_dir_all(directory).unwrap();
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
    let source = "define make takes { base is Int } { define add takes { value is Int } gives Int { give base + value; } give add; } keep escaped set make(40); change index set 0; repeat (index < 100) { define temporary takes { } gives Int { give index; } temporary(); index = index + 1; } escaped(2)";
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
        "keep writeResult is Result<Null, String> set std.files.write(\"{path}\", \"hello\"); assert(choose (writeResult) {{ case Ok carries value => yes, case Err carries error => no }}, \"write\"); keep readResult is Result<String, String> set std.files.read(\"{path}\"); keep text set choose (readResult) {{ case Ok carries value => value, case Err carries error => error }}; assert(choose (std.files.exists(\"{path}\")) {{ case Ok carries present => present, case Err carries error => no }}, \"exists\"); assert((std.path.basename(\"{path}\") ?? \"\") == \"message.txt\", \"basename\"); std.time.sleep(0.0); text"
    );
    assert_eq!(eval_vm(&source), Value::String("hello".into()));

    let process = "keep result is Result<String, String> set std.process.run(\"nivren-command-that-does-not-exist-4f3d\", []); choose (result) { case Ok carries output => no, case Err carries error => yes }";
    assert_eq!(eval_vm(process), Value::Bool(true));
    assert!(nivren::check("std.files.read(42)").is_err());

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

    let program = "keep result is Result<String, String> set std.json.compact(\"{\\\"ok\\\": true}\"); choose result { case Ok carries value => value, case Err carries error => error }";
    assert_eq!(eval_vm(program), Value::String("{\"ok\":true}".into()));
}

#[test]
fn json_values_round_trip_through_nivren_collections() {
    let source = r#"
keep decoded set std.json.parse("{\"name\":\"Nivren\",\"stable\":true,\"versions\":[2,3]}")
choose decoded {
    case Ok carries value => std.json.encode(value),
    case Err carries problem => err(problem)
}
"#;
    let expected = Value::Ok(Arc::new(Value::String(
        r#"{"name":"Nivren","stable":true,"versions":[2,3]}"#.into(),
    )));
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);

    assert_eq!(
        eval("std.json.encode(std.set.of(1))"),
        Value::Err(Arc::new(Value::String("JSON cannot represent Set".into())))
    );
}

#[test]
fn shapes_are_typed_json_schemas_in_both_engines() {
    let source = r#"
choice Role { Admin, Member }
shape Address { city is String, postal is U32 }

shape User {
    name is String,
    score is U8,
    tags is [String],
    alias is String?,
    address is Address,
    role is Role,
}

define display_name takes { source is String } gives Result<String, String> {
    keep user set std.json.decode(User, source) or give
    give ok(user.name)
}

keep decoded set std.json.decode(User, "{\"name\":\"Ada\",\"score\":255,\"tags\":[\"compiler\"],\"alias\":null,\"address\":{\"city\":\"London\",\"postal\":12345},\"role\":\"Admin\"}")
choose decoded {
    case Ok carries user => std.json.encode(user),
    case Err carries problem => err(problem),
}
"#;
    let expected = Value::Ok(Arc::new(Value::String(
        r#"{"address":{"city":"London","postal":12345},"alias":null,"name":"Ada","role":"Admin","score":255,"tags":["compiler"]}"#.into(),
    )));
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);

    for invalid in [
        r#"{"name":"Ada","score":256,"tags":[],"alias":null,"address":{"city":"X","postal":1},"role":"Admin"}"#,
        r#"{"name":"Ada","score":1,"tags":[],"address":{"city":"X","postal":1},"role":"Admin"}"#,
        r#"{"name":"Ada","score":1,"tags":[],"alias":null,"address":{"city":"X"},"role":"Admin"}"#,
        r#"{"name":"Ada","score":1,"tags":[],"alias":null,"address":{"city":"X","postal":1},"role":"Owner"}"#,
        r#"{"name":"Ada","score":1,"tags":[],"alias":null,"address":{"city":"X","postal":1},"role":"Admin","admin":true}"#,
    ] {
        let program = format!(
            "choice Role {{ Admin, Member }}\n\
             shape Address {{ city is String, postal is U32 }}\n\
             shape User {{ name is String, score is U8, tags is [String], alias is String?, address is Address, role is Role }}\n\
             keep decoded set std.json.decode(User, {invalid:?})\n\
             choose decoded {{ case Ok carries value => no, case Err carries problem => yes }}"
        );
        assert_eq!(eval_tree(&program), Value::Bool(true));
        assert_eq!(eval_vm(&program), Value::Bool(true));
    }
}

#[test]
fn json_lines_stream_with_a_bounded_record_buffer() {
    let path = std::env::temp_dir().join(format!(
        "nivren-json-lines-{}-{}.ndjson",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(&path, "{\"id\":1}\n{\"id\":2}\n").unwrap();
    let source = format!(
        r#"
shape Item {{ id is Int }}
define load takes {{ path is String }} gives Result<String, String> needs FileRead {{
    keep opened set std.files.open_read(path) or give
    using file set opened {{
        keep first set std.json.read_next_as(Item, file, 64) or give
        keep second set std.json.read_next_as(Item, file, 64) or give
        keep ending set std.json.read_next_as(Item, file, 64) or give
        assert(ending == none, "stream reaches end")
        keep first_item set first ?? Item(0)
        keep second_item set second ?? Item(0)
        assert(first_item.id + second_item.id == 3, "typed streamed fields")
        give std.json.encode([first_item, second_item])
    }}
}}
load({:?})
"#,
        path.to_string_lossy()
    );
    let expected = Value::Ok(Arc::new(Value::String("[{\"id\":1},{\"id\":2}]".into())));
    assert_eq!(eval_tree(&source), expected);
    assert_eq!(eval_vm(&source), expected);

    fs::write(&path, "{\"payload\":\"too long\"}\n{\"id\":3}\n").unwrap();
    let recovery = format!(
        r#"
define recover takes {{ path is String }} gives Result<String, String> needs FileRead {{
    keep opened set std.files.open_read(path) or give
    using file set opened {{
        keep rejected set std.json.read_next(file, 8)
        keep recovered set choose rejected {{
            case Ok carries value => err("oversized record accepted"),
            case Err carries problem => std.json.read_next(file, 64),
        }}
        keep next set recovered or give
        give std.json.encode(next)
    }}
}}
recover({:?})
"#,
        path.to_string_lossy()
    );
    let recovered = Value::Ok(Arc::new(Value::String("{\"id\":3}".into())));
    assert_eq!(eval_tree(&recovery), recovered);
    assert_eq!(eval_vm(&recovery), recovered);
    fs::remove_file(path).unwrap();
}

#[test]
fn file_line_iterators_are_lazy_bounded_and_recover_after_errors() {
    let path = std::env::temp_dir().join(format!(
        "nivren-lines-{}-{}.txt",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::write(&path, "alpha\nway-too-long\nomega\r\n").unwrap();
    let source = format!(
        r#"
define load takes {{ path is String }} gives Result<Bool, String> needs FileRead {{
    keep opened set std.files.open_read(path) or give
    using file set opened {{
        keep lines set std.iter.lines(file, 8) or give
        keep first set std.iter.next(lines) ?? err("missing first line")
        keep oversized set std.iter.next(lines) ?? ok("missing")
        keep third set std.iter.next(lines) ?? err("missing third line")
        keep ending set std.iter.next(lines)
        keep first_text set first or give
        keep third_text set third or give
        keep rejected set choose oversized {{ case Ok carries value => no, case Err carries problem => yes }}
        give ok(first_text == "alpha" and third_text == "omega" and rejected and ending == none)
    }}
}}
load({:?})
"#,
        path.to_string_lossy()
    );
    let expected = Value::Ok(Arc::new(Value::Bool(true)));
    assert_eq!(eval_tree(&source), expected);
    assert_eq!(eval_vm(&source), expected);
    fs::remove_file(path).unwrap();
}

#[test]
fn immutable_bytes_round_trip_unicode_and_check_bounds() {
    let source = r#"
keep data is Bytes set std.bytes.from_string("Nivren 🜁")
keep length is Int set std.bytes.length(data)
keep sliced set std.bytes.slice(data, 0, 6)
keep text set choose sliced {
    case Ok carries part => choose std.bytes.to_string(part) {
        case Ok carries value => value,
        case Err carries problem => problem,
    },
    case Err carries problem => problem,
}
assert(text == "Nivren", "byte slice")
length
"#;
    assert_eq!(eval_tree(source), Value::Int(11));
    assert_eq!(eval_vm(source), Value::Int(11));

    let invalid = eval_vm(
        "keep made set std.bytes.from_values([0, 256]) choose made { case Ok carries value => no, case Err carries problem => yes }",
    );
    assert_eq!(invalid, Value::Bool(true));

    let outside = eval_vm(
        "keep data set std.bytes.from_string(\"one\") keep found set std.bytes.get(data, 9) choose found { case Ok carries value => no, case Err carries problem => yes }",
    );
    assert_eq!(outside, Value::Bool(true));
}

#[test]
fn explicit_text_concatenation_is_typed_bounded_and_dual_engine() {
    let source = r#"
choose std.text.concat("Niv", "ren") {
    case Ok carries value => value,
    case Err carries problem => problem,
}
"#;
    let expected = Value::String("Nivren".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
    assert!(nivren::check("std.text.concat(\"value\", 1)").is_err());
}

#[test]
fn bounded_text_partition_and_float_conversion_are_dual_engine() {
    let source = r#"
define inspect takes { } gives Result<String, String> {
    keep parts set std.text.split("MOVED 42 [::1]:6379", " ", 4) or give
    keep address set std.text.split_last(parts[2], ":") or give
    keep number set std.float.parse("1.5") or give
    keep rendered set std.float.format(number) or give
    assert(std.text.starts_with(parts[0], "MOVE"), "prefix")
    give std.text.concat(address[1], rendered)
}
inspect()
"#;
    let expected = Value::Ok(Arc::new(Value::String("63791.5".into())));
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
    assert_eq!(
        eval_vm(
            "choose std.float.parse(\"NaN\") { case Ok carries value => no, case Err carries problem => yes }"
        ),
        Value::Bool(true)
    );
}

#[test]
fn int_text_conversion_is_explicit_checked_and_dual_engine() {
    let source = r#"
define round_trip takes { } gives Result<Int, String> {
    keep parsed set std.int.parse("-9223372036854775808") or give
    assert(std.int.format(parsed) == "-9223372036854775808", "Int format")
    give ok(parsed)
}
round_trip()
"#;
    let expected = Value::Ok(Arc::new(Value::Int(i64::MIN)));
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
    assert_eq!(
        eval_vm(
            "choose std.int.parse(\"9223372036854775808\") { case Ok carries value => no, case Err carries problem => yes }"
        ),
        Value::Bool(true)
    );
}

#[test]
fn binary_codecs_are_typed_bounded_and_endian_explicit() {
    let source = r#"
define verify takes { } gives Result<Int, String> {
    keep number set std.u16.from_int(4660) or give
    keep big set std.binary.u16_be(number)
    keep little set std.binary.u16_le(number)
    keep big_first set std.bytes.get(big, 0) or give
    keep little_first set std.bytes.get(little, 0) or give
    assert(big_first == 18, "big endian first byte")
    assert(little_first == 52, "little endian first byte")
    keep decoded set std.binary.read_u16_be(big, 0) or give
    keep decoded_int set std.u16.to_int(decoded) or give

    keep signed_bytes set std.binary.int_le(-42)
    keep signed set std.binary.read_int_le(signed_bytes, 0) or give
    assert(signed == -42, "signed Int round trip")

    keep float_bytes set std.binary.float_be(1.5)
    keep floated set std.binary.read_float_be(float_bytes, 0) or give
    assert(floated == 1.5, "Float round trip")

    keep joined set std.binary.concat(big, little) or give
    assert(std.bytes.length(joined) == 4, "bounded concatenation")
    give ok(decoded_int)
}
verify()
"#;
    let expected = Value::Ok(Arc::new(Value::Int(4660)));
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);

    let outside = r#"
keep bytes set std.bytes.from_values([1, 2, 3])
choose bytes {
    case Ok carries value => choose std.binary.read_u32_be(value, 0) {
        case Ok carries number => no,
        case Err carries problem => yes,
    },
    case Err carries problem => no,
}
"#;
    assert_eq!(eval_tree(outside), Value::Bool(true));
    assert_eq!(eval_vm(outside), Value::Bool(true));

    assert!(nivren::check("std.binary.u16_be(1)").is_err());
    assert!(nivren::check("std.binary.read_int_be(\"bytes\", 0)").is_err());
}

#[test]
fn cryptographic_hashes_and_hmacs_are_bounded_and_constant_time_verified() {
    let source = r#"
define verify takes { } gives Result<Int, String> {
    keep key set std.bytes.from_string("secret")
    keep message set std.bytes.from_string("Nivren")
    keep digest set std.crypto.sha256(message) or give
    keep repeated set std.crypto.sha256(message) or give
    assert(digest == repeated, "deterministic SHA-256")
    assert(std.bytes.length(digest) == 32, "SHA-256 width")

    keep tag set std.crypto.hmac_sha256(key, message) or give
    keep valid set std.crypto.hmac_sha256_verify(key, message, tag) or give
    keep invalid set std.crypto.hmac_sha256_verify(key, std.bytes.from_string("other"), tag) or give
    assert(valid, "valid HMAC")
    assert(not invalid, "invalid HMAC")
    give ok(std.bytes.length(tag))
}

verify()
"#;
    let expected = Value::Ok(Arc::new(Value::Int(32)));
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);

    let short_tag = r#"
keep key set std.bytes.from_string("key")
keep message set std.bytes.from_string("message")
keep tag set std.bytes.from_string("short")
choose std.crypto.hmac_sha256_verify(key, message, tag) {
    case Ok carries valid => no,
    case Err carries problem => yes,
}
"#;
    assert_eq!(eval_vm(short_tag), Value::Bool(true));
}

#[test]
fn secure_randomness_and_argon2id_are_capability_checked_and_bounded() {
    let source = r#"
define passwords takes { } gives Result<Bool, String> {
    keep salt set std.bytes.from_string("0123456789abcdef")
    keep encoded set std.crypto.password_hash("correct horse", salt, 8192, 1, 1) or give
    keep valid set std.crypto.password_verify("correct horse", encoded) or give
    keep invalid set std.crypto.password_verify("wrong", encoded) or give
    give ok(valid and not invalid)
}

passwords()
"#;
    let expected = Value::Ok(Arc::new(Value::Bool(true)));
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);

    let entropy = r#"
define entropy takes { } gives Result<Int, String> needs Random {
    keep bytes set std.crypto.random_bytes(32) or give
    give ok(std.bytes.length(bytes))
}
entropy()
"#;
    assert_eq!(eval_tree(entropy), Value::Ok(Arc::new(Value::Int(32))));
    assert!(nivren::check("define entropy takes { } { std.crypto.random_bytes(32) }").is_err());
    let program = nivren::parser::parse(nivren::lexer::scan(entropy).unwrap()).unwrap();
    let denied = nivren::runtime::Interpreter::new()
        .with_capabilities(Vec::<String>::new())
        .run(&program)
        .unwrap_err();
    assert!(denied.message.contains("does not allow Random"));

    let hostile = "$argon2id$v=19$m=999999999,t=1,p=1$c2FsdHNhbHRzYWx0c2FsdA$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let hostile_source = format!(
        "choose std.crypto.password_verify(\"password\", \"{hostile}\") {{ case Ok carries valid => no, case Err carries problem => yes }}"
    );
    assert_eq!(eval_vm(&hostile_source), Value::Bool(true));
    assert_eq!(
        eval_vm(
            "choose std.crypto.password_hash(\"password\", std.bytes.from_string(\"short\"), 8192, 1, 1) { case Ok carries value => no, case Err carries problem => yes }"
        ),
        Value::Bool(true)
    );
}

#[test]
fn authenticated_encryption_detects_tampering_and_enforces_key_nonce_and_size_bounds() {
    let source = r#"
define protect takes { } gives Result<Bool, String> {
    keep key set std.crypto.key_import(std.bytes.from_string("0123456789abcdef0123456789abcdef")) or give
    keep nonce set std.bytes.from_string("unique-nonce")
    keep context set std.bytes.from_string("account:42")
    keep message set std.bytes.from_string("Nivren secret")
    keep ciphertext set std.crypto.encrypt(key, nonce, context, message) or give
    assert(std.bytes.length(ciphertext) == std.bytes.length(message) + 16, "authentication tag")
    keep plaintext set std.crypto.decrypt(key, nonce, context, ciphertext) or give
    keep rejected set choose std.crypto.decrypt(key, nonce, std.bytes.from_string("account:43"), ciphertext) {
        case Ok carries value => no,
        case Err carries problem => yes,
    }
    give ok(plaintext == message and rejected)
}

protect()
"#;
    let expected = Value::Ok(Arc::new(Value::Bool(true)));
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);

    assert_eq!(
        eval_vm(
            "choose std.crypto.key_import(std.bytes.from_string(\"short\")) { case Ok carries value => no, case Err carries problem => yes }"
        ),
        Value::Bool(true)
    );
    assert!(nivren::check(
        "std.crypto.encrypt(std.bytes.from_string(\"0123456789abcdef0123456789abcdef\"), std.bytes.from_string(\"unique-nonce\"), std.bytes.from_string(\"\"), std.bytes.from_string(\"message\"))"
    )
    .is_err());
    assert!(
        nivren::check("define compare takes { key is SecretKey } gives Bool { give key == key }")
            .is_err()
    );
    assert!(nivren::check("define key takes { } { std.crypto.key_generate() }").is_err());
    let imported = eval_vm(
        "std.crypto.key_import(std.bytes.from_string(\"0123456789abcdef0123456789abcdef\"))",
    );
    assert_eq!(imported.to_string(), "Ok(<secret-key>)");
    assert_eq!(
        eval_vm(
            "choose std.crypto.key_import(std.bytes.from_string(\"0123456789abcdef0123456789abcdef\")) { case Ok carries key => choose std.json.encode(key) { case Ok carries text => no, case Err carries problem => yes }, case Err carries problem => no }"
        ),
        Value::Bool(true)
    );
}

#[test]
fn ed25519_matches_rfc8032_and_rejects_tampering_in_both_engines() {
    let source = r#"
define verify_vector takes { } gives Result<Bool, String> {
    keep seed set std.encoding.hex_decode("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60") or give
    keep key set std.crypto.key_import(seed) or give
    keep public set std.crypto.ed25519_public(key) or give
    keep expected_public set std.encoding.hex_decode("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a") or give
    assert(public == expected_public, "RFC 8032 public key")
    keep message set std.bytes.from_string("")
    keep signature set std.crypto.ed25519_sign(key, message) or give
    keep expected_signature set std.encoding.hex_decode("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b") or give
    assert(signature == expected_signature, "RFC 8032 signature")
    keep valid set std.crypto.ed25519_verify(public, message, signature) or give
    keep changed set std.crypto.ed25519_verify(public, std.bytes.from_string("changed"), signature) or give
    give ok(valid and not changed)
}
verify_vector()
"#;
    assert_eq!(eval_tree(source), Value::Ok(Arc::new(Value::Bool(true))));
    assert_eq!(eval_vm(source), Value::Ok(Arc::new(Value::Bool(true))));
    assert_eq!(
        eval(
            "choose std.crypto.ed25519_verify(std.bytes.from_string(\"short\"), std.bytes.from_string(\"\"), std.bytes.from_string(\"short\")) { case Ok carries value => no, case Err carries problem => yes }"
        ),
        Value::Bool(true)
    );
}

#[test]
fn gzip_and_zlib_are_deterministic_bounded_and_portable_in_both_engines() {
    let source = r#"
define roundtrip takes { } gives Result<Bool, String> {
    keep input set std.bytes.from_string("Nivren Nivren Nivren")
    keep first set std.compression.gzip(input, 6) or give
    keep second set std.compression.gzip(input, 6) or give
    assert(first == second, "gzip output is deterministic")
    keep restored set std.compression.gzip_decode(first, 1024) or give
    keep packed set std.compression.zlib(input, 9) or give
    keep inflated set std.compression.zlib_decode(packed, 1024) or give
    give ok(restored == input and inflated == input)
}
roundtrip()
"#;
    let expected = Value::Ok(Arc::new(Value::Bool(true)));
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);

    let limited = r#"
keep input set std.bytes.from_string("a long value")
keep packed set std.compression.gzip(input, 6)
choose packed {
    case Ok carries bytes => choose std.compression.gzip_decode(bytes, 2) {
        case Ok carries value => no,
        case Err carries problem => yes,
    },
    case Err carries problem => no,
}
"#;
    assert_eq!(eval_vm(limited), Value::Bool(true));
    assert_eq!(
        eval_vm(
            "choose std.compression.gzip_decode(std.bytes.from_string(\"invalid\"), 1024) { case Ok carries value => no, case Err carries problem => yes }"
        ),
        Value::Bool(true)
    );
    assert!(nivren::check("std.compression.gzip(\"text\", 6)").is_err());
}

#[test]
fn hex_and_base64_encodings_are_canonical_bounded_and_portable() {
    let source = r#"
define roundtrip takes { } gives Result<Bool, String> {
    keep bytes set std.bytes.from_string("Nivren?")
    keep hex set std.encoding.hex_encode(bytes) or give
    keep standard set std.encoding.base64_encode(bytes) or give
    keep url set std.encoding.base64url_encode(bytes) or give
    assert(hex == "4e697672656e3f", "lowercase canonical hex")
    assert(standard == "Tml2cmVuPw==", "padded standard base64")
    assert(url == "Tml2cmVuPw", "unpadded URL-safe base64")
    keep from_hex set std.encoding.hex_decode("4E697672656E3F") or give
    keep from_standard set std.encoding.base64_decode(standard) or give
    keep from_url set std.encoding.base64url_decode(url) or give
    give ok(from_hex == bytes and from_standard == bytes and from_url == bytes)
}
roundtrip()
"#;
    let expected = Value::Ok(Arc::new(Value::Bool(true)));
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
    assert_eq!(
        eval_vm(
            "choose std.encoding.hex_decode(\"xyz\") { case Ok carries bytes => no, case Err carries problem => yes }"
        ),
        Value::Bool(true)
    );
    assert_eq!(
        eval_vm(
            "choose std.encoding.base64_decode(\"not base64\") { case Ok carries bytes => no, case Err carries problem => yes }"
        ),
        Value::Bool(true)
    );
    assert!(nivren::check("std.encoding.hex_encode(\"text\")").is_err());
}

#[test]
fn csv_tables_are_quoted_bounded_typed_and_portable_in_both_engines() {
    let source = r#"
define roundtrip takes { } gives Result<Bool, String> {
    keep headers set ["name", "note"]
    keep rows set std.csv.decode("Ada,\"hello, Nivren\"\r\nLin,\"line one\nline two\"\r\n", headers, ",", 10) or give
    assert(len(rows) == 2, "two CSV records")
    assert(std.map.get(rows[0], "note") == "hello, Nivren", "quoted delimiter")
    assert(std.map.get(rows[1], "note") == "line one\nline two", "quoted newline")
    keep encoded set std.csv.encode(rows, headers, ",") or give
    keep decoded set std.csv.decode(encoded, headers, ",", 10) or give
    give ok(decoded == rows)
}
roundtrip()
"#;
    let expected = Value::Ok(Arc::new(Value::Bool(true)));
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);

    assert_eq!(
        eval_vm(
            "choose std.csv.decode(\"a,b\\r\\nc\\r\\n\", [\"left\", \"right\"], \",\", 10) { case Ok carries rows => no, case Err carries problem => yes }"
        ),
        Value::Bool(true)
    );
    assert_eq!(
        eval_vm(
            "choose std.csv.decode(\"a\\r\\nb\\r\\n\", [\"value\"], \",\", 1) { case Ok carries rows => no, case Err carries problem => yes }"
        ),
        Value::Bool(true)
    );
    assert!(nivren::check("std.csv.decode(1, [\"value\"], \",\", 10)").is_err());
}

#[test]
fn persistent_maps_and_sets_are_generic_and_deterministic() {
    let source = r#"
keep first is Map<String, Int> set std.map.of("nivren", 1)
keep scores is Map<String, Int> set std.map.set(first, "nivren", 2)
keep score is Int set std.map.get(scores, "nivren") ?? 0
keep names is Set<String> set std.set.add(std.set.of("nivren"), "language")
assert(std.map.length(first) == 1, "persistent source map")
assert(std.map.contains(scores, "nivren"), "map contains")
assert(std.set.contains(names, "language"), "set contains")
score + std.set.length(names)
"#;
    assert_eq!(eval_tree(source), Value::Int(4));
    assert_eq!(eval_vm(source), Value::Int(4));

    assert!(nivren::check("std.set.add(std.set.of(1), \"two\")").is_err());
    assert!(nivren::check("keep wrong is Map<String> set std.map.of(\"a\", 1)").is_err());

    let unstable = nivren::run(
        "define identity<Value> takes { value is Value } gives Value { give value } std.map.of(identity, 1)",
    )
    .unwrap_err();
    assert!(
        unstable
            .iter()
            .any(|error| error.message.contains("Comparable")
                || error.message.contains("immutable comparable key"))
    );
}

#[test]
fn generic_list_algorithms_compose_through_readable_pipelines() {
    let source = r#"
define double takes { value is Int } gives Int { give value * 2 }
define even takes { value is Int } gives Bool { give value % 2 == 0 }
define sum takes { total is Int, value is Int } gives Int { give total + value }
define positive takes { value is Int } gives Bool { give value > 0 }
keep values is [Int] set [1, 2, 3] through std.list.transform(double) through std.list.select(even)
assert(std.list.any(values, positive), "any")
assert(std.list.every(values, positive), "every")
std.list.fold(values, 0, sum)
"#;
    assert_eq!(eval_tree(source), Value::Int(12));
    assert_eq!(eval_vm(source), Value::Int(12));

    assert!(
        nivren::check(
            "define wrong takes { value is Int } gives Int { give value } std.list.select([1], wrong)",
        )
        .is_err()
    );

    let hidden_effect = nivren::check(
        "define note takes { value is Int } gives Int needs Log { std.log.info(\"value\") give value } define collect takes { } gives [Int] { give std.list.transform([1], note) }",
    )
    .unwrap_err();
    assert!(
        hidden_effect
            .iter()
            .any(|error| error.message.contains("needs Log"))
    );
}

#[test]
fn iterator_values_adapt_bound_and_consume_sequences() {
    let source = r#"
define double takes { value is Int } gives Int { give value * 2 }
define above_two takes { value is Int } gives Bool { give value > 2 }
keep source set std.iter.from([1, 2, 3, 4, 5])
keep mapped set std.iter.transform(source, double)
keep selected set std.iter.select(mapped, above_two)
keep bounded set std.iter.take(std.iter.skip(selected, 1), 2)
change total set 0
each value within bounded { total = total + value }
keep cursor set std.iter.from([7])
keep first set std.iter.next(cursor) ?? 0
keep ending set std.iter.next(cursor) ?? 9
total + first + ending
"#;
    assert_eq!(eval_tree(source), Value::Int(30));
    assert_eq!(eval_vm(source), Value::Int(30));

    let collected = r#"
keep stream set std.iter.from(["a", "b"])
std.iter.collect(stream)
"#;
    assert_eq!(
        eval_vm(collected),
        Value::Array(Arc::new(vec![
            Value::String("a".into()),
            Value::String("b".into())
        ]))
    );

    let hidden_effect = nivren::check(
        "define note takes { value is Int } gives Int needs Log { std.log.info(\"value\") give value } define collect takes { } gives Iterator<Int> { give std.iter.transform(std.iter.from([1]), note) }",
    )
    .unwrap_err();
    assert!(
        hidden_effect
            .iter()
            .any(|error| error.message.contains("needs Log"))
    );
}

#[test]
fn iterator_callback_adapters_are_truly_lazy_and_share_one_cursor() {
    let source = r#"
change calls set 0
define observe takes { value is Int } gives Int {
    calls = calls + 1
    give value * 2
}
define above_four takes { value is Int } gives Bool { give value > 4 }

keep mapped set std.iter.transform(std.iter.from([1, 2, 3, 4]), observe)
assert(calls == 0, "transform must not run eagerly")
keep first set std.iter.next(mapped) ?? -1
assert(calls == 1, "next evaluates exactly one transform")
keep selected set std.iter.select(mapped, above_four)
assert(calls == 1, "select must not scan eagerly")
keep chosen set std.iter.next(selected) ?? -1
assert(calls == 3, "select stops at its first match")
keep remaining set std.iter.next(mapped) ?? -1
first + chosen + remaining + calls
"#;
    assert_eq!(eval_tree(source), Value::Int(20));
    assert_eq!(eval_vm(source), Value::Int(20));
}

#[test]
fn iterator_terminals_chain_fold_query_and_short_circuit_in_both_engines() {
    let source = r#"
define add takes { total is Int, value is Int } gives Int { give total + value }
define even takes { value is Int } gives Bool { give value % 2 == 0 }
define positive takes { value is Int } gives Bool { give value > 0 }
define three takes { value is Int } gives Bool { give value == 3 }

keep joined set std.iter.chain(std.iter.from([1, 2]), std.iter.from([3, 4]))
keep sum set std.iter.fold(joined, 0, add)
keep found set std.iter.find(std.iter.from([1, 2, 3, 4]), three) ?? 0
keep has_even set std.iter.any(std.iter.from([1, 2, 3, 4]), even)
keep all_positive set std.iter.every(std.iter.from([1, 2, 3, 4]), positive)
keep count set std.iter.count(std.iter.from([1, 2, 3, 4]))
change score set sum + found + count
when has_even { score = score + 10 }
when all_positive { score = score + 20 }
score
"#;
    assert_eq!(eval_tree(source), Value::Int(47));
    assert_eq!(eval_vm(source), Value::Int(47));
}

#[test]
fn lazy_range_sources_are_bounded_single_pass_and_dual_engine() {
    let source = r#"
define add takes { total is Int, value is Int } gives Int { give total + value }
define sample takes { } gives Result<Int, String> {
    keep source set std.iter.range(0, 1000000, 1) or give
    keep first set std.iter.next(source) ?? -1
    keep page set std.iter.take(source, 3)
    keep subtotal set std.iter.fold(page, 0, add)
    keep descending set std.iter.range(5, -1, -2) or give
    keep descending_total set std.iter.fold(descending, 0, add)
    give ok(first + subtotal + descending_total)
}
sample()
"#;
    let expected = Value::Ok(Arc::new(Value::Int(15)));
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);

    let zero_step = eval_vm(
        "choose std.iter.range(0, 10, 0) { case Ok carries stream => no, case Err carries problem => yes }",
    );
    assert_eq!(zero_step, Value::Bool(true));

    let excessive = eval_vm(
        "choose std.iter.range(0, 1000001, 1) { case Ok carries stream => no, case Err carries problem => yes }",
    );
    assert_eq!(excessive, Value::Bool(true));
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
        "keep connection set std.net.connect(\"127.0.0.1\", {port}, 2.0); choose (connection) {{ case Ok carries stream => choose (std.net.read(stream, 5)) {{ case Ok carries text => text, case Err carries error => error }}, case Err carries error => error }}"
    );
    assert_eq!(eval_vm(&source), Value::String("hello".into()));
    server.join().unwrap();
    assert!(nivren::check("std.net.connect(\"localhost\", \"80\", 1.0)").is_err());
}

#[test]
fn tcp_framing_reads_exact_bytes_without_consuming_the_next_message() {
    use std::io::Write as _;

    for vm in [false, true] {
        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("cannot bind framing test: {error}"),
        };
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.write_all(b"+OK\r\nhelloNEXT\r\n").unwrap();
        });
        let source = format!(
            r#"
define framed takes {{ }} gives Result<String, String> needs Network {{
    using stream set std.net.connect("127.0.0.1", {port}, 2.0) or give {{
        keep line set std.net.read_line(stream, 64, 2.0) or give
        assert(line == "+OK", "line framing")
        keep body set std.net.read_exact_bytes(stream, 5, 2.0) or give
        keep text set std.bytes.to_string(body) or give
        keep next set std.net.read_line(stream, 64, 2.0) or give
        assert(next == "NEXT", "next frame remains")
        give ok(text)
    }}
}}
framed()
"#
        );
        let result = if vm {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(result, Value::Ok(Arc::new(Value::String("hello".into()))));
        server.join().unwrap();
    }
}

#[test]
fn official_redis_decodes_recursive_arrays_without_frame_overread() {
    use std::io::Write as _;

    let redis = fs::read_to_string("packages/nivren_redis/src/main.niv").unwrap();
    for vm in [false, true] {
        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("cannot bind Redis framing test: {error}"),
        };
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .write_all(b"*3\r\n+OK\r\n:42\r\n$5\r\nhello\r\n+NEXT\r\n")
                .unwrap();
        });
        let source = format!(
            r#"{redis}
define probe takes {{ }} gives Result<Int, String> needs Network {{
    using stream set connect("127.0.0.1", {port}, 2.0) or give {{
        keep first set receive(stream, 2.0, 1024) or give
        keep count set choose first {{
            case Array carries items => len(items),
            case Text carries text => -1,
            case Error carries problem => -1,
            case Integer carries number => -1,
            case Boolean carries value => -1,
            case Double carries number => -1,
            case BigNumber carries number => -1,
            case Bulk carries data => -1,
            case BlobError carries data => -1,
            case Verbatim carries data => -1,
            case Map carries entries => -1,
            case Set carries values => -1,
            case Push carries values => -1,
            case Null => -1
        }}
        keep second set receive(stream, 2.0, 1024) or give
        keep next set choose second {{
            case Text carries text => text == "NEXT",
            case Error carries problem => no,
            case Integer carries number => no,
            case Boolean carries value => no,
            case Double carries number => no,
            case BigNumber carries number => no,
            case Bulk carries data => no,
            case BlobError carries data => no,
            case Verbatim carries data => no,
            case Array carries items => no,
            case Map carries entries => no,
            case Set carries values => no,
            case Push carries values => no,
            case Null => no
        }}
        when not next {{ give ok(-1) }}
        give ok(count)
    }}
}}
choose probe() {{ case Ok carries value => value, case Err carries problem => -2 }}
"#
        );
        let result = if vm {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(result, Value::Int(3));
        server.join().unwrap();
    }
}

#[test]
fn official_redis_authenticates_and_pipelines_without_frame_loss() {
    use std::io::{Read as _, Write as _};

    let redis = fs::read_to_string("packages/nivren_redis/src/main.niv").unwrap();
    for vm in [false, true] {
        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("cannot bind Redis pipeline test: {error}"),
        };
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let auth = b"*2\r\n$4\r\nAUTH\r\n$6\r\nsecret\r\n";
            let mut request = vec![0; auth.len()];
            stream.read_exact(&mut request).unwrap();
            assert_eq!(request, auth);
            stream.write_all(b"+OK\r\n").unwrap();
            let pipeline = b"*1\r\n$4\r\nPING\r\n*2\r\n$3\r\nGET\r\n$3\r\nkey\r\n";
            request.resize(pipeline.len(), 0);
            stream.read_exact(&mut request).unwrap();
            assert_eq!(request, pipeline);
            stream.write_all(b"+PONG\r\n$5\r\nvalue\r\n").unwrap();
        });
        let source = format!(
            r#"{redis}
define probe takes {{ }} gives Result<String, String> needs Network {{
    keep opened set open("127.0.0.1", {port}, 2.0) or give
    keep empty set pool(2) or give
    keep stored set pool_add(empty, opened) or give
    keep leased set pool_take(stored) or give
    keep authenticated set authenticate(leased.connection, "", "secret", 2.0, 1024) or give
    keep responses set pipeline(leased.connection, [["PING"], ["GET", "key"]], 2.0, 1024) or give
    keep pong set choose responses[0] {{
        case Text carries message => message == "PONG",
        case Error carries problem => no,
        case Integer carries number => no,
        case Boolean carries flag => no,
        case Double carries number => no,
        case BigNumber carries number => no,
        case Bulk carries data => no,
        case BlobError carries data => no,
        case Verbatim carries data => no,
        case Array carries values => no,
        case Map carries entries => no,
        case Set carries values => no,
        case Push carries values => no,
        case Null => no
    }}
    keep value set choose responses[1] {{
        case Bulk carries data => data == std.bytes.from_string("value"),
        case Text carries message => no,
        case Error carries problem => no,
        case Integer carries number => no,
        case Boolean carries flag => no,
        case Double carries number => no,
        case BigNumber carries number => no,
        case Array carries values => no,
        case BlobError carries data => no,
        case Verbatim carries data => no,
        case Map carries entries => no,
        case Set carries values => no,
        case Push carries values => no,
        case Null => no
    }}
    close(leased.connection) or give
    when pong and value and len(leased.pool.idle) == 0 {{ give ok(std.int.format(len(responses))) }}
    give ok("invalid responses")
}}
choose probe() {{ case Ok carries value => value, case Err carries problem => problem }}
"#
        );
        let result = if vm {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(result, Value::String("2".into()));
        server.join().unwrap();
    }
}

#[test]
fn official_redis_secure_connection_verifies_certificates() {
    use std::io::{Read as _, Write as _};

    let redis = fs::read_to_string("packages/nivren_redis/src/main.niv").unwrap();
    for vm in [false, true] {
        let rcgen::CertifiedKey { cert, key_pair } =
            rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let certificate_pem = cert.pem();
        let certificate = cert.der().clone();
        let private_key = rustls::pki_types::PrivateKeyDer::Pkcs8(
            rustls::pki_types::PrivatePkcs8KeyDer::from(key_pair.serialize_der()),
        );
        let server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate], private_key)
            .unwrap();
        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("cannot bind secure Redis test: {error}"),
        };
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let connection = rustls::ServerConnection::new(Arc::new(server_config)).unwrap();
            let mut stream = rustls::StreamOwned::new(connection, stream);
            let expected = b"*1\r\n$4\r\nPING\r\n";
            let mut request = vec![0; expected.len()];
            stream.read_exact(&mut request).unwrap();
            assert_eq!(request, expected);
            stream.write_all(b"+PONG\r\n").unwrap();
        });
        let root = serde_json::to_string(&certificate_pem).unwrap();
        let source = format!(
            r#"{redis}
define probe takes {{ }} gives Result<Bool, String> needs Network {{
    keep options set std.map.set(std.web.tls_options(), "additional_root_pem", {root})
    keep opened set open_secure("localhost", {port}, 3.0, options) or give
    keep responses set pipeline(opened, [["PING"]], 3.0, 1024) or give
    close(opened) or give
    give ok(choose responses[0] {{
        case Text carries message => message == "PONG",
        case Error carries problem => no,
        case Integer carries number => no,
        case Boolean carries flag => no,
        case Double carries number => no,
        case BigNumber carries number => no,
        case Bulk carries data => no,
        case BlobError carries data => no,
        case Verbatim carries data => no,
        case Array carries values => no,
        case Map carries entries => no,
        case Set carries values => no,
        case Push carries values => no,
        case Null => no
    }})
}}
choose probe() {{ case Ok carries value => value, case Err carries problem => no }}
"#
        );
        let result = if vm {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(result, Value::Bool(true));
        server.join().unwrap();
    }
}

#[test]
fn official_redis_follows_bounded_moved_and_ask_redirects() {
    use std::io::{Read as _, Write as _};

    let redis = fs::read_to_string("packages/nivren_redis/src/main.niv").unwrap();
    for vm in [false, true] {
        for kind in ["MOVED", "ASK"] {
            let first = match std::net::TcpListener::bind("127.0.0.1:0") {
                Ok(listener) => listener,
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
                Err(error) => panic!("cannot bind Redis redirect source: {error}"),
            };
            let second = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let first_port = first.local_addr().unwrap().port();
            let second_port = second.local_addr().unwrap().port();
            let kind_owned = kind.to_string();
            let first_server = std::thread::spawn(move || {
                let (mut stream, _) = first.accept().unwrap();
                let ping = b"*1\r\n$4\r\nPING\r\n";
                let mut request = vec![0; ping.len()];
                stream.read_exact(&mut request).unwrap();
                assert_eq!(request, ping);
                write!(stream, "-{kind_owned} 42 127.0.0.1:{second_port}\r\n").unwrap();
            });
            let ask = kind == "ASK";
            let second_server = std::thread::spawn(move || {
                let (mut stream, _) = second.accept().unwrap();
                if ask {
                    let asking = b"*1\r\n$6\r\nASKING\r\n";
                    let mut request = vec![0; asking.len()];
                    stream.read_exact(&mut request).unwrap();
                    assert_eq!(request, asking);
                    stream.write_all(b"+OK\r\n").unwrap();
                }
                let ping = b"*1\r\n$4\r\nPING\r\n";
                let mut request = vec![0; ping.len()];
                stream.read_exact(&mut request).unwrap();
                assert_eq!(request, ping);
                stream.write_all(b"+PONG\r\n").unwrap();
            });
            let source = format!(
                r#"{redis}
define probe takes {{ }} gives Result<Bool, String> needs Network {{
    keep configured set client("127.0.0.1", {first_port}, "", "", no, std.web.tls_options(), 2.0, 1024, 2) or give
    keep outcome set execute(configured, ["PING"]) or give
    give ok(choose outcome.response {{
        case Text carries message => message == "PONG",
        case Error carries problem => no,
        case Integer carries number => no,
        case Boolean carries flag => no,
        case Double carries number => no,
        case BigNumber carries number => no,
        case Bulk carries data => no,
        case BlobError carries data => no,
        case Verbatim carries data => no,
        case Array carries values => no,
        case Map carries entries => no,
        case Set carries values => no,
        case Push carries values => no,
        case Null => no
    }})
}}
choose probe() {{ case Ok carries value => value, case Err carries problem => no }}
"#
            );
            let result = if vm {
                eval_vm(&source)
            } else {
                eval_tree(&source)
            };
            assert_eq!(result, Value::Bool(true), "{kind} in VM={vm}");
            first_server.join().unwrap();
            second_server.join().unwrap();
        }
    }
}

#[test]
fn official_redis_decodes_bounded_resp3_aggregates() {
    use std::io::Write as _;

    let redis = fs::read_to_string("packages/nivren_redis/src/main.niv").unwrap();
    for vm in [false, true] {
        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("cannot bind RESP3 test: {error}"),
        };
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.write_all(
                b"*9\r\n#t\r\n,1.5\r\n(123456789012345678901\r\n!3\r\nERR\r\n=9\r\ntxt:hello\r\n%1\r\n+key\r\n:1\r\n~2\r\n+a\r\n+b\r\n>2\r\n+message\r\n+payload\r\n_\r\n+NEXT\r\n",
            )
            .unwrap();
        });
        let source = format!(
            r#"{redis}
define probe takes {{ }} gives Result<Int, String> needs Network {{
    keep opened set open("127.0.0.1", {port}, 2.0) or give
    keep first set receive_connection(opened, 2.0, 4096) or give
    keep count set choose first {{
        case Array carries values => len(values),
        case Text carries value => -1, case Error carries value => -1, case Integer carries value => -1,
        case Boolean carries value => -1, case Double carries value => -1, case BigNumber carries value => -1,
        case Bulk carries value => -1, case BlobError carries value => -1, case Verbatim carries value => -1,
        case Map carries value => -1, case Set carries value => -1, case Push carries value => -1, case Null => -1
    }}
    keep second set receive_connection(opened, 2.0, 4096) or give
    keep framed set choose second {{
        case Text carries value => value == "NEXT",
        case Error carries value => no, case Integer carries value => no, case Boolean carries value => no,
        case Double carries value => no, case BigNumber carries value => no, case Bulk carries value => no,
        case BlobError carries value => no, case Verbatim carries value => no, case Array carries value => no,
        case Map carries value => no, case Set carries value => no, case Push carries value => no, case Null => no
    }}
    close(opened)
    when framed {{ give ok(count) }}
    give ok(-1)
}}
choose probe() {{ case Ok carries value => value, case Err carries problem => -2 }}
"#
        );
        let result = if vm {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(result, Value::Int(9));
        server.join().unwrap();
    }
}

#[test]
#[ignore = "requires NIVREN_REDIS_PORT pointing at a live Redis release"]
fn official_redis_live_release_matrix() {
    let port = std::env::var("NIVREN_REDIS_PORT")
        .expect("NIVREN_REDIS_PORT is required")
        .parse::<u16>()
        .expect("NIVREN_REDIS_PORT must be a port");
    let redis = fs::read_to_string("packages/nivren_redis/src/main.niv").unwrap();
    let source = format!(
        r#"{redis}
define probe takes {{ }} gives Result<Bool, String> needs Network {{
    keep configured set client("127.0.0.1", {port}, "", "", no, std.web.tls_options(), 3.0, 65536, 2) or give
    keep hello set execute(configured, ["HELLO", "3"]) or give
    keep hello_ok set choose hello.response {{
        case Map carries entries => len(entries) > 0,
        case Text carries value => no, case Error carries value => no, case Integer carries value => no,
        case Boolean carries value => no, case Double carries value => no, case BigNumber carries value => no,
        case Bulk carries value => no, case BlobError carries value => no, case Verbatim carries value => no,
        case Array carries value => no, case Set carries value => no, case Push carries value => no, case Null => no
    }}
    keep stored set execute(hello.client, ["SET", "nivren:matrix", "edition3"]) or give
    keep fetched set execute(stored.client, ["GET", "nivren:matrix"]) or give
    keep value_ok set choose fetched.response {{
        case Bulk carries value => value == std.bytes.from_string("edition3"),
        case Text carries value => no, case Error carries value => no, case Integer carries value => no,
        case Boolean carries value => no, case Double carries value => no, case BigNumber carries value => no,
        case BlobError carries value => no, case Verbatim carries value => no, case Array carries value => no,
        case Map carries value => no, case Set carries value => no, case Push carries value => no, case Null => no
    }}
    close(fetched.client.connection)
    give ok(hello_ok and value_ok)
}}
choose probe() {{ case Ok carries value => value, case Err carries problem => no }}
"#
    );
    assert_eq!(eval_tree(&source), Value::Bool(true));
    assert_eq!(eval_vm(&source), Value::Bool(true));
}

#[test]
fn tcp_partial_writes_make_backpressure_and_progress_explicit() {
    use std::io::Read as _;

    for bytecode in [false, true] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0; 4];
            stream.read_exact(&mut bytes).unwrap();
            bytes
        });
        let source = format!(
            r#"
define send takes {{ }} gives Result<Int, String> needs Network {{
    keep stream set std.net.connect("127.0.0.1", {port}, 2.0) or give
    using connection set stream {{
        give std.net.write_some(connection, "abcdefgh", 4, 2.0)
    }}
}}
choose send() {{ case Ok carries written => written, case Err carries problem => -1 }}
"#
        );
        let result = if bytecode {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(result, Value::Int(4));
        assert_eq!(server.join().unwrap(), *b"abcd");
    }
}

#[test]
fn tcp_readiness_waits_on_the_os_reactor_in_both_engines() {
    use std::io::Write as _;

    for bytecode in [false, true] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(25));
            stream.write_all(b"ready").unwrap();
        });
        let source = format!(
            r#"
define probe takes {{ }} gives Result<Bool, String> needs Network {{
    keep stream set std.net.connect("127.0.0.1", {port}, 2.0) or give
    using connection set stream {{
        give std.net.wait_ready(connection, "read", 2.0)
    }}
}}
choose probe() {{ case Ok carries ready => ready, case Err carries problem => no }}
"#
        );
        let result = if bytecode {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(result, Value::Bool(true));
        server.join().unwrap();
    }
}

#[test]
fn tcp_reactor_selects_many_streams_and_drives_bounded_adapters() {
    use std::io::{Read as _, Write as _};

    for bytecode in [false, true] {
        let first_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let first_port = first_listener.local_addr().unwrap().port();
        let second_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let second_port = second_listener.local_addr().unwrap().port();
        let first_server = std::thread::spawn(move || {
            let (mut stream, _) = first_listener.accept().unwrap();
            // Keep the readiness order deterministic even on slower CI hosts.
            std::thread::sleep(std::time::Duration::from_millis(500));
            stream.write_all(b"late").unwrap();
        });
        let second_server = std::thread::spawn(move || {
            let (mut stream, _) = second_listener.accept().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
            stream.write_all(b"early").unwrap();
            let mut acknowledgement = [0; 3];
            stream.read_exact(&mut acknowledgement).unwrap();
            acknowledgement
        });
        let source = format!(
            r#"
define exchange takes {{ }} gives Result<Int, String> needs Network {{
    keep first_opened set std.net.connect("127.0.0.1", {first_port}, 2.0) or give
    using first set first_opened {{
        keep second_opened set std.net.connect("127.0.0.1", {second_port}, 2.0) or give
        using second set second_opened {{
            keep selected set std.net.wait_ready_any([first, second], "read", 2.0) or give
            keep index set selected ?? -1
            keep message set std.net.read_ready(second, 5, 2.0) or give
            keep written set std.net.write_ready(second, "ack", 1, 2.0) or give
            when message == "early" {{ give ok(index) }}
            give ok(-2)
        }}
    }}
}}
choose exchange() {{ case Ok carries index => index, case Err carries problem => -3 }}
"#
        );
        let result = if bytecode {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(result, Value::Int(1));
        assert_eq!(second_server.join().unwrap(), *b"ack");
        first_server.join().unwrap();
    }
}

#[test]
fn web_requests_are_bounded_typed_and_preserve_status_headers_and_body() {
    use std::io::{Read as _, Write as _};

    for bytecode in [false, true] {
        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("cannot bind loopback test listener: {error}"),
        };
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut request = vec![0; 4096];
            let read = stream.read(&mut request).unwrap();
            request.truncate(read);
            stream
                .write_all(
                    b"HTTP/1.1 418 Teapot\r\nContent-Type: text/plain\r\nContent-Length: 6\r\nConnection: close\r\n\r\nnivren",
                )
                .unwrap();
            request
        });
        let source = format!(
            r#"
keep headers set std.map.of("X-Nivren", "Edition3")
keep response set std.web.request("POST", "http://127.0.0.1:{port}/echo", headers, "hello", 2.0, 1024)
choose response {{
    case Ok carries data => (std.map.get(data, "status") ?? "missing") + ":" + (std.map.get(data, "body") ?? "missing") + ":" + (std.map.get(data, "header:content-type") ?? "missing"),
    case Err carries problem => problem
}}
"#
        );
        let value = if bytecode {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(value, Value::String("418:nivren:text/plain".into()));
        let request = String::from_utf8(server.join().unwrap()).unwrap();
        assert!(request.starts_with("POST /echo HTTP/1.1\r\n"));
        assert!(request.contains("X-Nivren: Edition3\r\n"));
        assert!(request.ends_with("\r\n\r\nhello"));
    }

    assert!(
        nivren::check(
            "std.web.request(\"GET\", \"http://localhost\", std.web.headers(), \"\", 1.0, 1024)"
        )
        .is_ok()
    );
}

#[test]
fn official_trace_exports_bounded_otlp_http_json_in_both_engines() {
    use std::io::{Read as _, Write as _};

    let trace = fs::read_to_string("packages/nivren_trace/src/main.niv").unwrap();
    for bytecode in [false, true] {
        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("cannot bind OTLP test listener: {error}"),
        };
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut request = vec![0; 8192];
            let read = stream.read(&mut request).unwrap();
            request.truncate(read);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
            String::from_utf8(request).unwrap()
        });
        let source = format!(
            r#"{trace}
define send takes {{ }} gives Result<String, String> needs Network {{
    keep value set context("4bf92f3577b34da6a3ce929d0e0e4736", "00f067aa0ba902b7", yes) or give
    keep attribute set otlp_attribute("service.name", "nivren") or give
    keep span set otlp_span(value, "request", "100", "250", [attribute]) or give
    give export_otlp_json("http://127.0.0.1:{port}/v1/traces", std.web.headers(), span, 2.0)
}}
choose send() {{ case Ok carries status => status, case Err carries problem => problem }}
"#
        );
        let value = if bytecode {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(value, Value::String("200".into()));
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v1/traces HTTP/1.1\r\n"));
        assert!(request.contains("Content-Type: application/json\r\n"));
        assert!(request.contains("\"resourceSpans\""));
        assert!(request.contains("\"traceId\":\"4bf92f3577b34da6a3ce929d0e0e4736\""));
        assert!(request.ends_with("}]}]}]}"));
    }
}

#[test]
fn url_components_are_strict_bounded_and_unicode_safe_in_both_engines() {
    let source = r#"
define roundtrip takes { } gives Result<Bool, String> {
    keep encoded set std.web.encode_component("Nivren / 🜁 +") or give
    assert(encoded == "Nivren%20%2F%20%F0%9F%9C%81%20%2B", "RFC 3986 component")
    keep decoded set std.web.decode_component(encoded) or give
    keep plus set std.web.decode_component("a+b") or give
    give ok(decoded == "Nivren / 🜁 +" and plus == "a+b")
}
roundtrip()
"#;
    assert_eq!(eval_tree(source), Value::Ok(Arc::new(Value::Bool(true))));
    assert_eq!(eval_vm(source), Value::Ok(Arc::new(Value::Bool(true))));
    assert_eq!(
        eval_vm(
            "choose std.web.decode_component(\"%GG\") { case Ok carries value => no, case Err carries problem => yes }"
        ),
        Value::Bool(true)
    );
}

#[test]
fn websocket_standard_library_exchanges_bounded_text_in_both_engines() {
    use std::io::Read as _;

    for bytecode in [false, true] {
        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("cannot bind WebSocket listener: {error}"),
        };
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = vec![];
            while !bytes.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                bytes.push(byte[0]);
            }
            let request = String::from_utf8(bytes).unwrap();
            let mut lines = request[..request.len() - 4].split("\r\n");
            assert_eq!(lines.next().unwrap(), "GET /echo HTTP/1.1");
            let headers = lines
                .map(|line| {
                    let (name, value) = line.split_once(':').unwrap();
                    (name.to_ascii_lowercase(), value.trim().to_string())
                })
                .collect::<BTreeMap<_, _>>();
            let mut socket =
                nivren::websocket::WebSocket::accept(Arc::new(Mutex::new(stream)), "GET", &headers)
                    .unwrap();
            assert_eq!(socket.receive_text(1024).unwrap(), "hello");
            socket.send_text("echo:hello").unwrap();
        });
        let source = format!(
            r#"
define exchange takes {{ }} gives Result<String, String> needs Network {{
    keep opened set std.web.websocket_connect("127.0.0.1", {port}, "/echo", 2.0) or give
    using socket set opened {{
        keep sent set std.web.websocket_send(socket, "hello") or give
        give std.web.websocket_receive(socket, 1024)
    }}
}}
choose exchange() {{ case Ok carries message => message, case Err carries problem => problem }}
"#
        );
        let value = if bytecode {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(value, Value::String("echo:hello".into()));
        server.join().unwrap();
    }
}

#[test]
fn secure_websocket_listeners_serve_verified_tls_in_both_engines() {
    for bytecode in [false, true] {
        let reservation = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("cannot reserve secure WebSocket port: {error}"),
        };
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);

        let rcgen::CertifiedKey { cert, key_pair } =
            rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let certificate_pem = cert.pem();
        let private_key_pem = key_pair.serialize_pem();
        let trusted_certificate = cert.der().clone();
        let rcgen::CertifiedKey {
            cert: client_cert,
            key_pair: client_key,
        } = rcgen::generate_simple_self_signed(vec!["nivren-client".into()]).unwrap();
        let client_ca_pem = client_cert.pem();
        let client_certificate = client_cert.der().clone();
        let client_private_key = rustls::pki_types::PrivateKeyDer::Pkcs8(
            rustls::pki_types::PrivatePkcs8KeyDer::from(client_key.serialize_der()),
        );
        let client = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            let stream = loop {
                match std::net::TcpStream::connect(("127.0.0.1", port)) {
                    Ok(stream) => break stream,
                    Err(_) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    Err(error) => panic!("cannot connect to secure Nivren listener: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut roots = rustls::RootCertStore::empty();
            roots.add(trusted_certificate).unwrap();
            let config = rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_client_auth_cert(vec![client_certificate], client_private_key)
                .unwrap();
            let server_name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
            let connection = rustls::ClientConnection::new(Arc::new(config), server_name).unwrap();
            let stream = rustls::StreamOwned::new(connection, stream);
            let mut socket =
                nivren::websocket::WebSocket::connect_tls(stream, "localhost", "/secure").unwrap();
            socket.send_text("encrypted hello").unwrap();
            assert_eq!(socket.receive_text(1024).unwrap(), "secure:encrypted hello");
        });
        let certificate_literal = serde_json::to_string(&certificate_pem).unwrap();
        let key_literal = serde_json::to_string(&private_key_pem).unwrap();
        let client_ca_literal = serde_json::to_string(&client_ca_pem).unwrap();
        let source = format!(
            r#"
define serve takes {{ }} gives Result<String, String> needs Network {{
    keep auth_policy set std.map.set(std.web.tls_options(), "client_auth", "required")
    keep options set std.map.set(auth_policy, "client_ca_pem", {client_ca_literal})
    keep opened set std.web.websocket_secure_listen("127.0.0.1", {port}, {certificate_literal}, {key_literal}, options) or give
    using listener set opened {{
        keep accepted set std.web.websocket_secure_accept(listener, 3.0) or give
        using socket set accepted {{
            keep message set std.web.websocket_receive(socket, 1024) or give
            keep sent set std.web.websocket_send(socket, "secure:" + message) or give
            give ok(message)
        }}
    }}
}}
choose serve() {{ case Ok carries message => message, case Err carries problem => problem }}
"#
        );
        let result = if bytecode {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(result, Value::String("encrypted hello".into()));
        client.join().unwrap();
    }
}

#[test]
fn secure_websocket_clients_present_verified_mtls_identity_in_both_engines() {
    for bytecode in [false, true] {
        let rcgen::CertifiedKey {
            cert: server_cert,
            key_pair: server_key,
        } = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let server_certificate_pem = server_cert.pem();
        let server_certificate = server_cert.der().clone();
        let server_private_key = rustls::pki_types::PrivateKeyDer::Pkcs8(
            rustls::pki_types::PrivatePkcs8KeyDer::from(server_key.serialize_der()),
        );
        let rcgen::CertifiedKey {
            cert: client_cert,
            key_pair: client_key,
        } = rcgen::generate_simple_self_signed(vec!["nivren-client".into()]).unwrap();
        let client_certificate_pem = client_cert.pem();
        let client_private_key_pem = client_key.serialize_pem();
        let mut client_roots = rustls::RootCertStore::empty();
        client_roots.add(client_cert.der().clone()).unwrap();
        let client_verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(client_roots))
            .build()
            .unwrap();
        let server_config = rustls::ServerConfig::builder()
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(vec![server_certificate], server_private_key)
            .unwrap();
        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("cannot bind mTLS WebSocket listener: {error}"),
        };
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let connection = rustls::ServerConnection::new(Arc::new(server_config)).unwrap();
            let stream = rustls::StreamOwned::new(connection, stream);
            let mut socket = nivren::websocket::WebSocket::accept_tls_request(stream).unwrap();
            assert_eq!(socket.receive_text(1024).unwrap(), "client identity");
            socket.send_text("accepted identity").unwrap();
        });
        let server_root_literal = serde_json::to_string(&server_certificate_pem).unwrap();
        let client_certificate_literal = serde_json::to_string(&client_certificate_pem).unwrap();
        let client_key_literal = serde_json::to_string(&client_private_key_pem).unwrap();
        let source = format!(
            r#"
define exchange takes {{ }} gives Result<String, String> needs Network {{
    keep roots set std.map.set(std.web.tls_options(), "additional_root_pem", {server_root_literal})
    keep identity set std.map.set(roots, "client_certificate_pem", {client_certificate_literal})
    keep policy set std.map.set(identity, "client_private_key_pem", {client_key_literal})
    keep opened set std.web.websocket_secure_connect("localhost", {port}, "/mutual", 3.0, policy) or give
    using socket set opened {{
        keep sent set std.web.websocket_send(socket, "client identity") or give
        give std.web.websocket_receive(socket, 1024)
    }}
}}
choose exchange() {{ case Ok carries message => message, case Err carries problem => problem }}
"#
        );
        let result = if bytecode {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(result, Value::String("accepted identity".into()));
        server.join().unwrap();
    }
}

#[test]
fn tcp_listeners_accept_bounded_connections_and_close_with_scope() {
    use std::io::{Read as _, Write as _};

    for bytecode in [false, true] {
        let reservation = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("cannot reserve loopback port: {error}"),
        };
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let client = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            let mut stream = loop {
                match std::net::TcpStream::connect(("127.0.0.1", port)) {
                    Ok(stream) => break stream,
                    Err(_) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    Err(error) => panic!("cannot connect to Nivren listener: {error}"),
                }
            };
            stream.write_all(b"ping").unwrap();
            let mut reply = [0; 4];
            stream.read_exact(&mut reply).unwrap();
            reply
        });
        let source = format!(
            r#"
define serve takes {{ listener is TcpListener }} gives Result<Int, String> needs Network {{
    using server set listener {{
        keep accepted set std.net.accept(server, 2.0)
        using connection set accepted or give {{
            keep request set std.net.read(connection, 4) or give
            keep sent set std.net.write(connection, "pong") or give
            give ok(len(request))
        }}
    }}
}}
keep opened set std.net.listen("127.0.0.1", {port})
choose opened {{ case Ok carries listener => serve(listener), case Err carries problem => err(problem) }}
"#
        );
        let result = if bytecode {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(result, Value::Ok(Arc::new(Value::Int(4))));
        assert_eq!(client.join().unwrap(), *b"pong");
    }
}

#[test]
fn tcp_line_iterators_are_lazy_bounded_and_recover_after_oversized_frames() {
    use std::io::Write as _;

    for bytecode in [false, true] {
        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("cannot bind TCP iterator listener: {error}"),
        };
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.write_all(b"one\r\n123456\r\nthree\r\n").unwrap();
        });
        let source = format!(
            r#"
define consume takes {{ }} gives Result<Bool, String> needs Network {{
    keep opened set std.net.connect("127.0.0.1", {port}, 2.0) or give
    using connection set opened {{
        keep lines set std.iter.tcp_lines(connection, 5, 2.0) or give
        keep first set std.iter.next(lines) ?? err("missing first line")
        keep oversized set std.iter.next(lines) ?? err("missing oversized line")
        keep third set std.iter.next(lines) ?? err("missing third line")
        keep ended set std.iter.next(lines)
        keep first_ok set choose first {{ case Ok carries value => value == "one", case Err carries problem => no }}
        keep overflow_ok set choose oversized {{ case Ok carries value => no, case Err carries problem => yes }}
        keep third_ok set choose third {{ case Ok carries value => value == "three", case Err carries problem => no }}
        give ok(first_ok and overflow_ok and third_ok and ended == none)
    }}
}}
choose consume() {{ case Ok carries value => value, case Err carries problem => no }}
"#
        );
        let value = if bytecode {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(value, Value::Bool(true));
        server.join().unwrap();
    }
}

#[test]
fn web_servers_parse_bounded_requests_and_write_managed_responses() {
    use std::io::{Read as _, Write as _};

    for bytecode in [false, true] {
        let reservation = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("cannot reserve loopback port: {error}"),
        };
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let client = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            let mut stream = loop {
                match std::net::TcpStream::connect(("127.0.0.1", port)) {
                    Ok(stream) => break stream,
                    Err(_) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    Err(error) => panic!("cannot connect to Nivren web server: {error}"),
                }
            };
            stream
                .write_all(
                    b"POST /items HTTP/1.1\r\nHost: localhost\r\nX-Test: yes\r\nContent-Length: 5\r\n\r\nhello",
                )
                .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        });
        let source = format!(
            r#"
define serve takes {{ listener is TcpListener }} gives Result<String, String> needs Network {{
    using server set listener {{
        keep accepted set std.net.accept(server, 2.0)
        using connection set accepted or give {{
            keep request set std.web.read_request(connection, 1024) or give
            keep path set std.map.get(request, "path") ?? ""
            keep body set std.map.get(request, "body") ?? ""
            keep headers set std.map.of("Content-Type", "text/plain")
            keep sent set std.web.respond(connection, 201, headers, "created") or give
            give ok(path + ":" + body)
        }}
    }}
}}
keep opened set std.net.listen("127.0.0.1", {port})
choose opened {{ case Ok carries listener => serve(listener), case Err carries problem => err(problem) }}
"#
        );
        let result = if bytecode {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(
            result,
            Value::Ok(Arc::new(Value::String("/items:hello".into())))
        );
        let response = client.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 201 Created\r\n"));
        assert!(response.contains("Content-Type: text/plain\r\n"));
        assert!(response.ends_with("\r\n\r\ncreated"));
    }
}

#[test]
fn using_scopes_close_resources_on_normal_and_early_return_paths() {
    use std::io::Read as _;

    for (early_return, bytecode) in [(false, false), (false, true), (true, false), (true, true)] {
        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("cannot bind loopback test listener: {error}"),
        };
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut received = vec![];
            stream.read_to_end(&mut received).unwrap();
            received
        });
        let function = if early_return {
            "define finish takes { stream is TcpStream } gives Int needs Network { using socket set stream { give 7 } }"
        } else {
            "define finish takes { stream is TcpStream } gives Int needs Network { using socket set stream { keep sent set std.net.write(socket, \"closed\") none } give 7 }"
        };
        let source = format!(
            "{function} keep opened set std.net.connect(\"127.0.0.1\", {port}, 2.0) choose opened {{ case Ok carries stream => finish(stream), case Err carries problem => 0 }}"
        );
        let value = if bytecode {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(value, Value::Int(7));
        let received = server.join().unwrap();
        if !early_return {
            assert_eq!(received, b"closed");
        }
    }

    assert!(nivren::check("using value set 42 { value }").is_err());
    let missing = nivren::check(
        "define finish takes { stream is TcpStream } gives Null { using socket set stream { none } }",
    )
    .unwrap_err();
    assert!(
        missing
            .iter()
            .any(|error| error.message.contains("needs Network"))
    );
}

#[test]
fn using_scopes_close_bounded_file_handles_in_both_engines() {
    let directory = module_fixture("file-resources");
    for (name, bytecode) in [("tree.txt", false), ("vm.txt", true)] {
        let path = directory.join(name);
        let source = format!(
            r#"
define save takes {{ path is String }} gives Result<Int, String> needs FileWrite {{
    keep opened set std.files.open_write(path)
    using file set opened or give {{
        keep written set std.files.write_to(file, "nivren") or give
        give ok(7)
    }}
}}
save("{}")
"#,
            nivren_string_contents(path.to_string_lossy())
        );
        let value = if bytecode {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(value, Value::Ok(Arc::new(Value::Int(7))));
        assert_eq!(fs::read_to_string(path).unwrap(), "nivren");
    }

    let readable = directory.join("read.txt");
    fs::write(&readable, "bounded").unwrap();
    let source = format!(
        r#"
define load takes {{ path is String }} gives Result<String, String> needs FileRead {{
    keep opened set std.files.open_read(path)
    using file set opened or give {{
        give std.files.read_from(file, 64)
    }}
}}
load("{}")
"#,
        nivren_string_contents(readable.to_string_lossy())
    );
    assert_eq!(
        eval_vm(&source),
        Value::Ok(Arc::new(Value::String("bounded".into())))
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn cross_resource_failure_stress_closes_files_and_tcp_streams_in_both_engines() {
    let directory = module_fixture("cross-resource-stress");
    let path = directory.join("record.txt");
    fs::write(&path, "bounded").unwrap();
    for bytecode in [false, true] {
        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("cannot bind resource stress listener: {error}"),
        };
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            for _ in 0..64 {
                let (stream, _) = listener.accept().unwrap();
                drop(stream);
            }
        });
        let source = format!(
            r#"
define stress takes {{ path is String }} gives Result<Int, String> needs FileRead, Network {{
    change index set 0
    repeat index < 64 {{
        keep opened_file set std.files.open_read(path) or give
        using file set opened_file {{
            keep closed set std.files.close(file) or give
            keep rejected set choose std.files.read_from(file, 1) {{ case Ok carries value => no, case Err carries problem => yes }}
            when not rejected {{ give err("closed file accepted a read") }}
        }}
        keep opened_stream set std.net.connect("127.0.0.1", {port}, 2.0) or give
        using connection set opened_stream {{
            keep rejected set choose std.net.read_exact_bytes(connection, 1, 2.0) {{ case Ok carries value => no, case Err carries problem => yes }}
            when not rejected {{ give err("closed peer produced an exact byte") }}
        }}
        index = index + 1
    }}
    give ok(index)
}}
stress("{}")
"#,
            nivren_string_contents(path.to_string_lossy())
        );
        let value = if bytecode {
            eval_vm(&source)
        } else {
            eval_tree(&source)
        };
        assert_eq!(value, Value::Ok(Arc::new(Value::Int(64))));
        server.join().unwrap();
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn async_files_use_bounded_executor_tasks_in_both_engines() {
    let directory = module_fixture("async-files");
    let path = directory.join("message.txt");
    let path = path
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let source = format!(
        r#"
define roundtrip takes {{ }} gives Result<String, String> needs FileRead, FileWrite, Task {{
    keep writing set std.files.write_async("{path}", "async nivren") or give
    keep written set wait writing or give
    keep reading set std.files.read_async("{path}", 1024) or give
    give wait reading
}}
choose roundtrip() {{ case Ok carries contents => contents, case Err carries problem => problem }}
"#
    );
    assert_eq!(eval_tree(&source), Value::String("async nivren".into()));
    assert_eq!(eval_vm(&source), Value::String("async nivren".into()));
    assert!(nivren::check("std.files.read_async(\"file\", 0)").is_ok());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn structured_tasks_cancel_and_exchange_channel_values() {
    let source = "keep channel set std.channels.create(1); define producer takes { } gives Int needs Channel { keep sent set std.channels.send(channel, 42, 2.0); give choose (sent) { case Ok carries value => 1, case Err carries error => 0 }; } keep task set std.tasks.spawn(producer); keep received set std.channels.receive(channel, 2.0); keep value set choose (received) { case Ok carries item => item, case Err carries error => 0 }; keep completed set std.tasks.await(task); assert(choose (completed) { case Ok carries code => code == 1, case Err carries error => no }, \"task completion\"); value";
    assert_eq!(eval_vm(source), Value::Int(42));

    let cancellation = "define forever takes { } gives Int { change value set 0; repeat (value < 9223372036854775807) { value = value + 1; } give value; } keep task set std.tasks.spawn(forever); std.tasks.cancel(task); keep result set std.tasks.await(task); choose (result) { case Ok carries value => no, case Err carries error => yes }";
    assert_eq!(eval_vm(cancellation), Value::Bool(true));
    assert!(nivren::check("std.tasks.spawn(42)").is_err());
    assert!(nivren::run("std.tasks.spawn(42)").is_err());
}

#[test]
fn bytecode_vm_matches_the_tree_interpreter() {
    let programs = [
        "2 + 3 * 4",
        "change n set 0; repeat (n < 5) { n = n + 1; } n",
        "when yes and not no { 42 } otherwise { 0 }",
        "define outer takes { x is Int } { define inner takes { y is Int } { give x + y; } give inner; } outer(2)(40)",
        "keep values set append([1, 2], 3); values[2]",
        "keep missing is String? set none; missing ?? \"fallback\"",
        "shape Person { name is String, age is Int } keep person set Person(\"Ada\", 37); person.age",
        "choice State { Idle, Ready } keep state set State.Ready; choose (state) { case Idle => 0, case Ready => 42 }",
        "keep result is Result<Int, String> set ok(42); choose (result) { case Ok carries value => value, case Err carries message => 0 }",
        "change total set 0; each (value within [10, 20, 12]) { total = total + value; } total",
        "define first takes { } gives Int { each (value within [42, 0]) { give value; } give 0; } first()",
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
        "define value takes { } gives Int { give 42; } expose { value };",
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
        "define inner takes { } gives Int { give 1 / 0; } define outer takes { } gives Int { give inner(); } outer()",
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
    let source = "define twice takes { value is Int } gives Int { give value * 2; }\nkeep answer set twice(21);\nanswer";
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
fn structured_log_events_are_machine_readable_and_capability_checked() {
    let directory = module_fixture("structured-log");
    let source = directory.join("event.niv");
    fs::write(
        &source,
        "std.log.event(\"info\", \"started\", std.map.of(\"request\", \"42\"))",
    )
    .unwrap();
    let source_text = source.display().to_string();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_niv"))
        .args(["run", &source_text])
        .output()
        .unwrap();
    assert!(output.status.success());
    let event: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(event["level"], "info");
    assert_eq!(event["message"], "started");
    assert_eq!(event["fields"]["request"], "42");

    let missing = nivren::check(
        "define emit takes { } gives Null { give std.log.event(\"info\", \"x\", std.map.of(\"key\", \"value\")) }",
    )
    .unwrap_err();
    assert!(
        missing
            .iter()
            .any(|error| error.message.contains("needs Log"))
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn cli_exports_stable_observation_and_privacy_safe_crash_reports() {
    let directory = module_fixture("observability-reports");
    let healthy = directory.join("healthy.niv");
    let failing = directory.join("failing.niv");
    let profile = directory.join("profile.json");
    let crash = directory.join("crash.json");
    fs::write(
        &healthy,
        "define one takes {} gives Int { give 1 }\n\
         define many takes {} gives [Int] or String needs Task {\n\
             keep first set perform std.tasks.spawn with { operation set one }\n\
             keep second set perform std.tasks.spawn with { operation set one }\n\
             give perform std.tasks.all with { tasks set [first, second] }\n\
         }\n\
         perform many with {}",
    )
    .unwrap();
    fs::write(
        &failing,
        "define secret takes { } gives Int { give 1 / 0 } secret() // PRIVATE-SOURCE",
    )
    .unwrap();
    let niv = env!("CARGO_BIN_EXE_niv");
    let output = std::process::Command::new(niv)
        .args([
            "profile",
            "--json",
            profile.to_str().unwrap(),
            healthy.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&fs::read(&profile).unwrap()).unwrap();
    assert_eq!(report["schema"], "org.nivren.observation.v1");
    assert_eq!(report["kind"], "profile");
    assert!(report["instructions"].as_u64().unwrap() > 0);
    assert!(report["execution"]["instructions"].as_u64().unwrap() > 0);
    assert!(report["memory"]["allocation_work_bytes"].as_u64().unwrap() > 0);
    assert!(
        report["memory"]["heap"]["tracked_environments"]
            .as_u64()
            .is_some()
    );
    assert!(report["effects"]["perform_boundaries"].as_u64().unwrap() >= 4);
    assert!(report["effects"]["count"].as_u64().unwrap() >= 3);
    assert_eq!(report["async_tasks"]["spawns"], 2);
    assert_eq!(report["async_tasks"]["joins"], 2);
    assert!(report["engines"]["native"]["fallbacks"].as_u64().is_some());

    let output = std::process::Command::new(niv)
        .args([
            "run",
            "--crash-report",
            crash.to_str().unwrap(),
            failing.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let bytes = fs::read(&crash).unwrap();
    let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(report["schema"], "org.nivren.crash.v1");
    assert_eq!(report["program"], "failing.niv");
    assert!(!String::from_utf8_lossy(&bytes).contains("PRIVATE-SOURCE"));
    assert!(!String::from_utf8_lossy(&bytes).contains(directory.to_str().unwrap()));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn cli_snapshot_tests_require_explicit_review_and_acceptance() {
    let directory = module_fixture("snapshot-tests");
    let source = directory.join("answer_test.niv");
    fs::write(&source, "6 * 7").unwrap();
    let niv = env!("CARGO_BIN_EXE_niv");
    let path = directory.to_str().unwrap();
    let missing = std::process::Command::new(niv)
        .args(["test", "--snapshots", path])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    let accepted = std::process::Command::new(niv)
        .args(["test", "--accept-snapshots", path])
        .output()
        .unwrap();
    assert!(accepted.status.success());
    assert_eq!(
        fs::read_to_string(format!("{}.snap", source.display())).unwrap(),
        "42\n"
    );
    let verified = std::process::Command::new(niv)
        .args(["test", "--snapshots", path])
        .output()
        .unwrap();
    assert!(verified.status.success());
    fs::write(&source, "7 * 7").unwrap();
    let changed = std::process::Command::new(niv)
        .args(["test", "--snapshots", path])
        .output()
        .unwrap();
    assert!(!changed.status.success());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn debugger_hook_steps_source_and_exposes_user_variables() {
    let source = "keep answer set 42;\nanswer + 1";
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
        "[package]\nname = \"sample\"\nversion = \"1.2.3\"\nentry = \"src/main.niv\"\n\n[capabilities]\nFileRead = \"path:./data\"\n",
    )
    .unwrap();
    fs::write(project.join("src/main.niv"), "keep answer set 42; answer").unwrap();
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
    assert_eq!(
        nivren::package::search("amp", &registry).unwrap(),
        vec![nivren::package::SearchResult {
            name: "sample".into(),
            versions: vec!["1.2.3".into()],
        }]
    );
    assert!(nivren::package::search("../", &registry).is_err());
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(registry.join("v1/index/sample/1.2.3.json")).unwrap())
            .unwrap();
    assert_eq!(metadata["capabilities"], serde_json::json!(["FileRead"]));
    assert_eq!(metadata["capability_scopes"]["FileRead"], "path:./data");
    nivren::package::set_yanked("sample", "1.2.3", &registry, true).unwrap();
    assert!(nivren::package::fetch("sample", "1.2.3", &registry).is_err());
    assert!(
        nivren::package::search("sample", &registry)
            .unwrap()
            .is_empty()
    );
    nivren::package::set_yanked("sample", "1.2.3", &registry, false).unwrap();
    assert_eq!(
        nivren::package::fetch("sample", "1.2.3", &registry).unwrap(),
        first
    );
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
        .insert("src/main.niv".into(), b"keep answer set 7; answer".to_vec());
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
fn official_packages_test_document_publish_install_and_run_together() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let registry = module_fixture("official-package-registry");
    let package_names = [
        "nivren_aead",
        "nivren_aws",
        "nivren_columnar",
        "nivren_compression",
        "nivren_crypto",
        "nivren_csv",
        "nivren_database",
        "nivren_desktop",
        "nivren_discord",
        "nivren_gpu",
        "nivren_image",
        "nivren_jwt",
        "nivren_matrix",
        "nivren_metrics",
        "nivren_oidc",
        "nivren_redis",
        "nivren_routing",
        "nivren_secrets",
        "nivren_sql",
        "nivren_stats",
        "nivren_svg",
        "nivren_testing",
        "nivren_trace",
        "nivren_validation",
        "nivren_wav",
    ];
    for name in package_names {
        let root = repository.join("packages").join(name);
        let mut edition_sources = vec![root.join("src/main.niv")];
        let tests_root = root.join("tests");
        if tests_root.exists() {
            edition_sources.extend(
                fs::read_dir(&tests_root)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .filter(|path| path.extension().is_some_and(|extension| extension == "niv")),
            );
        }
        for source_path in edition_sources {
            let source = fs::read_to_string(&source_path).unwrap();
            for (line_number, line) in source.lines().enumerate() {
                let line = line.trim_start();
                assert!(
                    !(line.starts_with("define ") && line.contains('(')),
                    "official package {name} uses an Edition 3 definition at {}:{}",
                    source_path.display(),
                    line_number + 1
                );
                assert!(
                    !((line.starts_with("shape ") || line.starts_with("choice "))
                        && line.contains('{')
                        && !line.contains(" holds")),
                    "official package {name} uses an Edition 3 declaration at {}:{}",
                    source_path.display(),
                    line_number + 1
                );
                assert!(
                    !((line.starts_with("keep ") || line.starts_with("change "))
                        && line.contains(" = ")
                        && !line.contains(" set ")),
                    "official package {name} uses Edition 3 binding syntax at {}:{}",
                    source_path.display(),
                    line_number + 1
                );
            }
        }
        let tested = Command::new(env!("CARGO_BIN_EXE_niv"))
            .args(["test", root.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            tested.status.success(),
            "official package {name} tests failed:\n{}",
            String::from_utf8_lossy(&tested.stderr)
        );
        let manifest = nivren::project::Manifest::load(&root).unwrap();
        let program =
            nivren::modules::load_project(&manifest.root, &manifest.entry_path()).unwrap();
        nivren::typecheck::check(&program).unwrap();
        let docs = nivren::documentation::generate(&manifest.name, &manifest.version, &program);
        assert!(docs.contains(name));
        assert!(docs.contains("## Public API"));
        let package = nivren::package::Package::build(&manifest).unwrap();
        let first = package.encode().unwrap();
        let second = nivren::package::Package::build(&manifest)
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(first, second);
        nivren::package::publish(&first, &registry).unwrap();
    }
    let results = nivren::package::search("nivren", &registry).unwrap();
    assert_eq!(
        results
            .iter()
            .map(|result| result.name.as_str())
            .collect::<Vec<_>>(),
        package_names
    );

    let consumer = module_fixture("official-package-consumer");
    fs::create_dir_all(consumer.join("src")).unwrap();
    fs::write(
        consumer.join("niv.toml"),
        "[package]\nname = \"official_consumer\"\nversion = \"1.0.0\"\nentry = \"src/main.niv\"\n\n[dependencies]\nnivren_aead = \"1.0.0\"\nnivren_aws = \"1.0.0\"\nnivren_columnar = \"1.0.0\"\nnivren_compression = \"1.0.0\"\nnivren_csv = \"1.0.0\"\nnivren_crypto = \"1.0.0\"\nnivren_database = \"1.0.0\"\nnivren_desktop = \"1.0.0\"\nnivren_discord = \"1.0.0\"\nnivren_gpu = \"1.0.0\"\nnivren_image = \"1.0.0\"\nnivren_jwt = \"1.0.0\"\nnivren_matrix = \"1.0.0\"\nnivren_metrics = \"1.0.0\"\nnivren_oidc = \"1.0.0\"\nnivren_redis = \"1.0.0\"\nnivren_routing = \"1.0.0\"\nnivren_secrets = \"1.0.0\"\nnivren_sql = \"1.0.0\"\nnivren_stats = \"1.0.0\"\nnivren_svg = \"1.0.0\"\nnivren_testing = \"1.0.0\"\nnivren_trace = \"1.0.0\"\nnivren_validation = \"1.0.0\"\nnivren_wav = \"1.0.0\"\n",
    )
    .unwrap();
    fs::write(
        consumer.join("src/main.niv"),
        r#"
use "@nivren_routing"
use "@nivren_aead"
use "@nivren_aws"
use "@nivren_columnar"
use "@nivren_testing"
use "@nivren_validation"
use "@nivren_discord"
use "@nivren_image"
use "@nivren_crypto"
use "@nivren_compression"
use "@nivren_csv"
use "@nivren_database"
use "@nivren_desktop"
use "@nivren_secrets"
use "@nivren_gpu"
use "@nivren_sql"
use "@nivren_stats"
use "@nivren_jwt"
use "@nivren_matrix"
use "@nivren_metrics"
use "@nivren_oidc"
use "@nivren_redis"
use "@nivren_svg"
use "@nivren_wav"
use "@nivren_trace"

keep health set nivren_routing.route("GET", "/health", "health")
keep selected set nivren_routing.first_match([health], "GET", "/health")
keep body set nivren_discord.message_body("hello")
keep encoded set choose body { case Ok carries text => text, case Err carries problem => "" }
keep crypto_ok set choose nivren_crypto.sign("secret", "hello") {
    case Ok carries tag => nivren_crypto.verify("secret", "hello", tag) == ok(yes),
    case Err carries problem => no,
}
keep sql_ok set choose nivren_sql.select("users", ["id"]) {
    case Ok carries query => query.text == "SELECT id FROM users",
    case Err carries problem => no,
}
keep redis_ok set nivren_redis.command(["PING"]) == ok("*1\r\n$4\r\nPING\r\n")
keep compressed set nivren_compression.gzip_text("Nivren", 6)
keep compression_ok set choose compressed { case Ok carries bytes => nivren_compression.gunzip_text(bytes, 1024) == ok("Nivren"), case Err carries problem => no }
keep csv_rows set nivren_csv.decode("Nivren,1\r\n", ["name", "version"], 10)
keep csv_ok set choose csv_rows { case Ok carries rows => len(rows) == 1 and std.map.get(rows[0], "name") == "Nivren", case Err carries problem => no }
keep database_pool set nivren_database.PoolConfig(1, 4, 5.0, 60.0)
keep database_ok set nivren_database.validate_pool(database_pool) == ok(database_pool)
keep desktop_window set nivren_desktop.Window("Nivren", 800, 600, "app://index.html")
keep desktop_ok set nivren_desktop.validate_window(desktop_window) == ok(desktop_window)
keep gpu_plan set nivren_gpu.AddPlan([1, 2], [3, 4], nivren_gpu.ComputeLimits(8, 4))
keep gpu_ok set choose nivren_gpu.compile_add(gpu_plan) { case Ok carries value => value.cpu_fallback == [4, 6], case Err carries problem => no }
keep stats_ok set nivren_stats.mean([1.0, 2.0, 3.0]) == ok(2.0)
keep jwt_secret set std.bytes.from_string("official package test secret")
keep jwt_token set nivren_jwt.sign_hs256("{\"sub\":\"42\"}", jwt_secret)
keep jwt_ok set choose jwt_token { case Ok carries token => nivren_jwt.verify_hs256(token, jwt_secret) == ok("{\"sub\":\"42\"}"), case Err carries problem => no }
keep aws_signed set nivren_aws.sign_v4("GET", "/", "Action=ListUsers&Version=2010-05-08", "content-type:application/x-www-form-urlencoded; charset=utf-8\nhost:iam.amazonaws.com\nx-amz-date:20150830T123600Z", "content-type;host;x-amz-date", "", "20150830T123600Z", "20150830", "us-east-1", "iam", "AKIDEXAMPLE", std.bytes.from_string("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY"))
keep aws_ok set choose aws_signed { case Ok carries value => value.canonical_request_hash == "f536975d06c0309214f805bb90ccff089219ecd68b2577efef23edd43b7e1a59", case Err carries problem => no }
keep matrix_value set nivren_matrix.matrix(1, 2, [2.0, 3.0])
keep matrix_ok set choose matrix_value { case Ok carries value => nivren_matrix.transpose(value).values == [2.0, 3.0], case Err carries problem => no }
keep columnar_value set nivren_columnar.table([nivren_columnar.Column.Ints(nivren_columnar.IntColumn("id", [1, 2]))])
keep columnar_ok set choose columnar_value { case Ok carries value => value.rows == 2, case Err carries problem => no }
keep svg_value set nivren_svg.canvas(16, 16)
keep svg_ok set choose svg_value { case Ok carries value => choose nivren_svg.render(value) { case Ok carries text => text != "", case Err carries problem => no }, case Err carries problem => no }
keep wav_value set nivren_wav.encode_pcm16(nivren_wav.Audio(48000, 1, [0, 42]))
keep wav_ok set choose wav_value { case Ok carries bytes => choose nivren_wav.decode_pcm16(bytes) { case Ok carries audio => audio.samples == [0, 42], case Err carries problem => no }, case Err carries problem => no }
keep image_pixels set std.bytes.from_values([255, 0, 0])
keep image_ok set choose image_pixels { case Ok carries pixels => choose nivren_image.image(1, 1, pixels) { case Ok carries value => choose nivren_image.encode_ppm(value) { case Ok carries bytes => nivren_image.decode_ppm(bytes) == ok(value), case Err carries problem => no }, case Err carries problem => no }, case Err carries problem => no }
keep metric_value set nivren_metrics.sample("nivren_ready", "Readiness", "gauge", 1.0, std.map.of("edition", "3"))
keep metrics_ok set choose metric_value { case Ok carries value => choose nivren_metrics.encode([value]) { case Ok carries text => text != "", case Err carries problem => no }, case Err carries problem => no }
keep trace_value set nivren_trace.parse("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01")
keep trace_ok set choose trace_value { case Ok carries value => nivren_trace.traceparent(value) == ok("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"), case Err carries problem => no }
keep oidc_ok set nivren_oidc.pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk") == ok("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
keep password_hash set nivren_secrets.hash_password_with_salt("secret", std.bytes.from_string("0123456789abcdef"))
keep secrets_ok set choose password_hash { case Ok carries hash => nivren_secrets.verify_password("secret", hash) == ok(yes), case Err carries problem => no }
keep aead_key set nivren_aead.import_key(std.bytes.from_string("0123456789abcdef0123456789abcdef"))
keep aead_nonce set std.bytes.from_string("unique-nonce")
keep aead_context set std.bytes.from_string("record:42")
keep aead_ok set choose aead_key {
    case Ok carries key => choose nivren_aead.seal_with_nonce(key, aead_nonce, aead_context, std.bytes.from_string("Nivren")) {
        case Ok carries value => nivren_aead.unseal(key, aead_context, value) == ok(std.bytes.from_string("Nivren")),
        case Err carries problem => no,
    },
    case Err carries problem => no,
}
keep checked set nivren_testing.expect_yes(selected != none and encoded != "" and crypto_ok and sql_ok and redis_ok and compression_ok and csv_ok and database_ok and desktop_ok and gpu_ok and stats_ok and jwt_ok and aws_ok and matrix_ok and columnar_ok and svg_ok and wav_ok and image_ok and metrics_ok and trace_ok and oidc_ok and secrets_ok and aead_ok, "official packages")
keep port set nivren_validation.range("port", 443, 1, 65535)
choose checked {
    case Ok carries value => choose port { case Ok carries number => number, case Err carries problem => 0 },
    case Err carries problem => 0,
}
"#,
    )
    .unwrap();
    let manifest = nivren::project::Manifest::load(&consumer).unwrap();
    assert_eq!(
        nivren::package::install_dependencies(&manifest, &registry).unwrap(),
        25
    );
    let program = nivren::modules::load_project(&manifest.root, &manifest.entry_path()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    assert_eq!(
        nivren::runtime::Interpreter::new().run(&program).unwrap(),
        Value::Int(443)
    );
    let chunk = nivren::bytecode::compile(&program).unwrap();
    assert_eq!(
        nivren::runtime::Interpreter::new()
            .run_bytecode(&chunk)
            .unwrap(),
        Value::Int(443)
    );

    fs::remove_dir_all(registry).unwrap();
    fs::remove_dir_all(consumer).unwrap();
}

#[test]
fn official_image_codec_is_bounded_in_both_engines() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packages/nivren_image");
    let program = nivren::modules::load_project(&root, &root.join("tests/core_test.niv")).unwrap();
    nivren::typecheck::check(&program).unwrap();
    assert_eq!(
        nivren::runtime::Interpreter::new()
            .with_instruction_limit(1_000_000)
            .run(&program)
            .unwrap(),
        Value::Ok(Arc::new(Value::Bool(true)))
    );
    let chunk = nivren::bytecode::compile(&program).unwrap();
    assert_eq!(
        nivren::runtime::Interpreter::new()
            .with_instruction_limit(1_000_000)
            .run_bytecode(&chunk)
            .unwrap(),
        Value::Ok(Arc::new(Value::Bool(true)))
    );
}

#[test]
fn hot_integer_functions_tier_to_native_code_with_checked_overflow() {
    let source = "define twice_sum takes { a is Int, b is Int } gives Int { keep sum set a + b; give sum * 2; } twice_sum(1, 2); twice_sum(20, 1)";
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

    let overflow = "define add takes { a is Int, b is Int } gives Int { give a + b; } add(9223372036854775807, 1)";
    let program = nivren::parser::parse(nivren::lexer::scan(overflow).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let mut interpreter = nivren::runtime::Interpreter::new();
    interpreter.set_jit_threshold(1);
    let error = interpreter.run_bytecode(&chunk).unwrap_err();
    assert!(error.message.contains("integer overflow"));
}

#[test]
fn native_tier_supports_argument_lists_larger_than_the_inline_fast_path() {
    let source = "define sum9 takes { a is Int, b is Int, c is Int, d is Int, e is Int, f is Int, g is Int, h is Int, i is Int } gives Int { give a + b + c + d + e + f + g + h + i; } sum9(1, 2, 3, 4, 5, 6, 7, 8, 9)";
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let mut interpreter = nivren::runtime::Interpreter::new();
    interpreter.set_jit_threshold(1);
    assert_eq!(interpreter.run_bytecode(&chunk).unwrap(), Value::Int(45));
    assert_eq!(interpreter.jit_stats().executions, 1);
}

#[test]
fn integer_call_frames_preserve_recursion_and_mutable_locals_before_jit() {
    let source = "define fibonacci takes { value is Int } gives Int { when value < 2 { give value; } give fibonacci(value - 1) + fibonacci(value - 2); } define adjust takes { value is Int } gives Int { change result set value; result = result + 2; give result; } fibonacci(10) + adjust(40)";
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let mut interpreter = nivren::runtime::Interpreter::new();
    interpreter.set_jit_threshold(u32::MAX);
    assert_eq!(interpreter.run_bytecode(&chunk).unwrap(), Value::Int(97));
    assert_eq!(interpreter.jit_stats().executions, 0);
}

#[test]
fn general_call_frames_preserve_values_and_lexical_shadowing() {
    let source = r#"
shape Sample { label is String, enabled is Bool }
keep label set "outer"
define inspect takes { sample is Sample } gives String {
    when sample.enabled {
        keep label set "inner"
        assert(label == "inner", "nested slot")
    }
    give label + sample.label
}
inspect(Sample("!", yes)) + inspect(Sample("?", no))
"#;
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let mut interpreter = nivren::runtime::Interpreter::new();
    interpreter.set_jit_threshold(u32::MAX);
    assert_eq!(
        interpreter.run_bytecode(&chunk).unwrap(),
        Value::String("outer!outer?".into())
    );
}

#[test]
fn fast_frames_do_not_leak_into_general_callees() {
    let source = r#"
define unwrap takes { value is Result<Int, String> } gives Int {
    give choose value {
        case Ok carries number => number,
        case Err carries problem => 0
    }
}
define wrapper takes { value is Int } gives Int {
    keep adjusted set value + 2
    give unwrap(ok(adjusted))
}
wrapper(40)
"#;
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let mut interpreter = nivren::runtime::Interpreter::new();
    interpreter.set_jit_threshold(u32::MAX);
    assert_eq!(interpreter.run_bytecode(&chunk).unwrap(), Value::Int(42));
}

#[test]
fn integer_root_slots_preserve_bindings_between_bytecode_runs() {
    let mut interpreter = nivren::runtime::Interpreter::new();
    let define = nivren::parser::parse(
        nivren::lexer::scan("change answer set 40; answer = answer + 2; answer").unwrap(),
    )
    .unwrap();
    nivren::typecheck::check(&define).unwrap();
    let define = nivren::bytecode::compile(&define).unwrap();
    assert_eq!(interpreter.run_bytecode(&define).unwrap(), Value::Int(42));

    let load = nivren::parser::parse(nivren::lexer::scan("answer").unwrap()).unwrap();
    let load = nivren::bytecode::compile(&load).unwrap();
    assert_eq!(interpreter.run_bytecode(&load).unwrap(), Value::Int(42));

    let redeclare =
        nivren::parser::parse(nivren::lexer::scan("change answer set 0").unwrap()).unwrap();
    let redeclare = nivren::bytecode::compile(&redeclare).unwrap();
    assert!(interpreter.run_bytecode(&redeclare).is_err());
}

#[test]
fn cli_emits_linkable_native_aot_objects_for_safe_integer_functions() {
    let directory = module_fixture("native-aot");
    fs::create_dir_all(directory.join("src")).unwrap();
    fs::write(
        directory.join("niv.toml"),
        "[package]\nname = \"native-aot\"\nversion = \"1.0.0\"\nentry = \"src/main.niv\"\n",
    )
    .unwrap();
    fs::write(
        directory.join("src/main.niv"),
        "define double takes { value is Int } gives Int { give value * 2 }\ndouble(21)",
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_niv"))
        .args(["build", "--aot", directory.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let extension = if cfg!(windows) { "obj" } else { "o" };
    let object = directory.join(format!("target/aot/double.{extension}"));
    assert!(fs::metadata(&object).unwrap().len() > 64);
    let program_object = directory.join(format!("target/aot/program.{extension}"));
    let first_program = fs::read(&program_object).unwrap();
    assert!(first_program.len() > 64);
    assert!(directory.join("target/aot/program.nivb").is_file());
    assert!(directory.join("target/aot/program.json").is_file());
    assert!(directory.join("target/aot/nivren_program.h").is_file());
    let repeated = std::process::Command::new(env!("CARGO_BIN_EXE_niv"))
        .args(["build", "--aot", directory.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(repeated.status.success());
    assert_eq!(fs::read(&program_object).unwrap(), first_program);

    #[cfg(unix)]
    {
        let host = directory.join("host.c");
        let executable = directory.join("host");
        fs::write(
            &host,
            "#include <stdint.h>\nextern int64_t nivren_double(const int64_t*, uint8_t*);\nint main(void){int64_t a[1]={21};uint8_t o=0;return nivren_double(a,&o)==42&&o==0?0:1;}\n",
        )
        .unwrap();
        let linked = std::process::Command::new("cc")
            .args([
                host.to_str().unwrap(),
                object.to_str().unwrap(),
                "-o",
                executable.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            linked.status.success(),
            "{}",
            String::from_utf8_lossy(&linked.stderr)
        );
        assert!(
            std::process::Command::new(executable)
                .status()
                .unwrap()
                .success()
        );

        let trace_host = directory.join("trace_host.c");
        let trace_executable = directory.join("trace_host");
        let metadata: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.join("target/aot/program.json")).unwrap())
                .unwrap();
        let instructions = metadata["instructions"].as_u64().unwrap();
        fs::write(
            &trace_host,
            format!(
                "#include <stdint.h>\ntypedef int64_t(*Callback)(void*,uint64_t);\nextern int64_t nivren_program(void*,Callback);\nstruct State{{uint64_t visited;}};\nstatic int64_t step(void* raw,uint64_t pc){{struct State* state=(struct State*)raw;state->visited++;return pc+1=={instructions}u?-1:(int64_t)(pc+1);}}\nint main(void){{struct State state={{0}};return nivren_program(&state,step)==-1&&state.visited=={instructions}u?0:1;}}\n"
            ),
        )
        .unwrap();
        let linked = std::process::Command::new("cc")
            .args([
                trace_host.to_str().unwrap(),
                program_object.to_str().unwrap(),
                "-o",
                trace_executable.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            linked.status.success(),
            "{}",
            String::from_utf8_lossy(&linked.stderr)
        );
        assert!(
            std::process::Command::new(trace_executable)
                .status()
                .unwrap()
                .success()
        );
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn complete_program_aot_does_not_require_an_integer_kernel() {
    let directory = module_fixture("complete-native-aot");
    fs::create_dir_all(directory.join("src")).unwrap();
    fs::write(
        directory.join("niv.toml"),
        "[package]\nname = \"complete-native-aot\"\nversion = \"1.0.0\"\nentry = \"src/main.niv\"\n",
    )
    .unwrap();
    fs::write(
        directory.join("src/main.niv"),
        "shape Greeting { text is String }\nGreeting(\"hello\").text",
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_niv"))
        .args(["build", "--aot", directory.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("optimized-kernels 0"));
    let extension = if cfg!(windows) { "obj" } else { "o" };
    assert!(
        directory
            .join(format!("target/aot/program.{extension}"))
            .is_file()
    );
    fs::remove_dir_all(directory).unwrap();
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
    let ownership: serde_json::Value =
        serde_json::from_slice(&fs::read(registry.join("v1/owners/trusted.json")).unwrap())
            .unwrap();
    assert_eq!(ownership["package"], "trusted");
    assert_eq!(ownership["publisher"], "team");
    let response = nivren::registry_server::handle_request_for_test(
        b"GET /v1/packages/trusted/1.0.0.nivpkg HTTP/1.1\r\nHost: registry.test\r\n\r\n",
        &registry,
        1_100,
        1,
    );
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
    assert!(response.ends_with(&package));
    let response = nivren::registry_server::handle_request_for_test(
        b"GET /v1/search/trust HTTP/1.1\r\nHost: registry.test\r\n\r\n",
        &registry,
        1_100,
        1,
    );
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
    assert!(String::from_utf8_lossy(&response).contains("\"name\":\"trusted\""));
    let yank = nivren::trust::sign_admin_action(
        root_secret,
        nivren::trust::RegistryAdminAction {
            format: 1,
            action: "yank".into(),
            package: "trusted".into(),
            version: "1.0.0".into(),
            generation: 1,
            issued_at: 1_000,
            expires_at: 2_000,
            reason: "release validation incident".into(),
            signature: String::new(),
        },
    )
    .unwrap();
    assert!(nivren::trust::verify_admin_action(&yank, root_public, 1_100, 0).is_ok());
    assert!(
        nivren::trust::verify_admin_action(&yank, root_public, 1_100, 1)
            .unwrap_err()
            .message
            .contains("replayed")
    );
    let yank_json = serde_json::to_vec(&yank).unwrap();
    let mut admin_request = format!(
        "POST /v1/admin HTTP/1.1\r\nHost: registry.test\r\nContent-Type: application/vnd.nivren.admin-v1+json\r\nContent-Length: {}\r\n\r\n",
        yank_json.len()
    )
    .into_bytes();
    admin_request.extend_from_slice(&yank_json);
    let response =
        nivren::registry_server::handle_request_for_test(&admin_request, &registry, 1_100, 0);
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
    let response = nivren::registry_server::handle_request_for_test(
        b"GET /v1/search/trust HTTP/1.1\r\nHost: registry.test\r\n\r\n",
        &registry,
        1_100,
        0,
    );
    assert!(!String::from_utf8_lossy(&response).contains("\"name\":\"trusted\""));
    let replay =
        nivren::registry_server::handle_request_for_test(&admin_request, &registry, 1_100, 0);
    assert!(replay.starts_with(b"HTTP/1.1 422 Unprocessable Content\r\n"));
    let audit = nivren::registry_server::handle_request_for_test(
        b"GET /v1/admin/1.json HTTP/1.1\r\nHost: registry.test\r\n\r\n",
        &registry,
        1_100,
        0,
    );
    assert!(audit.starts_with(b"HTTP/1.1 200 OK\r\n"));
    assert!(String::from_utf8_lossy(&audit).contains("release validation incident"));
    fs::write(registry.join("admin.reason"), "operator validation\n").unwrap();
    fs::write(registry.join("root.secret"), hex(&root_secret)).unwrap();
    fs::write(registry.join("root.public"), hex(&root_public)).unwrap();
    let signed_path = registry.join("signed-admin.json");
    let signed = Command::new(env!("CARGO_BIN_EXE_niv"))
        .args([
            "registry",
            "sign-admin",
            "unyank",
            "trusted",
            "1.0.0",
            "2",
            "1000",
            "2000",
            registry.join("admin.reason").to_str().unwrap(),
            registry.join("root.secret").to_str().unwrap(),
            signed_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        signed.status.success(),
        "{}",
        String::from_utf8_lossy(&signed.stderr)
    );
    let verified = Command::new(env!("CARGO_BIN_EXE_niv"))
        .args([
            "registry",
            "verify-admin",
            signed_path.to_str().unwrap(),
            registry.join("root.public").to_str().unwrap(),
            "1100",
            "1",
        ])
        .output()
        .unwrap();
    assert!(verified.status.success());
    fs::write(
        registry.join("v1/admin/pending.json"),
        fs::read(&signed_path).unwrap(),
    )
    .unwrap();
    let recovered = Command::new(env!("CARGO_BIN_EXE_niv"))
        .args([
            "registry",
            "recover-admin",
            registry.to_str().unwrap(),
            "1100",
            "0",
        ])
        .output()
        .unwrap();
    assert!(
        recovered.status.success(),
        "{}",
        String::from_utf8_lossy(&recovered.stderr)
    );
    assert!(!registry.join("v1/admin/pending.json").exists());
    let response = nivren::registry_server::handle_request_for_test(
        b"GET /v1/search/trust HTTP/1.1\r\nHost: registry.test\r\n\r\n",
        &registry,
        1_100,
        0,
    );
    assert!(String::from_utf8_lossy(&response).contains("\"name\":\"trusted\""));
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

#[test]
fn stop_ends_the_nearest_repeat_loop_in_both_engines() {
    let source = r#"
change total set 0
change value set 0
repeat while value < 10 {
    change value to value + 1
    when value == 4 { stop }
    change total to total + value
}
total
"#;
    assert_eq!(eval_tree(source), Value::Int(6));
    assert_eq!(eval_vm(source), Value::Int(6));
}

#[test]
fn skip_ends_only_the_current_loop_pass_in_both_engines() {
    let source = r#"
change total set 0
change value set 0
repeat while value < 6 {
    change value to value + 1
    when value == 2 { skip }
    change total to total + value
}
total
"#;
    assert_eq!(eval_tree(source), Value::Int(19));
    assert_eq!(eval_vm(source), Value::Int(19));
}

#[test]
fn stop_and_skip_control_each_loops_in_both_engines() {
    let stop_source = r#"
change total set 0
each item in [1, 2, 3, 4, 5] {
    when item == 4 { stop }
    change total to total + item
}
total
"#;
    assert_eq!(eval_tree(stop_source), Value::Int(6));
    assert_eq!(eval_vm(stop_source), Value::Int(6));
    let skip_source = r#"
change total set 0
each item in [1, 2, 3, 4, 5] {
    when item == 2 { skip }
    change total to total + item
}
total
"#;
    assert_eq!(eval_tree(skip_source), Value::Int(13));
    assert_eq!(eval_vm(skip_source), Value::Int(13));
}

#[test]
fn stop_only_ends_the_innermost_loop() {
    let source = r#"
change total set 0
each outer in [1, 2, 3] {
    each inner in [1, 2, 3] {
        when inner == 2 { stop }
        change total to total + inner
    }
    change total to total + 10
}
total
"#;
    assert_eq!(eval_tree(source), Value::Int(33));
    assert_eq!(eval_vm(source), Value::Int(33));
}

#[test]
fn loop_exits_outside_a_loop_are_rejected() {
    let errors = nivren::check("stop").unwrap_err();
    assert!(errors[0].to_string().contains("no 'repeat' or 'each' loop"));
    let errors = nivren::check("skip").unwrap_err();
    assert!(errors[0].to_string().contains("no 'repeat' or 'each' loop"));
}

#[test]
fn loop_exits_cannot_cross_function_or_using_boundaries() {
    let function_source = r#"
each item in [1] {
    define helper {
        skip
    }
}
"#;
    let errors = nivren::check(function_source).unwrap_err();
    assert!(errors[0].to_string().contains("function boundary"));
    let using_source = r#"
each item in [1] {
    using thing set append([], 1) {
        stop
    }
}
"#;
    let errors = nivren::check(using_source).unwrap_err();
    assert!(errors[0].to_string().contains("'using' scope"));
}

#[test]
fn contextual_stop_and_skip_leave_library_names_untouched() {
    let source = r#"
keep source set std.iter.from([1, 2, 3, 4])
keep advanced set std.iter.skip(source, 2)
len(std.iter.collect(advanced))
"#;
    assert_eq!(eval_tree(source), Value::Int(2));
    assert_eq!(eval_vm(source), Value::Int(2));
}

#[test]
fn when_carries_unwraps_present_maybe_values_in_both_engines() {
    let source = r#"
keep table set std.map.of(1, "one")
change seen set ""
when std.map.get(table, 1) carries value {
    change seen to value
} otherwise {
    change seen to "missing"
}
seen
"#;
    assert_eq!(eval_tree(source), Value::String("one".into()));
    assert_eq!(eval_vm(source), Value::String("one".into()));
}

#[test]
fn when_carries_takes_the_otherwise_branch_for_none_in_both_engines() {
    let source = r#"
keep table set std.map.of(1, "one")
change seen set ""
when std.map.get(table, 2) carries value {
    change seen to value
} otherwise {
    change seen to "missing"
}
seen
"#;
    assert_eq!(eval_tree(source), Value::String("missing".into()));
    assert_eq!(eval_vm(source), Value::String("missing".into()));
}

#[test]
fn when_carries_bindings_stay_inside_the_matched_branch() {
    let source = r#"
keep table set std.map.of(1, "one")
when std.map.get(table, 1) carries value {
    show value
}
show value
"#;
    let errors = nivren::check(source).unwrap_err();
    assert!(errors[0].to_string().contains("value"));
}

#[test]
fn when_carries_rejects_subjects_that_are_not_maybe_values() {
    let errors = nivren::check("when 5 carries x { show(x) }").unwrap_err();
    assert!(errors[0].to_string().contains("maybe"));
}

#[test]
fn loop_exits_pass_through_when_carries_branches_in_both_engines() {
    let source = r#"
keep table set std.map.of(3, "three")
change total set 0
each item in [1, 2, 3, 4] {
    when std.map.get(table, item) carries found {
        stop
    }
    change total to total + item
}
total
"#;
    assert_eq!(eval_tree(source), Value::Int(3));
    assert_eq!(eval_vm(source), Value::Int(3));
}

#[test]
fn text_literals_interpolate_values_in_both_engines() {
    let source = r#"
keep name set "world"
text "Hello {name}, {1 + 2} and {yes}!"
"#;
    let expected = Value::String("Hello world, 3 and yes!".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
}

#[test]
fn text_literals_escape_braces_and_allow_nested_hole_braces() {
    let source = r#"text "{{a}} has {len([1, 2, 3])} items""#;
    let expected = Value::String("{a} has 3 items".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
}

#[test]
fn text_holes_reject_values_without_canonical_text() {
    let errors = nivren::check(r#"text "{[1, 2]}""#).unwrap_err();
    assert!(errors[0].to_string().contains("text hole"));
}

#[test]
fn text_holes_reject_perform_boundaries() {
    let errors = nivren::check(r#"text "{perform 1}""#).unwrap_err();
    assert!(errors[0].to_string().contains("pure"));
}

#[test]
fn text_holes_reject_empty_and_unterminated_forms() {
    assert!(nivren::check(r#"text "{}""#).is_err());
    assert!(nivren::check(r#"text "{name""#).is_err());
    assert!(nivren::check(r#"text "name}""#).is_err());
}

#[test]
fn text_stays_an_ordinary_identifier_without_a_string() {
    let source = r#"
keep text set 5
text + 1
"#;
    assert_eq!(eval_tree(source), Value::Int(6));
    assert_eq!(eval_vm(source), Value::Int(6));
}

#[test]
fn choose_guards_select_arms_with_bound_values_in_both_engines() {
    let source = r#"
choice Size holds {
    case Small
    case Large carries Int
}
define describe takes { value is Size } gives String {
    give choose value {
        case Large carries amount when amount > 10 => "big"
        case Large carries amount => "large"
        case Small => "small"
    }
}
keep first set describe with { value set Size.Large(25) }
keep second set describe with { value set Size.Large(5) }
keep third set describe with { value set Size.Small }
text "{first} {second} {third}"
"#;
    let expected = Value::String("big large small".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
}

#[test]
fn choose_or_patterns_join_cases_in_both_engines() {
    let source = r#"
choice Color holds {
    case Red
    case Green
    case Blue
}
choose Color.Green {
    case Red or Green => "warm"
    case Blue => "cool"
}
"#;
    let expected = Value::String("warm".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
}

#[test]
fn choose_matches_literals_with_an_otherwise_arm_in_both_engines() {
    let source = r#"
choose 3 {
    case 1 => "one"
    case 2 => "two"
    otherwise as number => std.int.format(number)
}
"#;
    let expected = Value::String("3".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
}

#[test]
fn choose_matches_nested_shape_patterns_in_both_engines() {
    let source = r#"
shape Point holds {
    x is Int
    y is Int
}
keep origin set Point with { x set 0, y set 0 }
choose origin {
    case Point holds { x set 0, y set 0 } => "origin"
    case Point holds { x set x } => std.int.format(x)
}
"#;
    let expected = Value::String("origin".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
}

#[test]
fn choose_matches_case_payloads_with_nested_patterns_in_both_engines() {
    let source = r#"
shape Point holds {
    x is Int
    y is Int
}
choice Placement holds {
    case At carries Point
}
choose Placement.At(Point with { x set 0, y set 7 }) {
    case At carries Point holds { x set 0, y set y } => std.int.format(y)
    case At carries any => "elsewhere"
}
"#;
    let expected = Value::String("7".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
}

#[test]
fn choose_exhaustiveness_rules_guard_the_new_patterns() {
    let errors = nivren::check(r#"choose 5 { case 1 => "one" }"#).unwrap_err();
    assert!(errors[0].to_string().contains("otherwise"));
    let errors = nivren::check(
        r#"
choose 5 {
    otherwise => 1
    case 2 => 2
}
"#,
    )
    .unwrap_err();
    assert!(errors[0].to_string().contains("unreachable"));
    let errors = nivren::check(
        r#"
choice Color holds {
    case Red
    case Blue
}
choose Color.Red {
    case Red => 1
    case Red => 2
    case Blue => 3
}
"#,
    )
    .unwrap_err();
    assert!(errors[0].to_string().contains("duplicate"));
    let errors = nivren::check(
        r#"
choice Color holds {
    case Red
    case Blue
}
choose Color.Red {
    case Red => 1
    case Blue => 2
    otherwise => 3
}
"#,
    )
    .unwrap_err();
    assert!(errors[0].to_string().contains("already exhaustive"));
}

#[test]
fn choose_rejects_unsafe_or_impure_pattern_forms() {
    let errors = nivren::check(
        r#"
choose 1.5 {
    case 1.5 => 1
    otherwise => 2
}
"#,
    )
    .unwrap_err();
    assert!(errors[0].to_string().contains("safe selector"));
    let errors = nivren::check(
        r#"
choose 5 {
    case any when perform 1 => 1
    otherwise => 2
}
"#,
    )
    .unwrap_err();
    assert!(errors[0].to_string().contains("pure"));
    let errors = nivren::check(
        r#"
choice Size holds {
    case Small
    case Large carries Int
}
choose Size.Small {
    case Large carries amount or Small => 1
}
"#,
    )
    .unwrap_err();
    assert!(errors[0].to_string().contains("same names"));
}

#[test]
fn keep_destructures_shape_patterns_in_both_engines() {
    let source = r#"
shape Point holds {
    x is Int
    y is Int
}
keep Point holds { x set x, y set y } set Point with { x set 3, y set 4 }
x * 10 + y
"#;
    assert_eq!(eval_tree(source), Value::Int(34));
    assert_eq!(eval_vm(source), Value::Int(34));
}

#[test]
fn each_destructures_shape_patterns_in_both_engines() {
    let source = r#"
shape Row holds {
    id is Int
    label is String
}
change total set 0
each Row holds { id set id } in [
    Row with { id set 1, label set "one" },
    Row with { id set 2, label set "two" }
] {
    change total to total + id
}
total
"#;
    assert_eq!(eval_tree(source), Value::Int(3));
    assert_eq!(eval_vm(source), Value::Int(3));
}

#[test]
fn binding_patterns_must_be_irrefutable() {
    let source = r#"
shape Point holds {
    x is Int
    y is Int
}
keep Point holds { x set 0 } set Point with { x set 1, y set 2 }
"#;
    let errors = nivren::check(source).unwrap_err();
    assert!(errors[0].to_string().contains("never fails"));
}

#[test]
fn when_carries_tests_choice_cases_with_patterns_in_both_engines() {
    let source = r#"
choice Signal holds {
    case Quit
    case Retry carries Int
    case Note carries String
}
define describe takes { value is Signal } gives String {
    when value carries Retry carries delay {
        give text "retry {delay}"
    }
    when value carries Quit or Note carries any {
        give "handled"
    }
    give "other"
}
keep first set describe with { value set Signal.Retry(3) }
keep second set describe with { value set Signal.Quit }
keep third set describe with { value set Signal.Note("hi") }
text "{first} {second} {third}"
"#;
    let expected = Value::String("retry 3 handled handled".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
}

#[test]
fn promise_never_rejects_renounced_needs_and_calls() {
    let declaration = r#"
promise never Network
define fetch takes { address is String } gives String or String needs Network {
    give std.web.get(address, 1.0)
}
"#;
    let errors = nivren::check(declaration).unwrap_err();
    assert!(errors[0].to_string().contains("promise never Network"));
    let call = r#"
promise never Time
std.time.sleep(0.1)
"#;
    let errors = nivren::check(call).unwrap_err();
    assert!(errors[0].to_string().contains("promise never Time"));
}

#[test]
fn promise_only_within_confines_declared_scopes() {
    let outside = r#"
promise FileRead only within "path:./data"
define read_config takes { } gives String or String needs FileRead within "path:./other" {
    give std.files.read("./other/config")
}
"#;
    let errors = nivren::check(outside).unwrap_err();
    assert!(
        errors[0]
            .to_string()
            .contains("outside the promised boundaries")
    );
    let inside = r#"
promise FileRead only within "path:./data"
define read_config takes { } gives String or String needs FileRead within "path:./data" {
    give std.files.read("./data/config")
}
"#;
    assert!(nivren::check(inside).is_ok());
}

#[test]
fn promises_bind_only_their_enclosing_scope() {
    let source = r#"
{
    promise never Time
}
std.time.sleep(0.1)
"#;
    assert!(nivren::check(source).is_ok());
    let errors = nivren::check("promise never Magic").unwrap_err();
    assert!(errors[0].to_string().contains("not a capability"));
}

#[test]
fn promise_statements_run_as_quiet_declarations_in_both_engines() {
    let source = r#"
promise never Network
1 + 2
"#;
    assert_eq!(eval_tree(source), Value::Int(3));
    assert_eq!(eval_vm(source), Value::Int(3));
}

#[test]
fn samples_stay_quiet_in_ordinary_runs_and_execute_under_test_mode() {
    let failing = r#"
sample "adding" {
    1 + 1
} shows "3"
42
"#;
    assert_eq!(eval_tree(failing), Value::Int(42));
    assert_eq!(eval_vm(failing), Value::Int(42));
    let program = nivren::parser::parse(nivren::lexer::scan(failing).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let mut interpreter = nivren::runtime::Interpreter::new();
    interpreter.enable_samples();
    let error = interpreter.run_bytecode(&chunk).unwrap_err();
    assert!(error.to_string().contains("sample 'adding'"));
    let passing = failing.replace("shows \"3\"", "shows \"2\"");
    let program = nivren::parser::parse(nivren::lexer::scan(&passing).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let mut interpreter = nivren::runtime::Interpreter::new();
    interpreter.enable_samples();
    assert_eq!(interpreter.run_bytecode(&chunk).unwrap(), Value::Int(42));
    let mut interpreter = nivren::runtime::Interpreter::new();
    interpreter.enable_samples();
    assert_eq!(interpreter.run(&program).unwrap(), Value::Int(42));
}

#[test]
fn samples_are_checked_hermetic_and_uniquely_titled() {
    let errors = nivren::check("sample \"a\" { 1 }\nsample \"a\" { 2 }\nnone").unwrap_err();
    assert!(errors[0].to_string().contains("duplicate sample title"));
    let errors = nivren::check("sample \"io\" { std.time.sleep(0.1) }\nnone").unwrap_err();
    assert!(errors[0].to_string().contains("renounces"));
    let errors = nivren::check("sample \"x\" { keep a set 1 } shows \"1\"\nnone").unwrap_err();
    assert!(errors[0].to_string().contains("ends with one expression"));
}

#[test]
fn the_grown_text_library_builds_and_transforms_in_both_engines() {
    let source = r#"
define build takes { } gives String or String {
    keep sliced set std.text.slice("hello", 1, 3) or give
    keep replaced set std.text.replace("a-b-a", "a", "x", 2) or give
    keep upper set std.text.to_upper("up") or give
    keep padded set std.text.pad_start("7", 3, "0") or give
    keep repeated set std.text.repeat("ab", 2) or give
    give std.text.join([sliced, replaced, upper, padded, repeated], "|")
}
choose build with {} {
    case Ok carries value => value
    case Err carries message => message
}
"#;
    let expected = Value::String("el|x-b-x|UP|007|abab".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
}

#[test]
fn the_grown_text_library_tests_and_measures_in_both_engines() {
    let source = r#"
change passes set 0
each fact in [
    std.text.contains("hello", "ell"),
    std.text.ends_with("hello", "lo"),
    (std.text.index_of("hello", "l") ?? 0) == 2,
    len(std.text.lines("a\nb\r\nc")) == 3,
    std.text.trim_start("  x") == "x",
    std.text.trim_end("x  ") == "x",
    std.text.trim("  mid  ") == "mid"
] {
    when fact { change passes to passes + 1 }
}
passes
"#;
    assert_eq!(eval_tree(source), Value::Int(7));
    assert_eq!(eval_vm(source), Value::Int(7));
}

#[test]
fn calendar_fields_and_difference_read_datetimes_in_both_engines() {
    let source = r#"
define inspect takes { } gives String or String {
    keep opening set std.time.from_unix(0, "UTC") or give
    keep later set std.time.add_seconds(opening, 90061) or give
    keep difference set std.time.difference_seconds(later, opening) or give
    give std.text.join([
        std.int.format(std.time.year(opening)),
        std.int.format(std.time.month(opening)),
        std.int.format(std.time.day(opening)),
        std.int.format(std.time.weekday(opening)),
        std.int.format(std.time.hour(later)),
        std.int.format(std.time.minute(later)),
        std.int.format(std.time.second(later)),
        std.int.format(difference)
    ], ":")
}
choose inspect with {} {
    case Ok carries value => value
    case Err carries message => message
}
"#;
    let expected = Value::String("1970:1:1:4:1:1:1:90061".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
}

#[test]
fn monotonic_time_never_goes_backward() {
    let source = r#"
keep first set std.time.monotonic()
keep second set std.time.monotonic()
second >= first
"#;
    assert_eq!(eval_tree(source), Value::Bool(true));
    assert_eq!(eval_vm(source), Value::Bool(true));
}

#[test]
fn intent_stories_render_deterministic_plain_language() {
    let source = r#"
define fetch takes { path is String } gives String or String needs FileRead {
    give perform std.files.read(path)
}
perform fetch with { path set "notes.txt" }
"#;
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let graph = nivren::intent::analyze(&program, nivren::intent::Optimization::Enabled);
    graph.validate().unwrap();
    let first = graph.story();
    assert_eq!(first, graph.story());
    assert!(first.contains("reads through std.files.read"));
    assert!(first.contains("needs FileRead"));
    assert!(first.contains("visible effect boundary"));
    assert!(first.contains("required capabilities: FileRead"));
    let pure = nivren::parser::parse(nivren::lexer::scan("1 + 2").unwrap()).unwrap();
    let graph = nivren::intent::analyze(&pure, nivren::intent::Optimization::Enabled);
    assert!(graph.story().contains("pure computation"));
}

#[test]
fn effects_record_and_replay_byte_identically() {
    let source = r#"
keep moment set std.time.now_zoned("UTC")
choose moment {
    case Ok carries value => std.time.format(value)
    case Err carries message => message
}
"#;
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let mut recording = nivren::runtime::Interpreter::new();
    let recorder = recording.record_effects();
    let first = recording.run_bytecode(&chunk).unwrap();
    let entries = recorder.lock().unwrap().clone();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].capability, "Time");
    let mut replaying = nivren::runtime::Interpreter::new();
    replaying.replay_effects(entries.clone());
    assert_eq!(replaying.run_bytecode(&chunk).unwrap(), first);
    assert_eq!(replaying.replay_remaining(), 0);
    let mut replaying_tree = nivren::runtime::Interpreter::new();
    replaying_tree.replay_effects(entries.clone());
    assert_eq!(replaying_tree.run(&program).unwrap(), first);
    let divergent = r#"std.env.get("NIVREN_REPLAY_TEST")"#;
    let program = nivren::parser::parse(nivren::lexer::scan(divergent).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let mut replaying = nivren::runtime::Interpreter::new();
    replaying.replay_effects(entries);
    let error = replaying.run_bytecode(&chunk).unwrap_err();
    assert!(error.to_string().contains("replay diverged"));
}

#[test]
fn portable_plans_encode_and_decode_across_programs_in_both_engines() {
    let source = r#"
shape FetchPlan holds {
    address is String
    attempts is Int
} derives Json
define round_trip takes { } gives String or String {
    prepare request as FetchPlan with { address set "example.com", attempts set 3 }
    keep encoded set std.plans.encode(request) or give
    keep decoded set std.plans.decode(FetchPlan, encoded) or give
    give ok(decoded.address)
}
choose round_trip with {} {
    case Ok carries value => value
    case Err carries message => message
}
"#;
    let expected = Value::String("example.com".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
}

#[test]
fn plan_decoding_rejects_mismatched_shapes() {
    let source = r#"
shape FetchPlan holds {
    address is String
} derives Json
shape OtherPlan holds {
    address is String
    extra is Int
} derives Json
define attempt takes { } gives String or String {
    prepare request as FetchPlan with { address set "example.com" }
    keep encoded set std.plans.encode(request) or give
    keep decoded set std.plans.decode(OtherPlan, encoded) or give
    give ok("decoded")
}
choose attempt with {} {
    case Ok carries value => value
    case Err carries message => message
}
"#;
    let tree = eval_tree(source);
    let Value::String(message) = tree else {
        panic!("expected a message");
    };
    assert!(message.contains("shape"));
}

#[test]
fn the_gpu_stub_reports_visible_unavailability() {
    let source = r#"
when std.gpu.available() {
    "available"
} otherwise {
    choose std.gpu.open("cpu") {
        case Ok carries device => "opened"
        case Err carries message => message
    }
}
"#;
    let tree = eval_tree(source);
    let Value::String(message) = tree else {
        panic!("expected a message");
    };
    assert!(message.contains("no GPU adapter"));
}

#[test]
fn modules_need_trusted_to_cross_the_systems_boundary() {
    let span = nivren::ast::Span { line: 1, column: 1 };
    let module = |source: &str| nivren::ast::Stmt::Module {
        name: "bridge".into(),
        body: nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap(),
        exports: vec![],
        span,
    };
    let errors =
        nivren::typecheck::check(&[module(r#"std.host.invoke("nivren.echo", "{}")"#)]).unwrap_err();
    assert!(errors[0].to_string().contains("trusted"));
    assert!(
        nivren::typecheck::check(&[module(
            r#"
trusted "wraps the demo host bridge"
std.host.invoke("nivren.echo", "{}")
"#
        )])
        .is_ok()
    );
    let errors = nivren::typecheck::check(&[module(
        r#"
trusted "   "
std.host.invoke("nivren.echo", "{}")
"#,
    )])
    .unwrap_err();
    assert!(errors[0].to_string().contains("states its reason"));
    assert!(nivren::check(r#"std.host.invoke("nivren.echo", "{}")"#).is_ok());
}

#[test]
fn reflection_schema_describes_functions_in_both_engines() {
    let source = r#"
define add takes { left is Int, right is Int } gives Int {
    give left + right
}
choose std.reflect.schema(add) {
    case Ok carries schema => choose std.text.join([
        std.map.get(schema, "$kind") ?? "?",
        std.map.get(schema, "$name") ?? "?",
        std.map.get(schema, "left") ?? "?",
        std.map.get(schema, "right") ?? "?"
    ], " ") {
        case Ok carries joined => joined
        case Err carries message => message
    }
    case Err carries message => message
}
"#;
    let expected = Value::String("function add 0 1".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
}

#[test]
fn uint_arithmetic_is_checked_wrapping_is_explicit_and_dual_engine() {
    let source = r#"
define compute takes { } gives String or String {
    keep small set std.uint.from_int(7) or give
    keep large set std.uint.from_int(6) or give
    keep sum set small + large
    keep product set small * large
    keep wrapped set std.uint.wrapping_sub(std.uint.min(), small)
    keep back set std.uint.to_int(sum) or give
    give ok(text "{std.uint.format(sum)} {std.uint.format(product)} {std.uint.format(wrapped)} {back} {small < large}")
}
choose compute with {} {
    case Ok carries value => value
    case Err carries message => message
}
"#;
    let expected = Value::String("13 42 18446744073709551609 13 no".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
}

#[test]
fn uint_overflow_and_negation_are_typed_runtime_errors() {
    let overflow = r#"
define boom takes { } gives UInt or String {
    keep one set std.uint.from_int(1) or give
    give ok(std.uint.max() + one)
}
boom with {}
"#;
    let program = nivren::parser::parse(nivren::lexer::scan(overflow).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let error = nivren::runtime::Interpreter::new()
        .run_bytecode(&chunk)
        .unwrap_err();
    assert!(error.to_string().contains("unsigned integer overflow"));
    let negation = r#"
define flip takes { } gives UInt or String {
    keep one set std.uint.from_int(1) or give
    give ok(-one)
}
flip with {}
"#;
    let errors = nivren::check(negation).unwrap_err();
    assert!(errors[0].to_string().contains("UInt"));
    let mixed = nivren::check("std.uint.max() + 1").unwrap_err();
    assert!(mixed[0].to_string().contains("same type"));
}

#[test]
fn generators_expand_into_checked_declarations() {
    let source = r#"
generate schemas {
    keep fields set std.map.set(std.map.of("x", "Int"), "y", "Int")
    keep point set choose std.source.shape("Point", fields, ["Compare"]) {
        case Ok carries declaration => declaration
        otherwise as problem => problem
    }
    give [point]
}
expand schemas
keep origin set Point with { x set 1, y set 2 }
show(origin.x + origin.y)
"#;
    assert_eq!(eval(source), Value::Null);
    let misuse = source.replace("x set 1", "x set \"one\"");
    assert!(nivren::check(&misuse).is_err());
}

#[test]
fn generators_stay_pure_bounded_and_validated() {
    let errors = nivren::check("expand missing").unwrap_err();
    assert!(errors[0].to_string().contains("unknown generator"));
    let effectful = r#"
generate impure {
    std.time.sleep(0.1)
    give []
}
expand impure
"#;
    let errors = nivren::check(effectful).unwrap_err();
    assert!(errors[0].to_string().contains("does not allow Time"));
    let wrong_shape = r#"
generate wrong {
    give [42]
}
expand wrong
"#;
    let errors = nivren::check(wrong_shape).unwrap_err();
    assert!(errors[0].to_string().contains("source.Declaration"));
    let bad_label = r#"
generate sized takes { limit is Int } {
    give []
}
expand sized with { size set 3 }
"#;
    let errors = nivren::check(bad_label).unwrap_err();
    assert!(errors[0].to_string().contains("canonical order"));
}

#[test]
fn generated_choices_join_pattern_matching() {
    let source = r#"
generate signals {
    keep cases set std.map.set(std.map.of("Go", ""), "Wait", "Int")
    keep signal set choose std.source.choice("Signal", cases) {
        case Ok carries declaration => declaration
        otherwise as problem => problem
    }
    give [signal]
}
expand signals
show(choose Signal.Wait(7) {
    case Wait carries delay => delay
    case Go => 0
})
"#;
    assert_eq!(eval(source), Value::Null);
}

#[test]
fn i128_joins_the_fixed_width_family_in_both_engines() {
    let source = r#"
define compute takes { } gives String or String {
    keep big set std.i128.parse("170141183460469231731687303715884105727") or give
    keep one set std.i128.from_int(1) or give
    keep smaller set big - one
    give ok(std.i128.format(smaller))
}
choose compute with {} {
    case Ok carries value => value
    case Err carries message => message
}
"#;
    let expected = Value::String("170141183460469231731687303715884105726".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
}

#[test]
fn text_holes_render_display_shapes_and_datetimes_in_both_engines() {
    let source = r#"
shape Point holds {
    x is Int
    y is Int
} derives Display
keep origin set Point with { x set 1, y set 2 }
text "at {origin}"
"#;
    let tree = eval_tree(source);
    let Value::String(rendered) = &tree else {
        panic!("expected text");
    };
    assert!(rendered.starts_with("at "));
    assert!(rendered.contains('1') && rendered.contains('2'));
    assert_eq!(eval_vm(source), tree);
    let undisplayable = r#"
shape Quiet holds {
    x is Int
}
keep value set Quiet with { x set 1 }
text "at {value}"
"#;
    let errors = nivren::check(undisplayable).unwrap_err();
    assert!(errors[0].to_string().contains("Display"));
}

#[test]
fn promises_are_enforced_again_at_runtime_in_both_engines() {
    let source = r#"
promise never Time
std.time.sleep(0.0)
"#;
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let error = nivren::runtime::Interpreter::new()
        .run_bytecode(&chunk)
        .unwrap_err();
    assert!(error.to_string().contains("promise never Time"));
    let error = nivren::runtime::Interpreter::new()
        .run(&program)
        .unwrap_err();
    assert!(error.to_string().contains("promise never Time"));
    let scoped = r#"
define quiet takes { } gives Int {
    promise never Time
    give 1
}
keep first set quiet with {}
std.time.sleep(0.0)
first
"#;
    let program = nivren::parser::parse(nivren::lexer::scan(scoped).unwrap()).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    assert_eq!(
        nivren::runtime::Interpreter::new()
            .run_bytecode(&chunk)
            .unwrap(),
        Value::Int(1)
    );
}

#[test]
fn declared_payload_limits_bound_text_literals_in_both_engines() {
    let source = r#"
keep chunk set "0123456789abcdef"
text "{chunk}{chunk}{chunk}"
"#;
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let error = nivren::runtime::Interpreter::new()
        .with_payload_limit(32)
        .run_bytecode(&chunk)
        .unwrap_err();
    assert!(error.to_string().contains("payload limit"));
    let error = nivren::runtime::Interpreter::new()
        .with_payload_limit(32)
        .run(&program)
        .unwrap_err();
    assert!(error.to_string().contains("payload limit"));
    assert_eq!(
        nivren::runtime::Interpreter::new()
            .run_bytecode(&chunk)
            .unwrap(),
        Value::String("0123456789abcdef".repeat(3))
    );
}

#[test]
fn authority_diffs_show_added_and_removed_grants() {
    let old = "package = \"app\"\n[dependency.web]\nNetwork = \"host:api.example.com\"\n";
    let new = "package = \"app\"\n[dependency.web]\nNetwork = \"host:api.example.com,host:evil.example.com\"\n";
    let diff = nivren::package::authority_diff(old, new);
    assert!(diff.contains("- Network = \"host:api.example.com\""));
    assert!(diff.contains("+ Network = \"host:api.example.com,host:evil.example.com\""));
    assert!(
        nivren::package::authority_diff(old, old).contains("formatting-only")
            || nivren::package::authority_diff(old, old).is_empty()
    );
}

#[test]
fn published_diagnostics_carry_stable_catalog_codes() {
    let cases = [
        ("stop", "NIV5001"),
        ("choose 5 { case 1 => \"one\" }", "NIV5003"),
        (
            "choose 5 {\n    otherwise => 1\n    case 2 => 2\n}",
            "NIV5004",
        ),
        ("promise never Magic", "NIV5013"),
        ("promise never Time\nstd.time.sleep(0.1)", "NIV5011"),
        ("expand missing", "NIV5016"),
        (
            "shape Point holds {\n    x is Int\n    y is Int\n}\nkeep Point holds { x set 0 } set Point with { x set 1, y set 2 }",
            "NIV5017",
        ),
    ];
    for (source, expected) in cases {
        let errors = nivren::check(source).unwrap_err();
        assert_eq!(
            errors[0].code(),
            Some(expected),
            "for source: {source} ({})",
            errors[0].message
        );
    }
}

#[test]
fn generated_bindings_declare_literal_constants() {
    let source = r#"
generate constants {
    keep limit set choose std.source.binding("answer", 42) {
        case Ok carries declaration => declaration
        otherwise as problem => problem
    }
    give [limit]
}
expand constants
answer
"#;
    assert_eq!(nivren::run(source).unwrap(), Value::Int(42));
}

#[test]
fn user_iterate_adopters_drive_each_loops_in_both_engines() {
    let source = r#"
shape Counter holds {
    current is Int
    limit is Int
}
shape CounterStep holds {
    item is Int
    next is Counter
}
define counter_advance takes { state is Counter } gives CounterStep? {
    when state.current > state.limit {
        give none
    }
    give CounterStep with {
        item set state.current
        next set Counter with { current set state.current + 1, limit set state.limit }
    }
}
protocol Iterate { define advance takes { state is Self } gives CounterStep? }
adopt Iterate for Counter { advance set counter_advance }
change total set 0
each value in Counter with { current set 1, limit set 4 } {
    change total to total + value
}
total
"#;
    assert_eq!(eval_tree(source), Value::Int(10));
    assert_eq!(eval_vm(source), Value::Int(10));
}

#[test]
fn text_holes_may_contain_string_literals_in_both_engines() {
    let source = r#"
keep table set std.map.of("kind", "demo")
text "value {std.map.get(table, "kind") ?? "?"} end"
"#;
    let expected = Value::String("value demo end".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
}

#[test]
fn u128_completes_the_unsigned_family_in_both_engines() {
    let source = r#"
define compute takes { } gives String or String {
    keep huge set std.u128.parse("340282366920938463463374607431768211455") or give
    keep one set std.u128.from_int(1) or give
    keep smaller set huge - one
    give ok(std.u128.format(smaller))
}
choose compute with {} {
    case Ok carries value => value
    case Err carries message => message
}
"#;
    let expected = Value::String("340282366920938463463374607431768211454".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
}

#[test]
fn edition_five_projects_opt_into_strict_gates() {
    let manifest = nivren::project::Manifest::parse(
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nentry = \"src/main.niv\"\nedition = \"5\"\n",
        std::path::PathBuf::from("."),
    )
    .unwrap();
    assert_eq!(manifest.edition, 5);
    assert!(manifest.source().contains("edition = \"5\""));
    let default = nivren::project::Manifest::parse(
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nentry = \"src/main.niv\"\n",
        std::path::PathBuf::from("."),
    )
    .unwrap();
    assert_eq!(default.edition, 4);
    let error = nivren::project::Manifest::parse(
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nentry = \"src/main.niv\"\nedition = \"3\"\n",
        std::path::PathBuf::from("."),
    )
    .unwrap_err();
    assert!(error.to_string().contains("unknown edition"));
    let program = nivren::parser::parse(
        nivren::lexer::scan(r#"std.host.invoke("nivren.echo", "{}")"#).unwrap(),
    )
    .unwrap();
    assert!(nivren::typecheck::check_with_edition(&program, 4).is_ok());
    let errors = nivren::typecheck::check_with_edition(&program, 5).unwrap_err();
    assert!(errors[0].to_string().contains("trusted"));
}

#[test]
fn edition_five_lexis_upgrades_work_in_both_engines() {
    let source = r#"
keep million set 1_000_000
keep mask set 0xFF
keep bits set 0b1010
keep exponent set 1.5e3
keep unicode set "A\u{2192}B"
keep pem set raw "line\none"
text "{million} {mask} {bits} {exponent} {unicode} {pem}"
"#;
    let expected = Value::String("1000000 255 10 1500 A→B line\\none".into());
    assert_eq!(eval_tree(source), expected);
    assert_eq!(eval_vm(source), expected);
    let errors = nivren::lexer::scan(r#""bad \q escape""#).unwrap_err();
    assert!(errors[0].to_string().contains("unknown escape"));
    assert!(nivren::lexer::scan("1__0").is_err());
}

#[test]
fn use_as_names_imported_modules_explicitly() {
    let directory = module_fixture("use-as");
    fs::write(
        directory.join("helper.niv"),
        "define double takes { value is Int } gives Int { give value * 2 }\nexpose { double }",
    )
    .unwrap();
    fs::write(
        directory.join("main.niv"),
        "use \"helper.niv\" as tools\nshow(tools.double with { value set 21 })",
    )
    .unwrap();
    let program = nivren::modules::load(&directory.join("main.niv")).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    nivren::runtime::Interpreter::new()
        .run_bytecode(&chunk)
        .unwrap();
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn declared_gives_may_not_fall_off_the_end() {
    let errors = nivren::check(
        r#"
define lucky takes { value is Int } gives Int {
    when value > 0 {
        give value
    }
}
lucky with { value set 1 }
"#,
    )
    .unwrap_err();
    assert!(errors[0].to_string().contains("without 'give'"));
    assert!(
        nivren::check(
            r#"
define lucky takes { value is Int } gives Int {
    when value > 0 {
        give value
    } otherwise {
        give 0
    }
}
lucky with { value set 1 }
"#,
        )
        .is_ok()
    );
}

#[test]
fn expose_marks_declarations_in_place() {
    let directory = module_fixture("expose-modifier");
    fs::write(
        directory.join("helper.niv"),
        "expose define triple takes { value is Int } gives Int { give value * 3 }\ndefine hidden takes { } gives Int { give 0 }",
    )
    .unwrap();
    fs::write(
        directory.join("main.niv"),
        "use \"helper.niv\" as tools\nshow(tools.triple with { value set 14 })",
    )
    .unwrap();
    let program = nivren::modules::load(&directory.join("main.niv")).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let hidden = fs::read_to_string(directory.join("main.niv"))
        .unwrap()
        .replace("triple with { value set 14 }", "hidden with { }");
    fs::write(directory.join("main.niv"), hidden).unwrap();
    let program = nivren::modules::load(&directory.join("main.niv")).unwrap();
    assert!(nivren::typecheck::check(&program).is_err());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn the_verifier_rejects_loop_exit_bytecode_outside_a_loop_body() {
    let program = nivren::parser::parse(nivren::lexer::scan("stop").unwrap()).unwrap();
    let error = nivren::bytecode::compile(&program).unwrap_err();
    assert!(error[0].to_string().contains("outside a loop body"));
}
