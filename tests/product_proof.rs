use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use nivren::runtime::{Interpreter, Value};

fn database_driver_chunk() -> nivren::bytecode::Chunk {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proofs/edition4/database_driver.niv");
    let program = nivren::modules::load(&path).unwrap();
    nivren::typecheck::check(&program).unwrap();
    nivren::bytecode::compile(&program).unwrap()
}

fn database_host() -> (Interpreter, Arc<Mutex<Vec<String>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&events);
    let interpreter = Interpreter::new()
        .with_capabilities(["Native".to_string()])
        .with_host_callback(move |operation, request| {
            captured.lock().unwrap().push(operation.to_string());
            match operation {
                "nivren.handle.open:database" if request == "memory://product-proof" => {
                    Ok("database-proof-handle".into())
                }
                "nivren.handle.call:query" => {
                    assert!(request.contains("SELECT name FROM users"));
                    Ok("{\"rows\":[\"Ada\",\"Lin\"],\"next_cursor\":null}".into())
                }
                "nivren.handle.close" => Ok("closed".into()),
                _ => Err(format!("unexpected database host operation {operation}")),
            }
        });
    (interpreter, events)
}

#[test]
fn database_adapter_is_typed_scoped_and_equivalent_in_vm_and_native_control() {
    let chunk = database_driver_chunk();
    for native in [false, true] {
        let (mut interpreter, events) = database_host();
        let result = if native {
            interpreter.run_native(&chunk)
        } else {
            interpreter.run_bytecode(&chunk)
        };
        assert_eq!(result.unwrap(), Value::Ok(Arc::new(Value::Int(2))));
        assert_eq!(
            events.lock().unwrap().as_slice(),
            [
                "nivren.handle.open:database",
                "nivren.handle.call:query",
                "nivren.handle.close",
            ]
        );
    }
}

#[test]
fn desktop_host_is_typed_scoped_and_equivalent_in_vm_and_native_control() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proofs/edition4/desktop_host.niv");
    let program = nivren::modules::load(&path).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    for native in [false, true] {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let mut interpreter = Interpreter::new()
            .with_capabilities(["Native".to_string()])
            .with_host_callback(move |operation, request| {
                captured.lock().unwrap().push(operation.to_string());
                match operation {
                    "nivren.handle.open:desktop" => {
                        assert!(request.contains("app://index.html"));
                        Ok("desktop-proof-handle".into())
                    }
                    "nivren.handle.call:bridge" => {
                        assert!(request.contains("preferences.load"));
                        Ok("{\"theme\":\"dark\"}".into())
                    }
                    "nivren.handle.close" => Ok("closed".into()),
                    _ => Err(format!("unexpected desktop host operation {operation}")),
                }
            });
        let result = if native {
            interpreter.run_native(&chunk)
        } else {
            interpreter.run_bytecode(&chunk)
        };
        assert_eq!(
            result.unwrap(),
            Value::Ok(Arc::new(Value::String("{\"theme\":\"dark\"}".into())))
        );
        assert_eq!(
            events.lock().unwrap().as_slice(),
            [
                "nivren.handle.open:desktop",
                "nivren.handle.call:bridge",
                "nivren.handle.close",
            ]
        );
    }
}

#[test]
fn real_windows_webview_host_round_trips_the_bridge_or_reports_the_matrix() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proofs/edition4/desktop_host.niv");
    let program = nivren::modules::load(&path).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    for native in [false, true] {
        let host = nivren_desktop_host::DesktopHost::new();
        let mut interpreter = Interpreter::new()
            .with_capabilities(["Native".to_string()])
            .with_host_callback(host.callback());
        let result = if native {
            interpreter.run_native(&chunk)
        } else {
            interpreter.run_bytecode(&chunk)
        };
        let rendered = format!("{:?}", result.unwrap());
        if cfg!(windows) {
            // The shell page answered through real WebView2 page script.
            assert!(
                rendered.contains(r#""handled":true"#) && rendered.contains("request-1"),
                "expected a live bridge response, got: {rendered}"
            );
        } else {
            assert!(
                rendered.contains("available on Windows first"),
                "expected the platform matrix report, got: {rendered}"
            );
        }
    }
}

#[test]
fn real_webgpu_host_computes_or_reports_the_unavailable_matrix() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proofs/edition4/gpu_host.niv");
    let program = nivren::modules::load(&path).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    let probe = nivren_gpu_host::GpuHost::new();
    let adapter_present = probe
        .dispatch("nivren.handle.open:gpu", "webgpu-wgsl")
        .is_ok();
    for native in [false, true] {
        let host = nivren_gpu_host::GpuHost::new();
        let mut interpreter = Interpreter::new()
            .with_capabilities(["Native".to_string()])
            .with_host_callback(host.callback());
        let result = if native {
            interpreter.run_native(&chunk)
        } else {
            interpreter.run_bytecode(&chunk)
        };
        let result = result.unwrap();
        if adapter_present {
            assert_eq!(
                result,
                Value::Ok(Arc::new(Value::Array(Arc::new(vec![
                    Value::Int(11),
                    Value::Int(22),
                    Value::Int(33),
                    Value::Int(44),
                ]))))
            );
        } else {
            let rendered = format!("{result:?}");
            assert!(
                rendered.contains("no GPU adapter is available on this host"),
                "expected the clean unavailable report, got: {rendered}"
            );
        }
    }
}

