use std::hint::black_box;
use std::time::{Duration, Instant};

use nivren::runtime::{Interpreter, Value};

fn main() {
    let source = "define kernel(a: Int, b: Int) gives Int { give (a + b) * 2; } change index = 0; change result = 0; repeat (index < 200000) { result = kernel(index, 3); index = index + 1; } result";
    let tokens = nivren::lexer::scan(source).unwrap();
    let program = nivren::parser::parse(tokens).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();

    let vm = median(7, || {
        let mut interpreter = Interpreter::new();
        interpreter.set_jit_threshold(u32::MAX);
        let value = black_box(interpreter.run_bytecode(&chunk).unwrap());
        assert_eq!(value, Value::Int(400004));
        value
    });
    let native = median(7, || {
        let mut interpreter = Interpreter::new();
        interpreter.set_jit_threshold(16);
        let value = black_box(interpreter.run_bytecode(&chunk).unwrap());
        assert_eq!(value, Value::Int(400004));
        assert!(interpreter.jit_stats().executions > 100_000);
        value
    });
    let speedup = vm.as_secs_f64() / native.as_secs_f64();
    let complete_native = median(7, || {
        let mut interpreter = Interpreter::new();
        interpreter.set_jit_threshold(16);
        let value = black_box(interpreter.run_native(&chunk).unwrap());
        assert_eq!(value, Value::Int(400004));
        assert!(interpreter.jit_stats().executions > 100_000);
        assert_eq!(interpreter.native_stats().fallbacks, 0);
        value
    });
    let complete_native_ratio = complete_native.as_secs_f64() / native.as_secs_f64();
    println!("nivren_benchmark_vm_ms {:.3}", vm.as_secs_f64() * 1000.0);
    println!(
        "nivren_benchmark_tiered_ms {:.3}",
        native.as_secs_f64() * 1000.0
    );
    println!("nivren_benchmark_jit_speedup {speedup:.3}");
    println!(
        "nivren_benchmark_complete_native_ms {:.3}",
        complete_native.as_secs_f64() * 1000.0
    );
    println!("nivren_benchmark_complete_native_ratio {complete_native_ratio:.3}");

    let recursive_source = "define fibonacci(value: Int) gives Int { when value < 2 { give value; } give fibonacci(value - 1) + fibonacci(value - 2); } fibonacci(20)";
    let recursive_tokens = nivren::lexer::scan(recursive_source).unwrap();
    let recursive_program = nivren::parser::parse(recursive_tokens).unwrap();
    nivren::typecheck::check(&recursive_program).unwrap();
    let recursive_chunk = nivren::bytecode::compile(&recursive_program).unwrap();
    let recursive_tree = median(7, || {
        let value = black_box(Interpreter::new().run(&recursive_program).unwrap());
        assert_eq!(value, Value::Int(6765));
        value
    });
    let recursive_vm = median(7, || {
        let mut interpreter = Interpreter::new();
        interpreter.set_jit_threshold(u32::MAX);
        let value = black_box(interpreter.run_bytecode(&recursive_chunk).unwrap());
        assert_eq!(value, Value::Int(6765));
        value
    });
    let recursive_speedup = recursive_tree.as_secs_f64() / recursive_vm.as_secs_f64();
    println!(
        "nivren_benchmark_recursive_tree_ms {:.3}",
        recursive_tree.as_secs_f64() * 1000.0
    );
    println!(
        "nivren_benchmark_recursive_vm_ms {:.3}",
        recursive_vm.as_secs_f64() * 1000.0
    );
    println!("nivren_benchmark_recursive_speedup {recursive_speedup:.3}");

    let record_source = "shape Sample { alpha: Int, beta: Int, gamma: Int, delta: Int, epsilon: Int, zeta: Int, eta: Int, theta: Int } define checksum(sample: Sample) gives Int { give sample.theta + sample.alpha; } change index = 0; change result = 0; repeat (index < 100000) { result = checksum(Sample(index, 2, 3, 4, 5, 6, 7, 8)); index = index + 1; } result";
    let record_tokens = nivren::lexer::scan(record_source).unwrap();
    let record_program = nivren::parser::parse(record_tokens).unwrap();
    nivren::typecheck::check(&record_program).unwrap();
    let record_chunk = nivren::bytecode::compile(&record_program).unwrap();
    let record_vm = median(7, || {
        let mut interpreter = Interpreter::new();
        interpreter.set_jit_threshold(u32::MAX);
        let value = black_box(interpreter.run_bytecode(&record_chunk).unwrap());
        assert_eq!(value, Value::Int(100007));
        value
    });
    println!(
        "nivren_benchmark_record_fields_vm_ms {:.3}",
        record_vm.as_secs_f64() * 1000.0
    );

    if std::env::var_os("NIVREN_BENCH_GATE").is_some() {
        if speedup < 1.05 {
            eprintln!("performance gate failed: native tier speedup {speedup:.3} is below 1.05");
            std::process::exit(1);
        }
        if recursive_speedup < 1.10 {
            eprintln!(
                "performance gate failed: recursive bytecode speedup {recursive_speedup:.3} is below 1.10"
            );
            std::process::exit(1);
        }
        if complete_native_ratio > 1.10 {
            eprintln!(
                "performance gate failed: complete native ratio {complete_native_ratio:.3} exceeds 1.10"
            );
            std::process::exit(1);
        }
    }
}

fn median<T>(runs: usize, mut operation: impl FnMut() -> T) -> Duration {
    let _ = operation();
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let started = Instant::now();
        black_box(operation());
        samples.push(started.elapsed());
    }
    samples.sort();
    samples[runs / 2]
}
