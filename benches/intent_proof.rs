use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use nivren::runtime::Interpreter;

static FILE_ID: AtomicUsize = AtomicUsize::new(0);

fn compile(source: &str) -> nivren::bytecode::Chunk {
    let program = nivren::parser::parse(nivren::lexer::scan(source).unwrap()).unwrap();
    nivren::typecheck::check(&program).unwrap();
    nivren::bytecode::compile(&program).unwrap()
}

fn median(mut run: impl FnMut()) -> Duration {
    run();
    let mut measurements = Vec::with_capacity(7);
    for _ in 0..7 {
        let started = Instant::now();
        run();
        measurements.push(started.elapsed());
    }
    measurements.sort();
    measurements[measurements.len() / 2]
}

fn compare(name: &str, direct: &str, intent: &str) -> f64 {
    let direct = compile(direct);
    let intent = compile(intent);
    let direct_time = median(|| {
        Interpreter::new().run_bytecode(&direct).unwrap();
    });
    let intent_time = median(|| {
        Interpreter::new().run_bytecode(&intent).unwrap();
    });
    let ratio = intent_time.as_secs_f64() / direct_time.as_secs_f64();
    let allocation_work = |chunk: &nivren::bytecode::Chunk| {
        let mut interpreter = Interpreter::new();
        interpreter.enable_metrics();
        interpreter.run_bytecode(chunk).unwrap();
        interpreter
            .execution_metrics()
            .unwrap()
            .allocation_work_bytes
    };
    let direct_memory = allocation_work(&direct);
    let intent_memory = allocation_work(&intent);
    let memory_ratio = if direct_memory == 0 {
        1.0
    } else {
        intent_memory as f64 / direct_memory as f64
    };
    println!(
        "nivren_intent_{name}_direct_ms {:.3}",
        direct_time.as_secs_f64() * 1000.0
    );
    println!(
        "nivren_intent_{name}_optimized_ms {:.3}",
        intent_time.as_secs_f64() * 1000.0
    );
    println!("nivren_intent_{name}_ratio {ratio:.3}");
    println!("nivren_intent_{name}_memory_ratio {memory_ratio:.3}");
    if std::env::var_os("NIVREN_INTENT_BENCH_GATE").is_some() && memory_ratio > 1.10 {
        eprintln!("intent memory gate failed: {name} ratio {memory_ratio:.3} exceeds 1.10");
        std::process::exit(1);
    }
    ratio
}

fn local_http_server(requests: usize) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        for stream in listener.incoming().take(requests) {
            let mut stream = stream.unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .unwrap();
        }
    });
    (format!("http://{address}/health"), handle)
}

fn main() {
    let identifier = FILE_ID.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!(
        "nivren-intent-proof-{}-{identifier}.txt",
        std::process::id()
    ));
    fs::write(&file, "bounded intent proof payload").unwrap();
    let path = file.to_string_lossy().replace('\\', "\\\\");
    let file_prefix = "define work takes { path is String } gives Int or String needs FileRead { change count set 0\nrepeat while count < 64 {";
    let file_suffix =
        " or give\nchange count to count + 1 }\ngive ok(count) }\nwork with { path set \"";
    let direct_file = format!(
        "{file_prefix} keep contents set std.fs.read with {{ path set path }}{file_suffix}{path}\" }}"
    );
    let intent_file = format!(
        "{file_prefix} keep contents set perform std.fs.read with {{ path set path }}{file_suffix}{path}\" }}"
    );
    let file_ratio = compare("files", &direct_file, &intent_file);
    fs::remove_file(&file).unwrap();

    let http_runs = 8 * 7;
    let (direct_url, direct_server) = local_http_server(http_runs);
    let direct_http = format!(
        "define work takes {{}} gives Int or String needs Network {{ change count set 0\nrepeat while count < 7 {{ keep body set std.http.get with {{ url set \"{direct_url}\" timeout set 2.0 }} or give\nchange count to count + 1 }}\ngive ok(count) }}\nwork with {{}}"
    );
    let (intent_url, intent_server) = local_http_server(http_runs);
    let intent_http = format!(
        "define work takes {{}} gives Int or String needs Network {{ change count set 0\nrepeat while count < 7 {{ keep body set perform std.http.get with {{ url set \"{intent_url}\" timeout set 2.0 }} or give\nchange count to count + 1 }}\ngive ok(count) }}\nperform work with {{}}"
    );
    let http_ratio = compare("http", &direct_http, &intent_http);
    direct_server.join().unwrap();
    intent_server.join().unwrap();

    let database_body = "keep original set std.map.single with { key set \"count\" value set 1 }\nchange count set 0\nrepeat while count < 512 { keep transaction set std.transactions.begin with { map set original }\nkeep changed set std.transactions.set with { transaction set transaction key set \"count\" value set count } or give\nkeep committed set std.transactions.commit with { transaction set transaction } or give\nchange count to count + 1 }\ngive ok(count)";
    let direct_database =
        format!("define work takes {{}} gives Int or String {{ {database_body} }}\nwork with {{}}");
    let intent_database = format!(
        "shape Query holds {{ name is String }}\nprepare query as Query with {{ name set \"counter\" }}\ndefine work takes {{ query is Query }} gives Int or String {{ {database_body} }}\nwork with {{ query set perform query }}"
    );
    let database_ratio = compare("database", &direct_database, &intent_database);

    let direct_concurrency = "define work takes {} gives Int or String needs Channel { keep channel set std.channels.create with { capacity set 1 }\nchange count set 0\nrepeat while count < 256 { keep sent set std.channels.send with { channel set channel value set count timeout set 1.0 } or give\nkeep received set std.channels.receive with { channel set channel timeout set 1.0 } or give\nchange count to count + 1 }\ngive ok(count) }\nwork with {}";
    let intent_concurrency = "define work takes {} gives Int or String needs Channel { keep channel set perform std.channels.create with { capacity set 1 }\nchange count set 0\nrepeat while count < 256 { keep sent set perform std.channels.send with { channel set channel value set count timeout set 1.0 } or give\nkeep received set perform std.channels.receive with { channel set channel timeout set 1.0 } or give\nchange count to count + 1 }\ngive ok(count) }\nperform work with {}";
    let concurrency_ratio = compare("concurrency", direct_concurrency, intent_concurrency);

    if std::env::var_os("NIVREN_INTENT_BENCH_GATE").is_some() {
        for (name, ratio) in [
            ("files", file_ratio),
            ("http", http_ratio),
            ("database", database_ratio),
            ("concurrency", concurrency_ratio),
        ] {
            if ratio > 1.10 {
                eprintln!("intent performance gate failed: {name} ratio {ratio:.3} exceeds 1.10");
                std::process::exit(1);
            }
        }
    }
}