#[test]
fn gpu_host_matches_checked_cpu_fallback_in_vm_and_native_control() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proofs/edition4/gpu_host.niv");
    let program = nivren::modules::load(&path).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    for native in [false, true] {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let mut interpreter = Interpreter::new()
            .with_capabilities(["Native".to_string()])
            .with_host_callback(move |operation, request| {
                captured.lock().unwrap().push(operation.to_string());
                match operation {
                    "nivren.handle.open:gpu" if request == "webgpu-wgsl" => {
                        Ok("gpu-proof-handle".into())
                    }
                    "nivren.handle.call:compute" => {
                        assert!(request.contains("webgpu-wgsl"));
                        assert!(request.contains("cpu_fallback"));
                        Ok("{\"values\":[11,22,33,44]}".into())
                    }
                    "nivren.handle.close" => Ok("closed".into()),
                    _ => Err(format!("unexpected GPU host operation {operation}")),
                }
            });
        let result = if native {
            interpreter.run_native(&chunk)
        } else {
            interpreter.run_bytecode(&chunk)
        };
        assert_eq!(
            result.unwrap(),
            Value::Ok(Arc::new(Value::Array(Arc::new(vec![
                Value::Int(11),
                Value::Int(22),
                Value::Int(33),
                Value::Int(44),
            ]))))
        );
        assert_eq!(
            events.lock().unwrap().as_slice(),
            [
                "nivren.handle.open:gpu",
                "nivren.handle.call:compute",
                "nivren.handle.close",
            ]
        );
    }
}

#[test]
fn bundled_sqlite_host_executes_real_edition_four_driver_workflow() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proofs/edition4/sqlite_driver.niv");
    let program = nivren::modules::load(&path).unwrap();
    nivren::typecheck::check(&program).unwrap();
    let chunk = nivren::bytecode::compile(&program).unwrap();
    for native in [false, true] {
        let root = std::env::temp_dir().join(format!(
            "nivren-sqlite-host-{}-{native}",
            std::process::id()
        ));
        let host = nivren_database_host::DatabaseHost::new(&root).unwrap();
        let mut interpreter = Interpreter::new()
            .with_capabilities(["Native".to_string()])
            .with_host_callback(host.callback());
        let result = if native {
            interpreter.run_native(&chunk)
        } else {
            interpreter.run_bytecode(&chunk)
        };
        assert_eq!(result.unwrap(), Value::Ok(Arc::new(Value::Int(2))));
        let _ = std::fs::remove_dir(&root);
    }
}

fn nivren_markdown_blocks(markdown: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current = None;
    for line in markdown.lines() {
        if line.trim() == "```nivren" {
            current = Some(String::new());
        } else if line.trim() == "```" {
            if let Some(source) = current.take() {
                blocks.push(source);
            }
        } else if let Some(source) = current.as_mut() {
            source.push_str(line);
            source.push('\n');
        }
    }
    assert!(current.is_none(), "unterminated nivren documentation fence");
    blocks
}

#[test]
fn release_facing_language_snippets_are_checked_edition_four() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let guides = [
        root.join("docs/LANGUAGE.md"),
        root.join("docs/STYLE_GUIDE.md"),
        root.join("docs/GETTING_STARTED.md"),
    ];
    let mut checked = 0;
    for guide in guides {
        let markdown = std::fs::read_to_string(&guide).unwrap();
        for (index, source) in nivren_markdown_blocks(&markdown).into_iter().enumerate() {
            nivren::check(&source).unwrap_or_else(|errors| {
                panic!(
                    "{} Nivren block {} did not check: {errors:?}\n{source}",
                    guide.display(),
                    index + 1
                )
            });
            checked += 1;
        }
    }
    assert_eq!(
        checked, 12,
        "release guide snippet count changed; review it"
    );
}

#[test]
fn current_release_docs_do_not_reintroduce_edition_three_claims() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for relative in [
        "docs/LANGUAGE.md",
        "docs/STANDARD_LIBRARY.md",
        "docs/STYLE_GUIDE.md",
        "docs/GETTING_STARTED.md",
        "docs/RELEASE_AUDIT.md",
    ] {
        let contents = std::fs::read_to_string(root.join(relative)).unwrap();
        for stale in [
            "Edition 3 draft",
            "local Edition 3",
            "new Edition 3 examples",
            "Edition 3 style corpus",
            "Twenty-two official packages",
        ] {
            assert!(
                !contents.contains(stale),
                "{relative} contains stale release-facing text: {stale}"
            );
        }
    }
    for entry in std::fs::read_dir(root.join("packages")).unwrap() {
        let readme = entry.unwrap().path().join("README.md");
        if readme.is_file() {
            let contents = std::fs::read_to_string(&readme).unwrap();
            assert!(
                !contents.contains("for Nivren Edition 3"),
                "{} still advertises Edition 3",
                readme.display()
            );
        }
    }
}
