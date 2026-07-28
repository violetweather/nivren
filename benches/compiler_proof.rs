use std::hint::black_box;
use std::time::{Duration, Instant};

use nivren_jit::{CompiledFunction, IntOp};

#[inline(never)]
fn safe_rust_kernel(left: i64, right: i64) -> i64 {
    left.checked_add(right).unwrap().checked_mul(2).unwrap()
}

fn median(mut operation: impl FnMut()) -> Duration {
    operation();
    let mut samples = Vec::with_capacity(9);
    for _ in 0..9 {
        let started = Instant::now();
        operation();
        samples.push(started.elapsed());
    }
    samples.sort();
    samples[samples.len() / 2]
}

fn main() {
    let operations = [
        IntOp::Load(0),
        IntOp::Load(1),
        IntOp::Add,
        IntOp::Constant(2),
        IntOp::Multiply,
        IntOp::Return,
    ];
    let native = CompiledFunction::compile(2, 2, &operations).unwrap();
    let iterations = 2_000_000i64;
    let rust = median(|| {
        let mut result = 0;
        for value in 0..iterations {
            result = black_box(safe_rust_kernel(black_box(value), 3));
        }
        assert_eq!(result, 4_000_004);
    });
    let nivren = median(|| {
        let mut result = 0;
        for value in 0..iterations {
            result = black_box(native.call(&[black_box(value), 3]).unwrap());
        }
        assert_eq!(result, 4_000_004);
    });
    let ratio = nivren.as_secs_f64() / rust.as_secs_f64();
    println!(
        "nivren_compiler_kernel_rust_ms {:.3}",
        rust.as_secs_f64() * 1000.0
    );
    println!(
        "nivren_compiler_kernel_native_ms {:.3}",
        nivren.as_secs_f64() * 1000.0
    );
    println!("nivren_compiler_kernel_rust_ratio {ratio:.3}");
    if std::env::var_os("NIVREN_COMPILER_BENCH_GATE").is_some() && ratio > 2.0 {
        eprintln!("compiler performance gate failed: native kernel ratio {ratio:.3} exceeds 2.0");
        std::process::exit(1);
    }
}
