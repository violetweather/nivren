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
