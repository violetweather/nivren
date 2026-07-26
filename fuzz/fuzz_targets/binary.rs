#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 66 * 1024 * 1024 {
        return;
    }
    let _ = nivren::bundle::decode(data);
    let _ = nivren::package::Package::decode(data);
    let _ = nivren::trust::PublishEnvelope::decode(data);
    if let Ok(source) = std::str::from_utf8(data) {
        let _ = nivren::json::compact(source);
    }
});
