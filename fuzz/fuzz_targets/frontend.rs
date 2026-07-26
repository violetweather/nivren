#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 1024 * 1024 {
        return;
    }
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(tokens) = nivren::lexer::scan(source) else {
        return;
    };
    let Ok(program) = nivren::parser::parse(tokens) else {
        return;
    };
    if nivren::typecheck::check(&program).is_err() {
        return;
    }
    let Ok(chunk) = nivren::bytecode::compile(&program) else {
        return;
    };
    if let Ok(bundle) = nivren::bundle::encode(&chunk) {
        let _ = nivren::bundle::decode(&bundle);
    }
});
