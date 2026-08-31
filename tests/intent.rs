use nivren::bytecode::{Chunk, Op};
use nivren::intent::{Optimization, analyze};
use nivren::runtime::{Interpreter, Value};

fn checked(source: &str) -> Vec<nivren::ast::Stmt> {
    let tokens = nivren::lexer::scan(source).unwrap();
    let program = nivren::parser::parse(tokens).unwrap();
    nivren::typecheck::check(&program).unwrap();
    program
}

fn compiled(source: &str) -> Chunk {
    nivren::bytecode::compile(&checked(source)).unwrap()
}

#[test]
fn niv_explain_matches_the_reviewed_snapshot() {
    let source = include_str!("../proofs/edition4/intent_snapshot.niv");
    let explained = nivren::compiler::Compiler::new()
        .explain(source, true)
        .unwrap();
    assert_eq!(explained, include_str!("snapshots/intent_explain.json"));
}

fn operations(chunk: &Chunk) -> Vec<Op> {
    let mut output = Vec::new();
    for instruction in &chunk.code {
        let mut operation = instruction.op.clone();
        if let Op::MakeFunction { body, .. } = &mut operation {
            let nested = operations(body);
            body.code.clear();
            output.push(operation);
            output.extend(nested);
        } else {
            output.push(operation);
        }
    }
    output
}

#[test]
fn pure_through_and_direct_calls_lower_to_equivalent_operations() {
    let prefix = "define double takes { value is Int } gives Int { give value * 2 }\n";
    let direct = operations(&compiled(&format!(
        "{prefix}double with {{ value set 21 }}"
    )));
    let intent = operations(&compiled(&format!("{prefix}21 through double")));
    assert_eq!(intent, direct);
    let graph = analyze(
        &checked(&format!("{prefix}21 through double")),
        Optimization::Enabled,
    );
    assert_eq!(graph.summary.pure_runtime_plan_allocations, 0);
    assert_eq!(graph.summary.fused_pipelines, 1);
}

#[test]
fn runtime_metrics_observe_plan_allocation_and_perform_boundary() {
    let chunk = compiled(
        "shape Request holds { path is String }\nprepare request as Request with { path set \"/health\" }\n(perform request).path",
    );
    let mut interpreter = Interpreter::new();
    interpreter.enable_metrics();
    assert_eq!(
        interpreter.run_bytecode(&chunk).unwrap(),
        Value::String("/health".into())
    );
    let metrics = interpreter.execution_metrics().unwrap();
    assert_eq!(metrics.plan_allocations, 1);
    assert_eq!(metrics.perform_boundaries, 1);
}

#[test]
fn runtime_effect_sequence_matches_source_and_excludes_unauthorized_calls() {
    let source = "define inspect takes {} gives String needs FileRead, Environment { keep exists set perform std.files.exists with { path set \"/definitely-not-a-nivren-file\" }\nkeep home set perform std.env.get with { name set \"NIVREN_INTENT_MISSING\" }\ngive \"done\" }\ninspect with {}";
    let chunk = compiled(source);
    let mut interpreter = Interpreter::new();
    interpreter.enable_metrics();
    assert_eq!(
        interpreter.run_bytecode(&chunk).unwrap(),
        Value::String("done".into())
    );
    let metrics = interpreter.execution_metrics().unwrap();
    assert_eq!(
        metrics.effect_sequence,
        vec!["FileRead:exists", "Environment:get"]
    );
    assert_eq!(metrics.perform_boundaries, 2);

    let mut denied = Interpreter::new().with_capabilities(Vec::<String>::new());
    denied.enable_metrics();
    assert!(denied.run_bytecode(&chunk).is_err());
    assert!(
        denied
            .execution_metrics()
            .unwrap()
            .effect_sequence
            .is_empty()
    );
}

#[test]
fn explain_reports_file_http_database_and_concurrency_resources() {
    let sources = [
        (
            "define file takes {} gives Bool or Problem needs FileRead { give perform std.files.exists with { path set \"/tmp/nivren-intent\" } }",
            "filesystem",
        ),
        (
            "define web takes {} gives String or Problem needs Network { give perform std.web.get with { url set \"http://127.0.0.1/\" timeout set 1.0 } }",
            "network-socket",
        ),
        (
            "define database takes {} gives Transaction<String, Int> { give std.transactions.create with { map set std.map.of with { key set \"id\" value set 1 } } }",
            "database-transaction",
        ),
        (
            "define concurrent takes {} gives Channel needs Channel { give perform std.channels.create with { capacity set 1 } }",
            "bounded-channel",
        ),
    ];
    for (source, resource) in sources {
        let graph = analyze(&checked(source), Optimization::Enabled);
        graph.validate().unwrap();
        assert!(
            graph
                .summary
                .resources
                .iter()
                .any(|found| found == resource),
            "missing {resource} in {}",
            graph.to_json()
        );
    }
}

#[test]
fn through_batches_and_parallel_task_plans_remain_bounded() {
    let batching = "define batches takes {} gives [[Int]] or Problem { give [1, 2, 3, 4, 5] through std.list.batch with { count set 2 } }\nbatches with {}";
    let chunk = compiled(batching);
    assert_eq!(
        Interpreter::new().run_bytecode(&chunk).unwrap().to_string(),
        "Ok([[1, 2], [3, 4], [5]])"
    );
    let graph = analyze(&checked(batching), Optimization::Enabled);
    assert_eq!(graph.summary.pure_runtime_plan_allocations, 0);
    assert_eq!(graph.summary.fused_pipelines, 1);
    graph.validate().unwrap();

    let parallel = "define one takes {} gives Int { give 1 }\ndefine many takes {} gives [Int] or Problem needs Task { keep first set perform std.tasks.spawn with { operation set one }\nkeep second set perform std.tasks.spawn with { operation set one }\ngive perform std.tasks.all with { tasks set [first, second] } }";
    let graph = analyze(&checked(parallel), Optimization::Enabled);
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.operation == "std.tasks.all"
                && node.buffering == "bounded"
                && node.cancellation == "propagated")
    );
    graph.validate().unwrap();
}

#[test]
fn bounded_channels_apply_backpressure_and_cancellation_cleans_up() {
    let backpressure = "define bounded takes {} gives Bool needs Channel { keep channel set perform std.channels.create with { capacity set 1 }\nkeep first set perform std.channels.send with { channel set channel value set 1 timeout set 0.01 }\nkeep second set perform std.channels.send with { channel set channel value set 2 timeout set 0.01 }\ngive choose second { case Ok carries value => no case Err carries problem => yes } }\nperform bounded with {}";
    assert_eq!(
        Interpreter::new()
            .run_bytecode(&compiled(backpressure))
            .unwrap(),
        Value::Bool(true)
    );

    let cancellation = "define forever takes {} gives Int { change value set 0\nrepeat while value < 9223372036854775807 { change value to value + 1 }\ngive value }\ndefine stop takes {} gives Bool needs Task { keep task set perform std.tasks.spawn with { operation set forever }\nperform std.tasks.cancel with { task set task }\nkeep result set perform std.tasks.await with { task set task }\ngive choose result { case Ok carries value => no case Err carries problem => yes } }\nperform stop with {}";
    assert_eq!(
        Interpreter::new()
            .run_bytecode(&compiled(cancellation))
            .unwrap(),
        Value::Bool(true)
    );
}
