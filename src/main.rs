use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

use nivren::error::NivError;
use nivren::runtime::{Interpreter, Value};

/// The interpreter allocates managed values constantly; mimalloc turns those
/// small allocations from system-heap calls into fast thread-local bumps.
/// Wasm targets keep their host allocator — the dependency is not built there.
#[cfg(not(target_family = "wasm"))]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    if let Some(code) = run_embedded_application() {
        return code;
    }
    let arguments: Vec<String> = env::args().skip(1).collect();
    match arguments.as_slice() {
        [] => repl(),
        [command] if command == "repl" => repl(),
        [command] if command == "lsp" => lsp(),
        [command] if command == "dap" => dap(),
        [command] if command == "version" || command == "--version" || command == "-V" => {
            println!("Nivren {}", nivren::VERSION);
            ExitCode::SUCCESS
        }
        [command] if command == "help" || command == "--help" || command == "-h" => {
            help();
            ExitCode::SUCCESS
        }
        [command] if command == "run" => run_file("."),
        [command, flag] if command == "run" && flag == "--native" => run_native_file("."),
        [command, flag, path] if command == "run" && flag == "--native" => run_native_file(path),
        [command, path] if command == "run" => run_file(path),
        [command, path, output] if command == "record" => record_file(path, output),
        [command, path, trace] if command == "replay" => replay_file(path, trace),
        [command, flag, output, path] if command == "run" && flag == "--crash-report" => {
            run_with_crash_report(path, output)
        }
        [command, path] if command == "new" => new_project(path),
        [command, name, version] if command == "add" => add_dependency(".", name, version),
        [command, name, version, path] if command == "add" => add_dependency(path, name, version),
        [command] if command == "dev" => run_project("."),
        [command, path] if command == "dev" => run_project(path),
        [command] if command == "ship" => ship_project("."),
        [command, path] if command == "ship" => ship_project(path),
        [command, action] if command == "workspace" => workspace_action(action, "."),
        [command, action, path] if command == "workspace" => workspace_action(action, path),
        [command, path] if command == "check" => check_file(path),
        [command] if command == "build" => build_project("."),
        [command, flag] if command == "build" && flag == "--standalone" => build_standalone("."),
        [command, standalone, native]
            if command == "build" && standalone == "--standalone" && native == "--native" =>
        {
            build_native_standalone(".")
        }
        [command, standalone, native, path]
            if command == "build" && standalone == "--standalone" && native == "--native" =>
        {
            build_native_standalone(path)
        }
        [command, flag, path] if command == "build" && flag == "--standalone" => {
            build_standalone(path)
        }
        [command, flag] if command == "build" && flag == "--aot" => build_aot("."),
        [command, flag, path] if command == "build" && flag == "--aot" => build_aot(path),
        [command, path] if command == "build" => build_project(path),
        [command, flag, registry, root] if command == "install" && flag == "--trusted" => {
            install_trusted_project(".", registry, root)
        }
        [command, flag, registry, root, path] if command == "install" && flag == "--trusted" => {
            install_trusted_project(path, registry, root)
        }
        [command, flag] if command == "install" && flag == "--offline" => {
            install_offline_project(".")
        }
        [command, flag, path] if command == "install" && flag == "--offline" => {
            install_offline_project(path)
        }
        [command, action] if command == "cache" && action == "list" => cache_list("."),
        [command, action, path] if command == "cache" && action == "list" => cache_list(path),
        [command, action] if command == "cache" && action == "prune" => cache_prune("."),
        [command, action, path] if command == "cache" && action == "prune" => cache_prune(path),
        [command, action] if command == "authority" && action == "lock" => authority_lock("."),
        [command, action, path] if command == "authority" && action == "lock" => {
            authority_lock(path)
        }
        [command, action] if command == "authority" && action == "check" => authority_check("."),
        [command, action, path] if command == "authority" && action == "check" => {
            authority_check(path)
        }
        [command, action] if command == "authority" && action == "report" => authority_report("."),
        [command, action, path] if command == "authority" && action == "report" => {
            authority_report(path)
        }
        [command, registry] if command == "install" => install_project(".", registry),
        [command, registry, path] if command == "install" => install_project(path, registry),
        [command] if command == "package" => package_project("."),
        [command, path] if command == "package" => package_project(path),
        [command, action, path] if command == "package" && action == "verify" => {
            verify_package(path)
        }
        [command, action, package, registry] if command == "registry" && action == "publish" => {
            registry_publish(package, registry)
        }
        [command, action, query, registry] if command == "registry" && action == "search" => {
            registry_search(query, registry)
        }
        [command, action, name, version, registry] if command == "registry" && action == "yank" => {
            registry_set_yanked(name, version, registry, true)
        }
        [command, action, name, version, registry]
            if command == "registry" && action == "unyank" =>
        {
            registry_set_yanked(name, version, registry, false)
        }
        [command, action, name, version, registry, destination]
            if command == "registry" && action == "fetch" =>
        {
            registry_fetch(name, version, registry, destination)
        }
        [
            command,
            action,
            package,
            provenance,
            authorization,
            status,
            advisories,
            root,
            now,
            minimum,
        ] if command == "registry" && action == "verify-release" => registry_verify_release(
            package,
            provenance,
            authorization,
            status,
            advisories,
            root,
            now,
            minimum,
        ),
        [command, action, registry, bind] if command == "registry" && action == "serve" => {
            registry_serve(registry, bind, "0")
        }
        [command, action, registry, bind, minimum]
            if command == "registry" && action == "serve" =>
        {
            registry_serve(registry, bind, minimum)
        }
        [command, action, package, provenance, authorization, output]
            if command == "registry" && action == "envelope" =>
        {
            registry_envelope(package, provenance, authorization, output)
        }
        [
            command,
            action,
            operation,
            name,
            version,
            generation,
            issued,
            expires,
            reason,
            secret,
            output,
        ] if command == "registry" && action == "sign-admin" => registry_sign_admin(
            operation, name, version, generation, issued, expires, reason, secret, output,
        ),
        [command, action, document, public, now, minimum]
            if command == "registry" && action == "verify-admin" =>
        {
            registry_verify_admin(document, public, now, minimum)
        }
        [command, action, registry, now, minimum]
            if command == "registry" && action == "recover-admin" =>
        {
            registry_recover_admin(registry, now, minimum)
        }
        [command, action, output] if command == "trust" && action == "keygen" => {
            trust_keygen(output)
        }
        [
            command,
            action,
            publisher,
            key,
            repository,
            workflow,
            expires,
            secret,
            output,
        ] if command == "trust" && action == "authorize" => trust_authorize(
            publisher, key, repository, workflow, expires, secret, output,
        ),
        [
            command,
            action,
            package,
            publisher,
            repository,
            workflow,
            commit,
            secret,
            output,
        ] if command == "trust" && action == "attest" => trust_attest(
            package, publisher, repository, workflow, commit, secret, output,
        ),
        [command, action, input, secret, output]
            if command == "trust" && action == "sign-status" =>
        {
            trust_sign_status(input, secret, output, None)
        }
        [command, action, input, secret, output, advisories]
            if command == "trust" && action == "sign-status" =>
        {
            trust_sign_status(input, secret, output, Some(advisories))
        }
        [command, action, input, secret, output]
            if command == "trust" && action == "sign-advisory" =>
        {
            trust_sign_advisory(input, secret, output)
        }
        [command, action] if command == "release" && action == "check" => release_check("."),
        [command, action, path] if command == "release" && action == "check" => release_check(path),
        [command, action, manifest, secret, output]
            if command == "release" && action == "sign-channel" =>
        {
            release_sign_channel(manifest, secret, output)
        }
        [command, action, manifest, public, now, minimum]
            if command == "release" && action == "verify-channel" =>
        {
            release_verify_channel(manifest, public, now, minimum, None)
        }
        [command, action, manifest, public, now, minimum, channel]
            if command == "release" && action == "verify-channel" =>
        {
            release_verify_channel(manifest, public, now, minimum, Some(channel))
        }
        [command, path] if command == "fmt" => format_path(path, false),
        [command, flag, path] if command == "fmt" && flag == "--check" => format_path(path, true),
        [command, language, path, output] if command == "bindgen" && language == "c" => {
            generate_c_bindings(path, output)
        }
        [command] if command == "doc" => document_project("."),
        [command, path] if command == "doc" => document_project(path),
        [command, path] if command == "disasm" => disassemble_path(path),
        [command, path] if command == "explain" => explain_path(path, true),
        [command, flag, path] if command == "explain" && flag == "--no-optimize" => {
            explain_path(path, false)
        }
        [command, flag, path] if command == "explain" && flag == "--story" => {
            explain_story_path(path)
        }
        [command, path, output] if command == "sourcemap" => source_map_path(path, output),
        [command, path] if command == "debug" => debug_path(path),
        [command, path, output] if command == "inspect" => inspect_path(path, output),
        [command, path] if command == "profile" => observe_path(path, false, None),
        [command, flag, output, path] if command == "profile" && flag == "--json" => {
            observe_path(path, false, Some(output))
        }
        [command, path] if command == "coverage" => observe_path(path, true, None),
        [command, flag, output, path] if command == "coverage" && flag == "--json" => {
            observe_path(path, true, Some(output))
        }
        [command] if command == "bench" => benchmark_path(".", None),
        [command, path] if command == "bench" => benchmark_path(path, None),
        [command, flag, output, path] if command == "bench" && flag == "--json" => {
            benchmark_path(path, Some(output))
        }
        [command] if command == "test" => test_path("tests/niv", None, None),
        [command, flag] if command == "test" && flag == "--property" => {
            test_path("tests/property", None, None)
        }
        [command, flag, path] if command == "test" && flag == "--property" => {
            test_path(path, None, None)
        }
        [command, flag] if command == "test" && flag == "--compat" => {
            test_path("tests/compat", None, None)
        }
        [command, flag, path] if command == "test" && flag == "--compat" => {
            test_path(path, None, None)
        }
        [command, flag] if command == "test" && flag == "--fuzz-smoke" => {
            test_path("tests/fuzz", None, None)
        }
        [command, flag, path] if command == "test" && flag == "--fuzz-smoke" => {
            test_path(path, None, None)
        }
        [command, flag, seconds] if command == "test" && flag == "--time" => {
            test_path_with_time("tests/niv", seconds)
        }
        [command, flag, seconds, path] if command == "test" && flag == "--time" => {
            test_path_with_time(path, seconds)
        }
        [command, flag] if command == "test" && flag == "--snapshots" => {
            test_path("tests/niv", Some(false), None)
        }
        [command, flag, path] if command == "test" && flag == "--snapshots" => {
            test_path(path, Some(false), None)
        }
        [command, flag] if command == "test" && flag == "--accept-snapshots" => {
            test_path("tests/niv", Some(true), None)
        }
        [command, flag, path] if command == "test" && flag == "--accept-snapshots" => {
            test_path(path, Some(true), None)
        }
        [command, path] if command == "test" => test_path(path, None, None),
        [path] if path.ends_with(".niv") => run_file(path),
        [path] if path.ends_with(".nivb") => run_file(path),
        _ => {
            eprintln!("error: invalid command\n");
            help();
            ExitCode::from(64)
        }
    }
}

fn read_secret_key(path: &str) -> Result<[u8; 32], String> {
    let text = zeroize::Zeroizing::new(
        fs::read_to_string(path)
            .map_err(|error| format!("cannot read signing key {path}: {error}"))?,
    );
    nivren::trust::parse_secret_key(text.trim()).map_err(|error| error.message)
}

fn unix_now() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| "system clock is before the Unix epoch".to_string())
}

fn trust_result(result: Result<String, String>) -> ExitCode {
    match result {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(65)
        }
    }
}

/// Writes a secret key file readable only by its owner, refusing to touch an
/// existing file.
fn write_secret_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

fn trust_keygen(output: &str) -> ExitCode {
    trust_result((|| {
        let mut secret = zeroize::Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut *secret)
            .map_err(|error| format!("cannot gather key entropy: {error}"))?;
        let path = Path::new(output);
        if path.exists() {
            return Err(format!("refusing to overwrite existing key file {output}"));
        }
        let encoded = zeroize::Zeroizing::new(format!("{}\n", nivren::trust::encode_hex(&*secret)));
        write_secret_file(path, encoded.as_bytes()).map_err(|error| error.to_string())?;
        let public = nivren::trust::public_key(*secret);
        Ok(format!(
            "secret {output}
public {}",
            nivren::trust::encode_hex(&public)
        ))
    })())
}

fn trust_authorize(
    publisher: &str,
    key: &str,
    repository: &str,
    workflow: &str,
    expires: &str,
    secret: &str,
    output: &str,
) -> ExitCode {
    trust_result((|| {
        let public = fs::read_to_string(key)
            .map_err(|error| format!("cannot read publisher key {key}: {error}"))?;
        nivren::trust::parse_public_key(public.trim()).map_err(|error| error.message)?;
        let expires = expires
            .parse::<u64>()
            .map_err(|_| "expires must be a Unix time in seconds".to_string())?;
        let root_secret = read_secret_key(secret)?;
        let authorization = nivren::trust::authorize_publisher(
            root_secret,
            publisher.to_string(),
            public.trim().to_string(),
            repository.to_string(),
            workflow.to_string(),
            expires,
        )
        .map_err(|error| error.message)?;
        let encoded = serde_json::to_vec_pretty(&authorization)
            .map_err(|error| format!("cannot encode authorization: {error}"))?;
        write_atomic(Path::new(output), &encoded).map_err(|error| error.to_string())?;
        Ok(format!(
            "authorized {publisher} until {expires} -> {output}"
        ))
    })())
}

fn trust_attest(
    package: &str,
    publisher: &str,
    repository: &str,
    workflow: &str,
    commit: &str,
    secret: &str,
    output: &str,
) -> ExitCode {
    trust_result((|| {
        let package_bytes =
            fs::read(package).map_err(|error| format!("cannot read package {package}: {error}"))?;
        let publisher_secret = read_secret_key(secret)?;
        let provenance = nivren::trust::attest_release(
            publisher_secret,
            &package_bytes,
            publisher.to_string(),
            repository.to_string(),
            workflow.to_string(),
            commit.to_string(),
            unix_now()?,
        )
        .map_err(|error| error.message)?;
        let encoded = serde_json::to_vec_pretty(&provenance)
            .map_err(|error| format!("cannot encode provenance: {error}"))?;
        write_atomic(Path::new(output), &encoded).map_err(|error| error.to_string())?;
        Ok(format!(
            "attested {} {} -> {output}",
            provenance.package, provenance.version
        ))
    })())
}

fn trust_sign_status(
    input: &str,
    secret: &str,
    output: &str,
    advisories: Option<&str>,
) -> ExitCode {
    trust_result((|| {
        let text = fs::read_to_string(input)
            .map_err(|error| format!("cannot read status {input}: {error}"))?;
        let mut status: nivren::trust::RegistryStatus = serde_json::from_str(&text)
            .map_err(|error| format!("invalid registry status: {error}"))?;
        if let Some(advisories) = advisories {
            let text = fs::read_to_string(advisories)
                .map_err(|error| format!("cannot read advisories {advisories}: {error}"))?;
            let advisories: Vec<nivren::trust::Advisory> = serde_json::from_str(&text)
                .map_err(|error| format!("invalid advisories: {error}"))?;
            status.advisories_sha256 = nivren::trust::advisories_sha256(&advisories);
        } else if status.advisories_sha256.is_empty() {
            return Err(
                "status has no advisories_sha256; pass the served advisories.json as the fourth argument"
                    .to_string(),
            );
        }
        let root_secret = read_secret_key(secret)?;
        let signed = nivren::trust::sign_status(root_secret, status);
        let encoded = serde_json::to_vec_pretty(&signed)
            .map_err(|error| format!("cannot encode status: {error}"))?;
        write_atomic(Path::new(output), &encoded).map_err(|error| error.to_string())?;
        Ok(format!(
            "signed generation {} -> {output}",
            signed.generation
        ))
    })())
}

fn trust_sign_advisory(input: &str, secret: &str, output: &str) -> ExitCode {
    trust_result((|| {
        let text = fs::read_to_string(input)
            .map_err(|error| format!("cannot read advisory {input}: {error}"))?;
        let advisory: nivren::trust::Advisory =
            serde_json::from_str(&text).map_err(|error| format!("invalid advisory: {error}"))?;
        let root_secret = read_secret_key(secret)?;
        let signed = nivren::trust::sign_advisory(root_secret, advisory);
        let encoded = serde_json::to_vec_pretty(&signed)
            .map_err(|error| format!("cannot encode advisory: {error}"))?;
        write_atomic(Path::new(output), &encoded).map_err(|error| error.to_string())?;
        Ok(format!("signed advisory {} -> {output}", signed.id))
    })())
}

fn release_sign_channel(manifest: &str, secret: &str, output: &str) -> ExitCode {
    let result = fs::read(manifest)
        .map_err(|error| format!("cannot read channel manifest: {error}"))
        .and_then(|bytes| {
            nivren::channel::ChannelManifest::decode(&bytes).map_err(|error| error.message)
        })
        .and_then(|mut manifest| {
            let secret = fs::read_to_string(secret)
                .map_err(|error| format!("cannot read channel signing key: {error}"))?;
            manifest
                .sign(secret.trim())
                .map_err(|error| error.message)?;
            manifest.encode().map_err(|error| error.message)
        })
        .and_then(|bytes| {
            write_atomic(Path::new(output), &bytes).map_err(|error| error.to_string())
        });
    match result {
        Ok(()) => {
            println!("signed {output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(65)
        }
    }
}

fn release_verify_channel(
    manifest: &str,
    public: &str,
    now: &str,
    minimum: &str,
    expected_channel: Option<&str>,
) -> ExitCode {
    let result = fs::read(manifest)
        .map_err(|error| format!("cannot read channel manifest: {error}"))
        .and_then(|bytes| {
            nivren::channel::ChannelManifest::decode(&bytes).map_err(|error| error.message)
        })
        .and_then(|manifest| {
            let public = fs::read_to_string(public)
                .map_err(|error| format!("cannot read channel public key: {error}"))?;
            let now = now
                .parse::<u64>()
                .map_err(|_| "invalid Unix time".to_string())?;
            let minimum = minimum
                .parse::<u64>()
                .map_err(|_| "invalid minimum generation".to_string())?;
            manifest
                .verify(public.trim(), now, minimum, expected_channel)
                .map_err(|error| error.message)?;
            Ok(manifest)
        });
    match result {
        Ok(manifest) => {
            println!(
                "verified {} {} generation {}",
                manifest.channel, manifest.version, manifest.generation
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(65)
        }
    }
}

fn generate_c_bindings(path: &str, output: &str) -> ExitCode {
    let source = match read_source(path) {
        Ok(source) => source,
        Err(code) => return code,
    };
    let header = match nivren::compiler::Compiler::new().c_bindings(&source) {
        Ok(header) => header,
        Err(errors) => {
            for error in errors {
                eprintln!(
                    "{path}:{}:{}: error: {}",
                    error.line, error.column, error.message
                );
            }
            return ExitCode::from(65);
        }
    };
    match fs::write(output, header) {
        Ok(()) => {
            println!("generated {output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: cannot write '{output}': {error}");
            ExitCode::from(74)
        }
    }
}

fn dap() -> ExitCode {
    match nivren::dap::serve() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {}", error.message);
            ExitCode::from(70)
        }
    }
}

fn run_embedded_application() -> Option<ExitCode> {
    let executable = env::current_exe().ok()?;
    let bytes = fs::read(&executable).ok()?;
    let application = nivren::standalone::extract(&bytes)?;
    let root = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let manifest = match nivren::project::Manifest::parse(&application.manifest, root) {
        Ok(manifest) => manifest,
        Err(error) => {
            report(&executable.display().to_string(), "", &[error]);
            return Some(ExitCode::from(70));
        }
    };
    let native = application
        .manifest
        .lines()
        .next()
        .is_some_and(|line| line == "# nivren-standalone-engine = native");
    let result = nivren::bundle::decode(&application.bundle).and_then(|chunk| {
        let mut interpreter = project_interpreter(&manifest);
        if native {
            interpreter.run_native(&chunk)
        } else {
            interpreter.run_bytecode(&chunk)
        }
    });
    Some(match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            report(&executable.display().to_string(), "", &[error]);
            ExitCode::from(70)
        }
    })
}

fn run_file(path: &str) -> ExitCode {
    if is_project_path(Path::new(path)) {
        return run_project(path);
    }
    if is_bundle_path(Path::new(path)) {
        return run_bundle(path);
    }
    let source = match read_source(path) {
        Ok(source) => source,
        Err(code) => return code,
    };
    match compile_file(Path::new(path)).and_then(|chunk| {
        Interpreter::new()
            .run_bytecode(&chunk)
            .map_err(|error| vec![error])
    }) {
        Ok(_) => ExitCode::SUCCESS,
        Err(errors) => {
            report(path, &source, &errors);
            ExitCode::from(70)
        }
    }
}

fn record_file(path: &str, output: &str) -> ExitCode {
    let source = match read_source(path) {
        Ok(source) => source,
        Err(code) => return code,
    };
    let mut interpreter = Interpreter::new();
    let recorder = interpreter.record_effects();
    match compile_file(Path::new(path)).and_then(|chunk| {
        interpreter
            .run_bytecode(&chunk)
            .map_err(|error| vec![error])
    }) {
        Ok(_) => {
            let entries = recorder.lock().unwrap();
            let mut trace = String::from("{\"schema\":\"org.nivren.effects.v1\"}\n");
            for entry in entries.iter() {
                trace.push_str(
                    &serde_json::json!({
                        "operation": entry.operation,
                        "capability": entry.capability,
                        "arguments": entry.arguments,
                        "result": entry.result,
                    })
                    .to_string(),
                );
                trace.push('\n');
            }
            if let Err(error) = write_atomic(Path::new(output), trace.as_bytes()) {
                eprintln!("error: cannot write {output}: {error}");
                return ExitCode::from(74);
            }
            println!("recorded {} effect(s) to {output}", entries.len());
            ExitCode::SUCCESS
        }
        Err(errors) => {
            report(path, &source, &errors);
            ExitCode::from(70)
        }
    }
}

fn replay_file(path: &str, trace: &str) -> ExitCode {
    let source = match read_source(path) {
        Ok(source) => source,
        Err(code) => return code,
    };
    let recorded = match fs::read_to_string(trace) {
        Ok(recorded) => recorded,
        Err(error) => {
            eprintln!("error: cannot read {trace}: {error}");
            return ExitCode::from(66);
        }
    };
    let mut lines = recorded.lines();
    if lines.next().map(str::trim) != Some("{\"schema\":\"org.nivren.effects.v1\"}") {
        eprintln!("error: {trace} is not an org.nivren.effects.v1 trace");
        return ExitCode::from(65);
    }
    let mut entries = vec![];
    for (index, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let parsed: serde_json::Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(error) => {
                eprintln!("error: trace entry {} is not JSON: {error}", index + 1);
                return ExitCode::from(65);
            }
        };
        let text = |key: &str| {
            parsed
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        };
        let (Some(operation), Some(capability), Some(arguments), Some(result)) = (
            text("operation"),
            text("capability"),
            text("arguments"),
            parsed.get("result").cloned(),
        ) else {
            eprintln!("error: trace entry {} is incomplete", index + 1);
            return ExitCode::from(65);
        };
        entries.push(nivren::runtime::EffectRecord {
            operation,
            capability,
            arguments,
            result,
        });
    }
    let mut interpreter = Interpreter::new();
    interpreter.replay_effects(entries);
    match compile_file(Path::new(path)).and_then(|chunk| {
        interpreter
            .run_bytecode(&chunk)
            .map_err(|error| vec![error])
    }) {
        Ok(_) => {
            let remaining = interpreter.replay_remaining();
            if remaining > 0 {
                eprintln!(
                    "error: replay diverged: {remaining} trace entr(y/ies) were never performed"
                );
                return ExitCode::from(70);
            }
            ExitCode::SUCCESS
        }
        Err(errors) => {
            report(path, &source, &errors);
            ExitCode::from(70)
        }
    }
}

fn run_native_file(path: &str) -> ExitCode {
    if is_project_path(Path::new(path)) {
        return run_native_project(path);
    }
    let source = if is_bundle_path(Path::new(path)) {
        String::new()
    } else {
        match read_source(path) {
            Ok(source) => source,
            Err(code) => return code,
        }
    };
    let result = if is_bundle_path(Path::new(path)) {
        fs::read(path)
            .map_err(|error| vec![NivError::new(error.to_string(), 1, 1)])
            .and_then(|bytes| nivren::bundle::decode(&bytes).map_err(|error| vec![error]))
    } else {
        compile_file(Path::new(path))
    }
    .and_then(|chunk| {
        Interpreter::new()
            .run_native(&chunk)
            .map_err(|error| vec![error])
    });
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(errors) => {
            report(path, &source, &errors);
            ExitCode::from(70)
        }
    }
}

fn run_bundle(path: &str) -> ExitCode {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("error: cannot read {path}: {error}");
            return ExitCode::from(66);
        }
    };
    let result =
        nivren::bundle::decode(&bytes).and_then(|chunk| Interpreter::new().run_bytecode(&chunk));
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            report(path, "", &[error]);
            ExitCode::from(70)
        }
    }
}

fn run_project(path: &str) -> ExitCode {
    let manifest = match nivren::project::Manifest::load(Path::new(path)) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!(
                "{path}:{}:{}: error: {}",
                error.line, error.column, error.message
            );
            return ExitCode::from(65);
        }
    };
    let entry = manifest.entry_path();
    let source = fs::read_to_string(&entry).unwrap_or_default();
    let result = compile_project(Path::new(path)).and_then(|(manifest, chunk)| {
        project_interpreter(&manifest)
            .run_bytecode(&chunk)
            .map_err(|error| vec![error])
    });
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(errors) => {
            report(&entry.display().to_string(), &source, &errors);
            ExitCode::from(70)
        }
    }
}

fn run_native_project(path: &str) -> ExitCode {
    let manifest = match nivren::project::Manifest::load(Path::new(path)) {
        Ok(manifest) => manifest,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(65);
        }
    };
    let entry = manifest.entry_path();
    let source = fs::read_to_string(&entry).unwrap_or_default();
    let result = compile_project(Path::new(path)).and_then(|(manifest, chunk)| {
        project_interpreter(&manifest)
            .run_native(&chunk)
            .map_err(|error| vec![error])
    });
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(errors) => {
            report(&entry.display().to_string(), &source, &errors);
            ExitCode::from(70)
        }
    }
}

/// Which bundled host serves one request: opens route by kind, later
/// calls and closes route by the handle's backend prefix.
#[derive(PartialEq)]
enum BundledHost {
    Database,
    Gpu,
    Desktop,
}

fn bundled_host_for(operation: &str, request: &str) -> BundledHost {
    match operation {
        "nivren.handle.open:gpu" => return BundledHost::Gpu,
        "nivren.handle.open:desktop" => return BundledHost::Desktop,
        "nivren.handle.close" => return bundled_host_for_handle(request),
        _ => {}
    }
    if operation.starts_with("nivren.handle.call:") {
        let handle = serde_json::from_str::<serde_json::Value>(request)
            .ok()
            .and_then(|envelope| {
                envelope
                    .get("handle")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            });
        if let Some(handle) = handle {
            return bundled_host_for_handle(&handle);
        }
    }
    BundledHost::Database
}

fn bundled_host_for_handle(handle: &str) -> BundledHost {
    if handle.starts_with("gpu-") {
        BundledHost::Gpu
    } else if handle.starts_with("desktop-") {
        BundledHost::Desktop
    } else {
        BundledHost::Database
    }
}

fn project_interpreter(manifest: &nivren::project::Manifest) -> Interpreter {
    let interpreter = Interpreter::new()
        .with_capabilities(manifest.capabilities.iter().cloned())
        .with_capability_scopes(manifest.capability_scopes.clone());
    let interpreter = if manifest.capabilities.contains("Native") {
        let database_root = manifest.root.join(".nivren").join("database");
        match nivren_database_host::DatabaseHost::new(database_root) {
            Ok(database) => {
                let database = database.callback();
                let gpu = nivren_gpu_host::GpuHost::new().callback();
                let desktop = nivren_desktop_host::DesktopHost::new().callback();
                interpreter.with_host_callback(move |operation, request| {
                    match bundled_host_for(operation, request) {
                        BundledHost::Gpu => gpu(operation, request),
                        BundledHost::Desktop => desktop(operation, request),
                        BundledHost::Database => database(operation, request),
                    }
                })
            }
            Err(error) => interpreter.with_host_callback(move |operation, _| {
                Err(format!(
                    "cannot initialize the built-in database host for {operation}: {error}"
                ))
            }),
        }
    } else {
        interpreter
    };
    let interpreter = if let Some(limit) = manifest.instruction_limit {
        interpreter.with_instruction_limit(limit)
    } else {
        interpreter
    };
    let interpreter = if let Some(limit) = manifest.memory_limit {
        interpreter.with_memory_limit(limit)
    } else {
        interpreter
    };
    if let Some(limit) = manifest.payload_limit {
        interpreter.with_payload_limit(limit)
    } else {
        interpreter
    }
}

fn install_project(path: &str, registry: &str) -> ExitCode {
    let manifest = match nivren::project::Manifest::load(Path::new(path)) {
        Ok(manifest) => manifest,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(65);
        }
    };
    match nivren::package::install_dependencies(&manifest, Path::new(registry)) {
        Ok(count) => {
            println!("installed {count} package(s)");
            ExitCode::SUCCESS
        }
        Err(error) => {
            report(path, "", &[error]);
            ExitCode::from(65)
        }
    }
}

fn install_offline_project(path: &str) -> ExitCode {
    let manifest = match nivren::project::Manifest::load(Path::new(path)) {
        Ok(manifest) => manifest,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(65);
        }
    };
    match nivren::package::install_offline_dependencies(&manifest) {
        Ok(count) => {
            println!("verified {count} cached package(s); no network used");
            ExitCode::SUCCESS
        }
        Err(error) => {
            report(path, "", &[error]);
            ExitCode::from(65)
        }
    }
}

fn cache_list(path: &str) -> ExitCode {
    let manifest = match nivren::project::Manifest::load(Path::new(path)) {
        Ok(manifest) => manifest,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(65);
        }
    };
    match nivren::package::cache_entries(&manifest) {
        Ok(entries) => {
            for entry in &entries {
                println!(
                    "{} {} {} {} bytes {}",
                    entry.name,
                    entry.version,
                    entry.sha256,
                    entry.bytes,
                    if entry.reachable {
                        "reachable"
                    } else {
                        "unused"
                    }
                );
            }
            println!("{} cached package(s)", entries.len());
            ExitCode::SUCCESS
        }
        Err(error) => {
            report(path, "", &[error]);
            ExitCode::from(65)
        }
    }
}

fn cache_prune(path: &str) -> ExitCode {
    let manifest = match nivren::project::Manifest::load(Path::new(path)) {
        Ok(manifest) => manifest,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(65);
        }
    };
    match nivren::package::prune_cache(&manifest) {
        Ok((removed, bytes)) => {
            println!("removed {removed} unused package(s), {bytes} archive bytes");
            ExitCode::SUCCESS
        }
        Err(error) => {
            report(path, "", &[error]);
            ExitCode::from(65)
        }
    }
}

fn authority_lock(path: &str) -> ExitCode {
    let manifest = match nivren::project::Manifest::load(Path::new(path)) {
        Ok(manifest) => manifest,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(65);
        }
    };
    match nivren::package::write_authority_lock(&manifest) {
        Ok(()) => {
            println!(
                "wrote {}",
                manifest
                    .root
                    .join(nivren::project::AUTHORITY_LOCKFILE_NAME)
                    .display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            report(path, "", &[error]);
            ExitCode::from(65)
        }
    }
}

fn authority_check(path: &str) -> ExitCode {
    let manifest = match nivren::project::Manifest::load(Path::new(path)) {
        Ok(manifest) => manifest,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(65);
        }
    };
    let expected = match nivren::package::installed_authority_lockfile(&manifest) {
        Ok(contents) => contents,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(65);
        }
    };
    let lock = manifest.root.join(nivren::project::AUTHORITY_LOCKFILE_NAME);
    match fs::read_to_string(&lock) {
        Ok(actual) if actual == expected => {
            println!("authority lock is current");
            ExitCode::SUCCESS
        }
        Ok(actual) => {
            eprintln!("error: authority lock is stale; review and run 'niv authority lock'");
            eprint!("{}", nivren::package::authority_diff(&actual, &expected));
            ExitCode::FAILURE
        }
        Err(_) => {
            eprintln!("error: authority lock is missing; run 'niv authority lock'");
            ExitCode::FAILURE
        }
    }
}

fn authority_report(path: &str) -> ExitCode {
    let manifest = match nivren::project::Manifest::load(Path::new(path)) {
        Ok(manifest) => manifest,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(65);
        }
    };
    match nivren::package::installed_authority_lockfile(&manifest) {
        Ok(contents) => {
            print!("{contents}");
            println!("declared limits:");
            println!(
                "  instructions = {}",
                manifest
                    .instruction_limit
                    .map_or("default".to_string(), |limit| limit.to_string())
            );
            println!(
                "  memory_bytes = {}",
                manifest
                    .memory_limit
                    .map_or("default".to_string(), |limit| limit.to_string())
            );
            println!(
                "  payload_bytes = {}",
                manifest
                    .payload_limit
                    .map_or("default (16777216)".to_string(), |limit| limit.to_string())
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            report(path, "", &[error]);
            ExitCode::from(65)
        }
    }
}

fn install_trusted_project(path: &str, registry: &str, root: &str) -> ExitCode {
    let root_key = match fs::read_to_string(root)
        .map_err(|error| format!("cannot read trusted root key: {error}"))
        .and_then(|value| nivren::trust::parse_public_key(&value).map_err(|error| error.message))
    {
        Ok(key) => key,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(66);
        }
    };
    let manifest = match nivren::project::Manifest::load(Path::new(path)) {
        Ok(manifest) => manifest,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(65);
        }
    };
    match nivren::package::install_trusted_dependencies(&manifest, registry, root_key) {
        Ok(count) => {
            println!("installed {count} trusted package(s)");
            ExitCode::SUCCESS
        }
        Err(error) => {
            report(path, "", &[error]);
            ExitCode::from(65)
        }
    }
}

fn release_check(path: &str) -> ExitCode {
    let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs(),
        Err(_) => {
            eprintln!("error: system clock is before Unix epoch");
            return ExitCode::from(70);
        }
    };
    match nivren::release::audit(Path::new(path), now) {
        Ok(audit) if audit.blockers.is_empty() => {
            println!(
                "Edition {} {} release gate passed: {}/{} evidence gates, {} conformance cases",
                audit.edition,
                audit.release_track,
                audit.evidence_passed,
                audit.evidence_required,
                audit.conformance_cases
            );
            ExitCode::SUCCESS
        }
        Ok(audit) => {
            eprintln!(
                "Edition {} {} release gate blocked:",
                audit.edition, audit.release_track
            );
            for blocker in audit.blockers {
                eprintln!("- {blocker}");
            }
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("error: {}", error.message);
            ExitCode::from(65)
        }
    }
}

fn check_file(path: &str) -> ExitCode {
    if is_project_path(Path::new(path)) {
        return check_project(path, false);
    }
    if is_bundle_path(Path::new(path)) {
        return match fs::read(path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| nivren::bundle::decode(&bytes).map_err(|error| error.message))
        {
            Ok(_) => {
                println!("{}: ok", Path::new(path).display());
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{path}: error: {error}");
                ExitCode::from(65)
            }
        };
    }
    let source = match read_source(path) {
        Ok(source) => source,
        Err(code) => return code,
    };
    match compile_file(Path::new(path)) {
        Ok(_) => {
            println!("{}: ok", Path::new(path).display());
            ExitCode::SUCCESS
        }
        Err(errors) => {
            report(path, &source, &errors);
            ExitCode::from(65)
        }
    }
}

fn is_project_path(path: &Path) -> bool {
    path.is_dir() || path.file_name().is_some_and(|name| name == "niv.toml")
}

fn is_bundle_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "nivb")
}

fn build_project(path: &str) -> ExitCode {
    check_project(path, true)
}

fn build_standalone(path: &str) -> ExitCode {
    build_standalone_engine(path, false)
}

fn build_native_standalone(path: &str) -> ExitCode {
    build_standalone_engine(path, true)
}

fn build_standalone_engine(path: &str, native: bool) -> ExitCode {
    let (manifest, chunk) = match compile_project(Path::new(path)) {
        Ok(result) => result,
        Err(errors) => {
            report(path, "", &errors);
            return ExitCode::from(65);
        }
    };
    let bundle = match nivren::bundle::encode(&chunk) {
        Ok(bundle) => bundle,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(70);
        }
    };
    let current = match env::current_exe().and_then(fs::read) {
        Ok(executable) => executable,
        Err(error) => {
            eprintln!("error: cannot read the Nivren executable: {error}");
            return ExitCode::from(74);
        }
    };
    let manifest_source = if native {
        format!("# nivren-standalone-engine = native\n{}", manifest.source())
    } else {
        manifest.source()
    };
    let application = match nivren::standalone::attach(&current, &bundle, &manifest_source) {
        Ok(application) => application,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(70);
        }
    };
    let target = manifest.root.join("target");
    if let Err(error) = fs::create_dir_all(&target) {
        eprintln!("error: cannot create {}: {error}", target.display());
        return ExitCode::from(73);
    }
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let output = target.join(format!("{}{suffix}", manifest.name));
    if let Err(error) = write_atomic(&output, &application) {
        eprintln!("error: cannot write {}: {error}", output.display());
        return ExitCode::from(73);
    }
    #[cfg(unix)]
    {
        let permissions = env::current_exe()
            .and_then(fs::metadata)
            .map(|metadata| metadata.permissions());
        if let Ok(permissions) = permissions
            && let Err(error) = fs::set_permissions(&output, permissions)
        {
            eprintln!(
                "error: cannot make {} executable: {error}",
                output.display()
            );
            return ExitCode::from(73);
        }
    }
    println!("standalone {}", output.display());
    ExitCode::SUCCESS
}

fn build_aot(path: &str) -> ExitCode {
    let manifest = match nivren::project::Manifest::load(Path::new(path)) {
        Ok(manifest) => manifest,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(65);
        }
    };
    if let Err(error) = verify_dependency_lock(&manifest) {
        report(path, "", &[error]);
        return ExitCode::from(65);
    }
    let program = match nivren::modules::load_project(&manifest.root, &manifest.entry_path()) {
        Ok(program) => program,
        Err(errors) => {
            report(path, "", &errors);
            return ExitCode::from(65);
        }
    };
    if let Err(errors) = nivren::typecheck::check(&program) {
        report(path, "", &errors);
        return ExitCode::from(65);
    }
    let chunk = match nivren::bytecode::compile(&program) {
        Ok(chunk) => chunk,
        Err(errors) => {
            report(path, "", &errors);
            return ExitCode::from(65);
        }
    };
    let eligible = program
        .iter()
        .filter_map(|statement| match statement {
            nivren::ast::Stmt::Function {
                name,
                type_params,
                params,
                return_type,
                needs,
                ..
            } if type_params.is_empty()
                && needs.is_empty()
                && params.iter().all(|parameter| {
                    matches!(parameter.ty, Some(nivren::ast::TypeRef::Named(ref name, _)) if name == "Int")
                })
                && matches!(return_type, Some(nivren::ast::TypeRef::Named(name, _)) if name == "Int") =>
            {
                Some(name.clone())
            }
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    let directory = manifest.root.join("target/aot");
    if let Err(error) = fs::create_dir_all(&directory) {
        eprintln!("error: cannot create {}: {error}", directory.display());
        return ExitCode::from(73);
    }
    let extension = if cfg!(windows) { "obj" } else { "o" };
    let program_object = directory.join(format!("program.{extension}"));
    let program_bytes = match nivren_jit::TraceObject::compile("nivren_program", chunk.code.len()) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("error: cannot compile complete program ahead of time: {error}");
            return ExitCode::from(70);
        }
    };
    if let Err(error) = write_atomic(&program_object, &program_bytes) {
        eprintln!("error: cannot write {}: {error}", program_object.display());
        return ExitCode::from(73);
    }
    let bundle = match nivren::bundle::encode(&chunk) {
        Ok(bundle) => bundle,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(70);
        }
    };
    let program_bundle = directory.join("program.nivb");
    if let Err(error) = write_atomic(&program_bundle, &bundle) {
        eprintln!("error: cannot write {}: {error}", program_bundle.display());
        return ExitCode::from(73);
    }
    let header = directory.join("nivren_program.h");
    let header_bytes = b"#ifndef NIVREN_PROGRAM_H\n#define NIVREN_PROGRAM_H\n#include <stdint.h>\n#ifdef __cplusplus\nextern \"C\" {\n#endif\ntypedef int64_t (*NivrenTraceCallback)(void *context, uint64_t instruction);\nint64_t nivren_program(void *context, NivrenTraceCallback callback);\n#ifdef __cplusplus\n}\n#endif\n#endif\n";
    if let Err(error) = write_atomic(&header, header_bytes) {
        eprintln!("error: cannot write {}: {error}", header.display());
        return ExitCode::from(73);
    }
    let metadata = format!(
        "{{\n  \"abi\": \"nivren-trace-v1\",\n  \"bytecode\": \"program.nivb\",\n  \"entry\": \"nivren_program\",\n  \"instructions\": {},\n  \"object\": \"program.{}\"\n}}\n",
        chunk.code.len(),
        extension
    );
    let metadata_path = directory.join("program.json");
    if let Err(error) = write_atomic(&metadata_path, metadata.as_bytes()) {
        eprintln!("error: cannot write {}: {error}", metadata_path.display());
        return ExitCode::from(73);
    }
    println!("aot {}", program_object.display());
    println!("aot {}", program_bundle.display());
    let mut emitted = 0usize;
    for instruction in &chunk.code {
        let nivren::bytecode::Op::MakeFunction { name, params, body } = &instruction.op else {
            continue;
        };
        if !eligible.contains(name) {
            continue;
        }
        let Some((slots, operations)) = nivren::bytecode::integer_native_plan(params, body) else {
            continue;
        };
        let symbol = format!("nivren_{name}");
        let bytes = match nivren_jit::AotObject::compile(&symbol, params.len(), slots, &operations)
        {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("error: cannot compile '{name}' ahead of time: {error}");
                return ExitCode::from(70);
            }
        };
        let output = directory.join(format!("{name}.{extension}"));
        if let Err(error) = write_atomic(&output, &bytes) {
            eprintln!("error: cannot write {}: {error}", output.display());
            return ExitCode::from(73);
        }
        println!("aot {}", output.display());
        emitted += 1;
    }
    println!("aot optimized-kernels {emitted}");
    // When the whole top-level chunk lowers to native integer code, also emit
    // it as one relocatable object: the exported root plus every planned
    // function calling one another directly, no runtime callback needed.
    if let Some(plan) = nivren::bytecode::integer_program_plan(&chunk) {
        let functions = plan
            .functions
            .iter()
            .map(|function| nivren_jit::PlanFunction {
                parameters: function.parameters,
                slots: function.slots,
                operations: function.operations.clone(),
            })
            .collect::<Vec<_>>();
        let root = nivren_jit::PlanRoot {
            slots: plan.root_slots,
            operations: plan.root_operations.clone(),
        };
        match nivren_jit::AotObject::compile_program(
            "nivren_program_native",
            &functions,
            &root,
            256,
        ) {
            Ok(bytes) => {
                let output = directory.join(format!("program_native.{extension}"));
                if let Err(error) = write_atomic(&output, &bytes) {
                    eprintln!("error: cannot write {}: {error}", output.display());
                    return ExitCode::from(73);
                }
                println!("aot {}", output.display());
                println!("aot native-program 1");
            }
            Err(error) => {
                eprintln!("error: cannot compile the native program ahead of time: {error}");
                return ExitCode::from(70);
            }
        }
    } else {
        println!("aot native-program 0");
    }
    ExitCode::SUCCESS
}

fn new_project(path: &str) -> ExitCode {
    let root = Path::new(path);
    if root.exists() {
        eprintln!("error: {} already exists", root.display());
        return ExitCode::from(73);
    }
    let Some(name) = root.file_name().and_then(|name| name.to_str()) else {
        eprintln!("error: project path needs a valid final name");
        return ExitCode::from(64);
    };
    let manifest_source = format!(
        "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nentry = \"src/main.niv\"\n",
        name.to_ascii_lowercase().replace(' ', "-")
    );
    if let Err(error) = nivren::project::Manifest::parse(&manifest_source, root.to_path_buf()) {
        report(path, "", &[error]);
        return ExitCode::from(64);
    }
    let result = (|| -> io::Result<()> {
        fs::create_dir_all(root.join("src"))?;
        fs::create_dir_all(root.join("tests/niv"))?;
        fs::write(root.join("niv.toml"), manifest_source)?;
        fs::write(
            root.join("src/main.niv"),
            "define main\ngives Nothing\n{\n    show(\"Welcome to Nivren\")\n    give none\n}\n\nmain with {}\n",
        )?;
        fs::write(
            root.join("tests/niv/main_test.niv"),
            "define answer\ngives Int\n{\n    give 42\n}\n\nassert(answer with {} == 42, \"answer\")\n",
        )?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            println!("created {}", root.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: could not create {}: {error}", root.display());
            ExitCode::from(73)
        }
    }
}

fn add_dependency(path: &str, name: &str, version: &str) -> ExitCode {
    let mut manifest = match nivren::project::Manifest::load(Path::new(path)) {
        Ok(manifest) => manifest,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(65);
        }
    };
    if let Err(error) = manifest.add_dependency(name, version) {
        report(path, "", &[error]);
        return ExitCode::from(64);
    }
    let manifest_path = manifest.root.join(nivren::project::MANIFEST_NAME);
    if let Err(error) = write_atomic(&manifest_path, manifest.source().as_bytes()) {
        eprintln!("error: cannot update {}: {error}", manifest_path.display());
        return ExitCode::from(73);
    }
    println!("added {name} {version}; run 'niv install <registry>' to fetch dependencies");
    ExitCode::SUCCESS
}

fn ship_project(path: &str) -> ExitCode {
    let checked = check_project(path, true);
    if checked != ExitCode::SUCCESS {
        return checked;
    }
    let manifest = match nivren::project::Manifest::load(Path::new(path)) {
        Ok(manifest) => manifest,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(65);
        }
    };
    let tests = manifest.root.join("tests/niv");
    if tests.exists() {
        let tested = test_path(&tests.display().to_string(), None, None);
        if tested != ExitCode::SUCCESS {
            return tested;
        }
    }
    let documented = document_project(path);
    if documented != ExitCode::SUCCESS {
        return documented;
    }
    let packaged = package_project(path);
    if packaged != ExitCode::SUCCESS {
        return packaged;
    }
    build_standalone(path)
}

fn workspace_action(action: &str, path: &str) -> ExitCode {
    let workspace = match nivren::workspace::Workspace::load(Path::new(path)) {
        Ok(workspace) => workspace,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(65);
        }
    };
    if !matches!(action, "check" | "build" | "test" | "bench" | "ship") {
        eprintln!("error: workspace action must be check, build, test, bench, or ship");
        return ExitCode::from(64);
    }
    for member in &workspace.members {
        let member_path = member.root.display().to_string();
        println!("workspace {action}: {}", member.name);
        let result = match action {
            "check" => check_project(&member_path, false),
            "build" => build_project(&member_path),
            "test" => {
                let tests = member.root.join("tests/niv");
                if tests.exists() {
                    test_path(&tests.display().to_string(), None, None)
                } else {
                    check_project(&member_path, false)
                }
            }
            "bench" => benchmark_path(&member_path, None),
            "ship" => ship_project(&member_path),
            _ => unreachable!(),
        };
        if result != ExitCode::SUCCESS {
            return result;
        }
    }
    println!(
        "workspace {action}: {} member(s) passed",
        workspace.members.len()
    );
    ExitCode::SUCCESS
}

fn package_project(path: &str) -> ExitCode {
    let (manifest, _) = match compile_project(Path::new(path)) {
        Ok(result) => result,
        Err(errors) => {
            report(path, "", &errors);
            return ExitCode::from(65);
        }
    };
    let package = match nivren::package::Package::build(&manifest) {
        Ok(package) => package,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(65);
        }
    };
    let bytes = match package.encode() {
        Ok(bytes) => bytes,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(65);
        }
    };
    let target = manifest.root.join("target");
    if let Err(error) = fs::create_dir_all(&target) {
        eprintln!("error: cannot create {}: {error}", target.display());
        return ExitCode::from(73);
    }
    let artifact = target.join(format!("{}-{}.nivpkg", manifest.name, manifest.version));
    if let Err(error) = write_atomic(&artifact, &bytes) {
        eprintln!("error: cannot write {}: {error}", artifact.display());
        return ExitCode::from(73);
    }
    println!("packaged {}", artifact.display());
    ExitCode::SUCCESS
}

fn verify_package(path: &str) -> ExitCode {
    match fs::read(path)
        .map_err(|error| NivError::new(error.to_string(), 1, 1))
        .and_then(|bytes| nivren::package::Package::decode(&bytes))
    {
        Ok(package) => {
            println!("{} {}: verified", package.name, package.version);
            ExitCode::SUCCESS
        }
        Err(error) => {
            report(path, "", &[error]);
            ExitCode::from(65)
        }
    }
}

fn registry_publish(package_path: &str, registry_path: &str) -> ExitCode {
    let bytes = match fs::read(package_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("error: cannot read {package_path}: {error}");
            return ExitCode::from(66);
        }
    };
    match nivren::package::publish(&bytes, Path::new(registry_path)) {
        Ok(path) => {
            println!("published {}", path.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            report(package_path, "", &[error]);
            ExitCode::from(65)
        }
    }
}

fn registry_search(query: &str, registry_path: &str) -> ExitCode {
    match nivren::package::search(query, Path::new(registry_path)) {
        Ok(results) if results.is_empty() => {
            println!("no packages found");
            ExitCode::SUCCESS
        }
        Ok(results) => {
            for result in results {
                println!("{} {}", result.name, result.versions.join(", "));
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            report(registry_path, "", &[error]);
            ExitCode::from(65)
        }
    }
}

fn registry_set_yanked(name: &str, version: &str, registry: &str, yanked: bool) -> ExitCode {
    match nivren::package::set_yanked(name, version, Path::new(registry), yanked) {
        Ok(()) => {
            println!(
                "{} {name} {version}",
                if yanked { "yanked" } else { "unyanked" }
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            report(registry, "", &[error]);
            ExitCode::from(65)
        }
    }
}

fn registry_fetch(name: &str, version: &str, registry_path: &str, destination: &str) -> ExitCode {
    let package = nivren::package::fetch(name, version, Path::new(registry_path))
        .and_then(|bytes| nivren::package::Package::decode(&bytes));
    match package.and_then(|package| package.extract(Path::new(destination))) {
        Ok(()) => {
            println!("fetched {name} {version} to {destination}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            report(registry_path, "", &[error]);
            ExitCode::from(65)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn registry_verify_release(
    package_path: &str,
    provenance_path: &str,
    authorization_path: &str,
    status_path: &str,
    advisories_path: &str,
    root_path: &str,
    now: &str,
    minimum: &str,
) -> ExitCode {
    let result = (|| -> Result<nivren::package::Package, NivError> {
        let read = |path: &str| {
            fs::read(path)
                .map_err(|error| NivError::new(format!("cannot read {path}: {error}"), 1, 1))
        };
        let package = read(package_path)?;
        let provenance =
            serde_json::from_slice::<nivren::trust::ReleaseProvenance>(&read(provenance_path)?)
                .map_err(|error| NivError::new(format!("invalid provenance: {error}"), 1, 1))?;
        let authorization = serde_json::from_slice::<nivren::trust::PublisherAuthorization>(&read(
            authorization_path,
        )?)
        .map_err(|error| NivError::new(format!("invalid authorization: {error}"), 1, 1))?;
        let status = serde_json::from_slice::<nivren::trust::RegistryStatus>(&read(status_path)?)
            .map_err(|error| {
            NivError::new(format!("invalid registry status: {error}"), 1, 1)
        })?;
        let advisories =
            serde_json::from_slice::<Vec<nivren::trust::Advisory>>(&read(advisories_path)?)
                .map_err(|error| NivError::new(format!("invalid advisories: {error}"), 1, 1))?;
        let root = String::from_utf8(read(root_path)?)
            .map_err(|_| NivError::new("root public key is not UTF-8", 1, 1))?;
        let root = nivren::trust::parse_public_key(&root)?;
        let now = now
            .parse::<u64>()
            .map_err(|_| NivError::new("time must be Unix seconds", 1, 1))?;
        let minimum = minimum
            .parse::<u64>()
            .map_err(|_| NivError::new("minimum generation must be an integer", 1, 1))?;
        nivren::trust::verify_release(
            &package,
            &provenance,
            &authorization,
            &status,
            &advisories,
            root,
            now,
            minimum,
        )
    })();
    match result {
        Ok(package) => {
            println!(
                "{} {}: trusted release verified",
                package.name, package.version
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            report(package_path, "", &[error]);
            ExitCode::from(65)
        }
    }
}

fn registry_serve(registry: &str, bind: &str, minimum: &str) -> ExitCode {
    let bind = match bind.parse() {
        Ok(bind) => bind,
        Err(error) => {
            eprintln!("error: invalid registry bind address: {error}");
            return ExitCode::from(64);
        }
    };
    let minimum = match minimum.parse() {
        Ok(minimum) => minimum,
        Err(error) => {
            eprintln!("error: invalid minimum status generation: {error}");
            return ExitCode::from(64);
        }
    };
    eprintln!(
        "Nivren registry serving {} on {bind}",
        Path::new(registry).display()
    );
    match nivren::registry_server::serve(nivren::registry_server::ServerConfig {
        registry: PathBuf::from(registry),
        bind,
        workers: 16,
        queue: 128,
        minimum_status_generation: minimum,
    }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            report(registry, "", &[error]);
            ExitCode::from(70)
        }
    }
}

fn registry_envelope(
    package_path: &str,
    provenance_path: &str,
    authorization_path: &str,
    output_path: &str,
) -> ExitCode {
    let result = (|| -> Result<Vec<u8>, NivError> {
        let package = fs::read(package_path)
            .map_err(|error| NivError::new(format!("cannot read package: {error}"), 1, 1))?;
        let provenance = serde_json::from_slice::<nivren::trust::ReleaseProvenance>(
            &fs::read(provenance_path)
                .map_err(|error| NivError::new(format!("cannot read provenance: {error}"), 1, 1))?,
        )
        .map_err(|error| NivError::new(format!("invalid provenance: {error}"), 1, 1))?;
        let authorization = serde_json::from_slice::<nivren::trust::PublisherAuthorization>(
            &fs::read(authorization_path).map_err(|error| {
                NivError::new(format!("cannot read authorization: {error}"), 1, 1)
            })?,
        )
        .map_err(|error| NivError::new(format!("invalid authorization: {error}"), 1, 1))?;
        nivren::trust::PublishEnvelope {
            package,
            provenance,
            authorization,
        }
        .encode()
    })();
    match result {
        Ok(bytes) => match write_atomic(Path::new(output_path), &bytes) {
            Ok(()) => {
                println!("created {output_path}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: cannot write {output_path}: {error}");
                ExitCode::from(73)
            }
        },
        Err(error) => {
            report(package_path, "", &[error]);
            ExitCode::from(65)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn registry_sign_admin(
    operation: &str,
    name: &str,
    version: &str,
    generation: &str,
    issued: &str,
    expires: &str,
    reason_path: &str,
    secret_path: &str,
    output: &str,
) -> ExitCode {
    let result = (|| -> Result<(), String> {
        let generation = generation
            .parse::<u64>()
            .map_err(|_| "invalid admin generation".to_string())?;
        let issued_at = issued
            .parse::<u64>()
            .map_err(|_| "invalid admin issue time".to_string())?;
        let expires_at = expires
            .parse::<u64>()
            .map_err(|_| "invalid admin expiry time".to_string())?;
        let reason = fs::read_to_string(reason_path)
            .map_err(|error| format!("cannot read admin reason: {error}"))?;
        let secret = fs::read_to_string(secret_path)
            .map_err(|error| format!("cannot read registry root secret: {error}"))?;
        let secret = nivren::trust::parse_secret_key(&secret).map_err(|error| error.message)?;
        let action = nivren::trust::sign_admin_action(
            secret,
            nivren::trust::RegistryAdminAction {
                format: 1,
                action: operation.into(),
                package: name.into(),
                version: version.into(),
                generation,
                issued_at,
                expires_at,
                reason: reason.trim().into(),
                signature: String::new(),
            },
        )
        .map_err(|error| error.message)?;
        let encoded = serde_json::to_vec_pretty(&action)
            .map_err(|error| format!("cannot encode admin action: {error}"))?;
        write_atomic(Path::new(output), &encoded).map_err(|error| error.to_string())
    })();
    match result {
        Ok(()) => {
            println!("signed {output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(65)
        }
    }
}

fn registry_verify_admin(document: &str, public: &str, now: &str, minimum: &str) -> ExitCode {
    let result = (|| -> Result<nivren::trust::RegistryAdminAction, String> {
        let action = serde_json::from_slice::<nivren::trust::RegistryAdminAction>(
            &fs::read(document).map_err(|error| format!("cannot read admin action: {error}"))?,
        )
        .map_err(|error| format!("invalid admin action: {error}"))?;
        let public = fs::read_to_string(public)
            .map_err(|error| format!("cannot read registry root public key: {error}"))?;
        let public = nivren::trust::parse_public_key(&public).map_err(|error| error.message)?;
        let now = now
            .parse::<u64>()
            .map_err(|_| "invalid Unix time".to_string())?;
        let minimum = minimum
            .parse::<u64>()
            .map_err(|_| "invalid minimum generation".to_string())?;
        nivren::trust::verify_admin_action(&action, public, now, minimum)
            .map_err(|error| error.message)?;
        Ok(action)
    })();
    match result {
        Ok(action) => {
            println!(
                "verified registry {} for {} {} generation {}",
                action.action, action.package, action.version, action.generation
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(65)
        }
    }
}

fn registry_recover_admin(registry: &str, now: &str, minimum: &str) -> ExitCode {
    let result = now
        .parse::<u64>()
        .map_err(|_| NivError::new("invalid Unix time", 1, 1))
        .and_then(|now| {
            minimum
                .parse::<u64>()
                .map_err(|_| NivError::new("invalid minimum generation", 1, 1))
                .map(|minimum| (now, minimum))
        })
        .and_then(|(now, minimum)| {
            nivren::registry_server::recover_admin(Path::new(registry), now, minimum)
        });
    match result {
        Ok(generation) => {
            println!("recovered registry admin generation {generation}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {}", error.message);
            ExitCode::from(65)
        }
    }
}

fn disassemble_path(path: &str) -> ExitCode {
    let chunk = if is_bundle_path(Path::new(path)) {
        match fs::read(path)
            .map_err(|error| NivError::new(error.to_string(), 1, 1))
            .and_then(|bytes| nivren::bundle::decode(&bytes))
        {
            Ok(chunk) => chunk,
            Err(error) => {
                report(path, "", &[error]);
                return ExitCode::from(65);
            }
        }
    } else if is_project_path(Path::new(path)) {
        match compile_project(Path::new(path)) {
            Ok((_, chunk)) => chunk,
            Err(errors) => {
                report(path, "", &errors);
                return ExitCode::from(65);
            }
        }
    } else {
        match compile_file(Path::new(path)) {
            Ok(chunk) => chunk,
            Err(errors) => {
                let source = fs::read_to_string(path).unwrap_or_default();
                report(path, &source, &errors);
                return ExitCode::from(65);
            }
        }
    };
    print!("{}", nivren::bytecode::disassemble(&chunk));
    ExitCode::SUCCESS
}

fn source_map_path(path: &str, output: &str) -> ExitCode {
    let chunk = if is_bundle_path(Path::new(path)) {
        match fs::read(path)
            .map_err(|error| NivError::new(error.to_string(), 1, 1))
            .and_then(|bytes| nivren::bundle::decode(&bytes))
        {
            Ok(chunk) => chunk,
            Err(error) => {
                report(path, "", &[error]);
                return ExitCode::from(65);
            }
        }
    } else if is_project_path(Path::new(path)) {
        match compile_project(Path::new(path)) {
            Ok((_, chunk)) => chunk,
            Err(errors) => {
                report(path, "", &errors);
                return ExitCode::from(65);
            }
        }
    } else {
        match compile_file(Path::new(path)) {
            Ok(chunk) => chunk,
            Err(errors) => {
                report(path, "", &errors);
                return ExitCode::from(65);
            }
        }
    };
    let map = nivren::bytecode::source_map(&chunk, path);
    match write_atomic(Path::new(output), map.as_bytes()) {
        Ok(()) => {
            println!("source map {output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: cannot write {output}: {error}");
            ExitCode::from(73)
        }
    }
}

fn observe_path(path: &str, coverage: bool, output: Option<&str>) -> ExitCode {
    let (manifest, chunk) = if is_bundle_path(Path::new(path)) {
        match fs::read(path)
            .map_err(|error| NivError::new(error.to_string(), 1, 1))
            .and_then(|bytes| nivren::bundle::decode(&bytes))
        {
            Ok(chunk) => (None, chunk),
            Err(error) => {
                report(path, "", &[error]);
                return ExitCode::from(65);
            }
        }
    } else if is_project_path(Path::new(path)) {
        match compile_project(Path::new(path)) {
            Ok((manifest, chunk)) => (Some(manifest), chunk),
            Err(errors) => {
                report(path, "", &errors);
                return ExitCode::from(65);
            }
        }
    } else {
        match compile_file(Path::new(path)) {
            Ok(chunk) => (None, chunk),
            Err(errors) => {
                let source = fs::read_to_string(path).unwrap_or_default();
                report(path, &source, &errors);
                return ExitCode::from(65);
            }
        }
    };
    let mut interpreter = manifest
        .as_ref()
        .map_or_else(Interpreter::new, project_interpreter);
    interpreter.enable_metrics();
    let started = Instant::now();
    if let Err(error) = interpreter.run_bytecode(&chunk) {
        let source = fs::read_to_string(path).unwrap_or_default();
        report(path, &source, &[error]);
        return ExitCode::from(70);
    }
    let elapsed = started.elapsed();
    let metrics = interpreter.execution_metrics().unwrap_or_default();
    let jit = interpreter.jit_stats();
    let native = interpreter.native_stats();
    let heap = interpreter.heap_stats();
    if let Some(output) = output {
        let executable = if coverage {
            let mut lines = BTreeSet::new();
            executable_lines(&chunk, &mut lines);
            lines
        } else {
            BTreeSet::new()
        };
        let missed = executable
            .iter()
            .filter(|line| !metrics.line_hits.contains_key(line))
            .copied()
            .collect::<Vec<_>>();
        let report = serde_json::json!({
            "schema": "org.nivren.observation.v1",
            "kind": if coverage { "coverage" } else { "profile" },
            "duration_nanoseconds": u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX),
            "instructions": metrics.instructions,
            "line_hits": metrics.line_hits,
            "operation_hits": metrics.operation_hits,
            "jit": { "compilations": jit.compilations, "executions": jit.executions },
            "execution": {
                "instructions": metrics.instructions,
                "operation_hits": metrics.operation_hits,
                "line_hits": metrics.line_hits,
            },
            "memory": {
                "allocation_work_bytes": metrics.allocation_work_bytes,
                "plan_allocations": metrics.plan_allocations,
                "heap": {
                    "tracked_environments": heap.tracked_environments,
                    "live_environments": heap.live_environments,
                    "collections": heap.collections,
                    "minor_collections": heap.minor_collections,
                    "major_collections": heap.major_collections,
                    "concurrent_marking": heap.concurrent_marking,
                },
            },
            "effects": {
                "perform_boundaries": metrics.perform_boundaries,
                "count": metrics.effect_sequence.len(),
                "sequence": metrics.effect_sequence,
            },
            "async_tasks": {
                "spawns": metrics.task_spawns,
                "blocking_submissions": metrics.blocking_task_submissions,
                "joins": metrics.task_joins,
                "cancellations": metrics.task_cancellations,
                "event_loop_waits": metrics.event_loop_waits,
            },
            "engines": {
                "jit": { "compilations": jit.compilations, "executions": jit.executions },
                "native": {
                    "compilations": native.compilations,
                    "executions": native.executions,
                    "fallbacks": native.fallbacks,
                },
            },
            "coverage": if coverage { Some(serde_json::json!({
                "executable": executable.len(),
                "hit": executable.len().saturating_sub(missed.len()),
                "missed_lines": missed,
            })) } else { None },
        });
        let bytes = serde_json::to_vec_pretty(&report).expect("observation JSON is representable");
        return match write_atomic(Path::new(output), &bytes) {
            Ok(()) => {
                println!("wrote {output}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: cannot write {output}: {error}");
                ExitCode::from(73)
            }
        };
    }
    if coverage {
        let mut executable = BTreeSet::new();
        executable_lines(&chunk, &mut executable);
        let missed = executable
            .iter()
            .filter(|line| !metrics.line_hits.contains_key(line))
            .copied()
            .collect::<Vec<_>>();
        let hit = executable.len().saturating_sub(missed.len());
        let percentage = if executable.is_empty() {
            100.0
        } else {
            hit as f64 * 100.0 / executable.len() as f64
        };
        println!(
            "coverage: {hit}/{} lines ({percentage:.1}%)",
            executable.len()
        );
        if !missed.is_empty() {
            println!(
                "missed: {}",
                missed
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    } else {
        println!(
            "profile: {} instructions in {:.3} ms",
            metrics.instructions,
            elapsed.as_secs_f64() * 1000.0
        );
        for (operation, hits) in metrics.operation_hits {
            println!("  {operation}: {hits}");
        }
        println!(
            "  native tier: {} compilation(s), {} execution(s)",
            jit.compilations, jit.executions
        );
        println!(
            "  memory: {} allocation-work byte(s), {} plan allocation(s), {} live environment(s)",
            metrics.allocation_work_bytes, metrics.plan_allocations, heap.live_environments
        );
        println!(
            "  effects: {} perform boundary/boundaries, {} observed effect(s)",
            metrics.perform_boundaries,
            metrics.effect_sequence.len()
        );
        println!(
            "  async: {} spawn(s), {} blocking submission(s), {} join(s), {} cancellation(s), {} event-loop wait(s)",
            metrics.task_spawns,
            metrics.blocking_task_submissions,
            metrics.task_joins,
            metrics.task_cancellations,
            metrics.event_loop_waits
        );
    }
    ExitCode::SUCCESS
}

fn benchmark_path(path: &str, output: Option<&str>) -> ExitCode {
    const WARMUPS: usize = 2;
    const SAMPLES: usize = 15;
    let (manifest, chunk) = if is_bundle_path(Path::new(path)) {
        match fs::read(path)
            .map_err(|error| NivError::new(error.to_string(), 1, 1))
            .and_then(|bytes| nivren::bundle::decode(&bytes))
        {
            Ok(chunk) => (None, chunk),
            Err(error) => {
                report(path, "", &[error]);
                return ExitCode::from(65);
            }
        }
    } else if is_project_path(Path::new(path)) {
        match compile_project(Path::new(path)) {
            Ok((manifest, chunk)) => (Some(manifest), chunk),
            Err(errors) => {
                report(path, "", &errors);
                return ExitCode::from(65);
            }
        }
    } else {
        match compile_file(Path::new(path)) {
            Ok(chunk) => (None, chunk),
            Err(errors) => {
                let source = fs::read_to_string(path).unwrap_or_default();
                report(path, &source, &errors);
                return ExitCode::from(65);
            }
        }
    };

    for _ in 0..WARMUPS {
        let mut interpreter = manifest
            .as_ref()
            .map_or_else(Interpreter::new, project_interpreter);
        if let Err(error) = interpreter.run_bytecode(&chunk) {
            report(path, "", &[error]);
            return ExitCode::from(70);
        }
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let mut interpreter = manifest
            .as_ref()
            .map_or_else(Interpreter::new, project_interpreter);
        let started = Instant::now();
        if let Err(error) = interpreter.run_bytecode(&chunk) {
            report(path, "", &[error]);
            return ExitCode::from(70);
        }
        samples.push(u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX));
    }
    samples.sort_unstable();
    let minimum = samples[0];
    let median = samples[SAMPLES / 2];
    let p95 = samples[(SAMPLES * 95).div_ceil(100) - 1];
    let report = serde_json::json!({
        "schema": "org.nivren.benchmark.v1",
        "path": path,
        "engine": "vm",
        "warmups": WARMUPS,
        "samples": SAMPLES,
        "minimum_nanoseconds": minimum,
        "median_nanoseconds": median,
        "p95_nanoseconds": p95,
    });
    if let Some(output) = output {
        let bytes = serde_json::to_vec_pretty(&report).expect("benchmark JSON is representable");
        return match write_atomic(Path::new(output), &bytes) {
            Ok(()) => {
                println!("wrote {output}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: cannot write {output}: {error}");
                ExitCode::from(73)
            }
        };
    }
    println!(
        "bench {path}: median {:.3} ms, p95 {:.3} ms, minimum {:.3} ms ({SAMPLES} samples)",
        median as f64 / 1_000_000.0,
        p95 as f64 / 1_000_000.0,
        minimum as f64 / 1_000_000.0,
    );
    ExitCode::SUCCESS
}

fn run_with_crash_report(path: &str, output: &str) -> ExitCode {
    let (manifest, chunk) = if is_bundle_path(Path::new(path)) {
        match fs::read(path)
            .map_err(|error| NivError::new(error.to_string(), 1, 1))
            .and_then(|bytes| nivren::bundle::decode(&bytes))
        {
            Ok(chunk) => (None, chunk),
            Err(error) => {
                report(path, "", &[error]);
                return ExitCode::from(65);
            }
        }
    } else if is_project_path(Path::new(path)) {
        match compile_project(Path::new(path)) {
            Ok((manifest, chunk)) => (Some(manifest), chunk),
            Err(errors) => {
                report(path, "", &errors);
                return ExitCode::from(65);
            }
        }
    } else {
        match compile_file(Path::new(path)) {
            Ok(chunk) => (None, chunk),
            Err(errors) => {
                let source = fs::read_to_string(path).unwrap_or_default();
                report(path, &source, &errors);
                return ExitCode::from(65);
            }
        }
    };
    let result = manifest
        .as_ref()
        .map_or_else(Interpreter::new, project_interpreter)
        .run_bytecode(&chunk);
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            let source = fs::read_to_string(path).unwrap_or_default();
            report(path, &source, std::slice::from_ref(&error));
            let trace = error
                .trace
                .iter()
                .map(|frame| {
                    serde_json::json!({
                        "function": frame.function,
                        "line": frame.line,
                        "column": frame.column,
                    })
                })
                .collect::<Vec<_>>();
            let report = serde_json::json!({
                "schema": "org.nivren.crash.v1",
                "runtime": nivren::VERSION,
                "program": Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or("program"),
                "error": {
                    "message": error.message,
                    "line": error.line,
                    "column": error.column,
                    "trace": trace,
                },
                "privacy": "source, arguments, environment, and local values omitted",
            });
            let bytes = serde_json::to_vec_pretty(&report).expect("crash JSON is representable");
            match write_atomic(Path::new(output), &bytes) {
                Ok(()) => eprintln!("crash report: {output}"),
                Err(write_error) => eprintln!("error: cannot write crash report: {write_error}"),
            }
            ExitCode::from(70)
        }
    }
}

fn debug_path(path: &str) -> ExitCode {
    let (manifest, chunk) = if is_bundle_path(Path::new(path)) {
        match fs::read(path)
            .map_err(|error| NivError::new(error.to_string(), 1, 1))
            .and_then(|bytes| nivren::bundle::decode(&bytes))
        {
            Ok(chunk) => (None, chunk),
            Err(error) => {
                report(path, "", &[error]);
                return ExitCode::from(65);
            }
        }
    } else if is_project_path(Path::new(path)) {
        match compile_project(Path::new(path)) {
            Ok((manifest, chunk)) => (Some(manifest), chunk),
            Err(errors) => {
                report(path, "", &errors);
                return ExitCode::from(65);
            }
        }
    } else {
        match compile_file(Path::new(path)) {
            Ok(chunk) => (None, chunk),
            Err(errors) => {
                let source = fs::read_to_string(path).unwrap_or_default();
                report(path, &source, &errors);
                return ExitCode::from(65);
            }
        }
    };
    let display_path = path.to_string();
    let source = fs::read_to_string(path).unwrap_or_default();
    let source_lines = source.lines().map(str::to_string).collect::<Vec<_>>();
    let mut breakpoints = BTreeSet::new();
    let mut stepping = true;
    let mut previous_line = None;
    let mut first = true;
    let mut interpreter = manifest
        .as_ref()
        .map_or_else(Interpreter::new, project_interpreter);
    eprintln!("Nivren debugger — type 'help' for commands");
    interpreter.set_debug_hook(move |event| {
        let changed_line = previous_line != Some(event.line);
        let should_stop =
            first || (changed_line && (stepping || breakpoints.contains(&event.line)));
        first = false;
        previous_line = Some(event.line);
        if !should_stop {
            return nivren::runtime::DebugControl::Continue;
        }
        eprintln!(
            "\n{}:{}:{}  {}  [instruction {}, stack {}]",
            display_path,
            event.line,
            event.column,
            event.operation,
            event.instruction,
            event.stack_depth
        );
        if let Some(line) = source_lines.get(event.line.saturating_sub(1)) {
            eprintln!("{line}");
        }
        loop {
            eprint!("(niv) ");
            let _ = io::stderr().flush();
            let mut command = String::new();
            if io::stdin().read_line(&mut command).unwrap_or(0) == 0 {
                return nivren::runtime::DebugControl::Terminate;
            }
            let mut words = command.split_whitespace();
            match words.next().unwrap_or("") {
                "n" | "next" | "s" | "step" => {
                    stepping = true;
                    return nivren::runtime::DebugControl::Continue;
                }
                "c" | "continue" => {
                    stepping = false;
                    return nivren::runtime::DebugControl::Continue;
                }
                "b" | "break" => match words.next().and_then(|line| line.parse::<usize>().ok()) {
                    Some(line) if line > 0 => {
                        breakpoints.insert(line);
                        eprintln!("breakpoint set at line {line}");
                    }
                    _ => eprintln!("usage: break <line>"),
                },
                "d" | "delete" => match words.next().and_then(|line| line.parse::<usize>().ok()) {
                    Some(line) if breakpoints.remove(&line) => {
                        eprintln!("breakpoint removed from line {line}");
                    }
                    _ => eprintln!("no breakpoint at that line"),
                },
                "p" | "print" => match words.next() {
                    Some(name) => match event.variables.get(name) {
                        Some(value) => eprintln!("{name} = {value}"),
                        None => eprintln!("unknown variable '{name}'"),
                    },
                    None => eprintln!("usage: print <variable>"),
                },
                "v" | "vars" | "variables" => {
                    if event.variables.is_empty() {
                        eprintln!("no user variables in scope");
                    }
                    for (name, value) in &event.variables {
                        eprintln!("{name} = {value}");
                    }
                }
                "q" | "quit" => return nivren::runtime::DebugControl::Terminate,
                "h" | "help" => eprintln!(
                    "next/step  continue  break <line>  delete <line>  print <name>  vars  quit"
                ),
                "" => {}
                other => eprintln!("unknown command '{other}'; type 'help'"),
            }
        }
    });
    match interpreter.run_bytecode(&chunk) {
        Ok(_) => {
            eprintln!("program finished");
            ExitCode::SUCCESS
        }
        Err(error) if error.message == nivren::runtime::DEBUGGER_TERMINATED => {
            eprintln!("program terminated by debugger");
            ExitCode::SUCCESS
        }
        Err(error) => {
            report(path, &source, &[error]);
            ExitCode::from(70)
        }
    }
}

fn inspect_path(path: &str, output: &str) -> ExitCode {
    let (manifest, chunk) = if is_bundle_path(Path::new(path)) {
        match fs::read(path)
            .map_err(|error| NivError::new(error.to_string(), 1, 1))
            .and_then(|bytes| nivren::bundle::decode(&bytes))
        {
            Ok(chunk) => (None, chunk),
            Err(error) => {
                report(path, "", &[error]);
                return ExitCode::from(65);
            }
        }
    } else if is_project_path(Path::new(path)) {
        match compile_project(Path::new(path)) {
            Ok((manifest, chunk)) => (Some(manifest), chunk),
            Err(errors) => {
                report(path, "", &errors);
                return ExitCode::from(65);
            }
        }
    } else {
        match compile_file(Path::new(path)) {
            Ok(chunk) => (None, chunk),
            Err(errors) => {
                let source = fs::read_to_string(path).unwrap_or_default();
                report(path, &source, &errors);
                return ExitCode::from(65);
            }
        }
    };
    let file = match fs::File::create(output) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("error: cannot create inspection stream {output}: {error}");
            return ExitCode::from(73);
        }
    };
    let stream = Arc::new(Mutex::new(file));
    let stream_error = Arc::new(Mutex::new(None::<String>));
    let write_event =
        |stream: &Arc<Mutex<fs::File>>, value: &serde_json::Value| -> Result<(), String> {
            let mut stream = stream.lock().unwrap();
            serde_json::to_writer(&mut *stream, value).map_err(|error| error.to_string())?;
            stream.write_all(b"\n").map_err(|error| error.to_string())?;
            stream.flush().map_err(|error| error.to_string())
        };
    if let Err(error) = write_event(
        &stream,
        &serde_json::json!({
            "schema": "org.nivren.inspect.v1",
            "kind": "started",
            "runtime": nivren::VERSION,
            "program": Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or("program"),
            "privacy": "source and variable values omitted",
        }),
    ) {
        eprintln!("error: cannot write inspection stream {output}: {error}");
        return ExitCode::from(73);
    }
    let hook_stream = Arc::clone(&stream);
    let hook_error = Arc::clone(&stream_error);
    let mut interpreter = manifest
        .as_ref()
        .map_or_else(Interpreter::new, project_interpreter);
    interpreter.enable_metrics();
    interpreter.set_debug_hook(move |event| {
        let variable_names = event.variables.keys().cloned().collect::<Vec<_>>();
        let value = serde_json::json!({
            "schema": "org.nivren.inspect.v1",
            "kind": "step",
            "instruction": event.instruction,
            "line": event.line,
            "column": event.column,
            "operation": event.operation,
            "stack_depth": event.stack_depth,
            "call_depth": event.call_depth,
            "variable_names": variable_names,
        });
        if let Err(error) = write_event(&hook_stream, &value) {
            *hook_error.lock().unwrap() = Some(error);
            return nivren::runtime::DebugControl::Terminate;
        }
        nivren::runtime::DebugControl::Continue
    });
    let started = Instant::now();
    let result = interpreter.run_bytecode(&chunk);
    if let Some(error) = stream_error.lock().unwrap().take() {
        eprintln!("error: cannot write inspection stream {output}: {error}");
        return ExitCode::from(73);
    }
    let metrics = interpreter.execution_metrics().unwrap_or_default();
    let heap = interpreter.heap_stats();
    let final_event = serde_json::json!({
        "schema": "org.nivren.inspect.v1",
        "kind": "finished",
        "status": if result.is_ok() { "ok" } else { "error" },
        "duration_nanoseconds": u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
        "instructions": metrics.instructions,
        "heap": {
            "tracked_environments": heap.tracked_environments,
            "live_environments": heap.live_environments,
            "collections": heap.collections,
        },
        "error": result.as_ref().err().map(|error| serde_json::json!({
            "message": error.message,
            "line": error.line,
            "column": error.column,
        })),
    });
    if let Err(error) = write_event(&stream, &final_event) {
        eprintln!("error: cannot write inspection stream {output}: {error}");
        return ExitCode::from(73);
    }
    match result {
        Ok(_) => {
            println!("inspection stream {output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            let source = fs::read_to_string(path).unwrap_or_default();
            report(path, &source, &[error]);
            ExitCode::from(70)
        }
    }
}

fn explain_story_path(path: &str) -> ExitCode {
    let program = if is_project_path(Path::new(path)) {
        let manifest = match nivren::project::Manifest::load(Path::new(path)) {
            Ok(manifest) => manifest,
            Err(error) => {
                report(path, "", &[error]);
                return ExitCode::from(65);
            }
        };
        match nivren::modules::load_project(&manifest.root, &manifest.entry_path()) {
            Ok(program) => program,
            Err(errors) => {
                report(path, "", &errors);
                return ExitCode::from(65);
            }
        }
    } else {
        let source = match read_source(path) {
            Ok(source) => source,
            Err(code) => return code,
        };
        match nivren::lexer::scan(&source)
            .and_then(nivren::parser::parse)
            .and_then(nivren::expand::expand_program)
        {
            Ok(program) => program,
            Err(errors) => {
                report(path, &source, &errors);
                return ExitCode::from(65);
            }
        }
    };
    if let Err(errors) = nivren::typecheck::check(&program) {
        report(path, "", &errors);
        return ExitCode::from(65);
    }
    let graph = nivren::intent::analyze(&program, nivren::intent::Optimization::Enabled);
    if let Err(message) = graph.validate() {
        report(path, "", &[NivError::new(message, 1, 1)]);
        return ExitCode::from(70);
    }
    print!("{}", graph.story());
    ExitCode::SUCCESS
}

fn explain_path(path: &str, optimized: bool) -> ExitCode {
    let output = if is_project_path(Path::new(path)) {
        let manifest = match nivren::project::Manifest::load(Path::new(path)) {
            Ok(manifest) => manifest,
            Err(error) => {
                report(path, "", &[error]);
                return ExitCode::from(65);
            }
        };
        let program = match nivren::modules::load_project(&manifest.root, &manifest.entry_path()) {
            Ok(program) => program,
            Err(errors) => {
                report(path, "", &errors);
                return ExitCode::from(65);
            }
        };
        if let Err(errors) = nivren::typecheck::check(&program) {
            report(path, "", &errors);
            return ExitCode::from(65);
        }
        let optimization = if optimized {
            nivren::intent::Optimization::Enabled
        } else {
            nivren::intent::Optimization::Disabled
        };
        let graph = nivren::intent::analyze(&program, optimization);
        if let Err(message) = graph.validate() {
            report(path, "", &[NivError::new(message, 1, 1)]);
            return ExitCode::from(70);
        }
        graph.to_json()
    } else {
        let source = match read_source(path) {
            Ok(source) => source,
            Err(code) => return code,
        };
        match nivren::compiler::Compiler::new().explain(&source, optimized) {
            Ok(output) => output,
            Err(errors) => {
                for error in errors {
                    eprintln!(
                        "{path}:{}:{}: error: {}",
                        error.line, error.column, error.message
                    );
                }
                return ExitCode::from(65);
            }
        }
    };
    print!("{output}");
    ExitCode::SUCCESS
}

fn executable_lines(chunk: &nivren::bytecode::Chunk, lines: &mut BTreeSet<usize>) {
    for instruction in &chunk.code {
        lines.insert(instruction.span.line);
        match &instruction.op {
            nivren::bytecode::Op::MakeFunction { body, .. }
            | nivren::bytecode::Op::DefineModule { body, .. }
            | nivren::bytecode::Op::Iterate { body, .. } => executable_lines(body, lines),
            nivren::bytecode::Op::Match(arms) => {
                for arm in arms {
                    executable_lines(&arm.body, lines);
                }
            }
            _ => {}
        }
    }
}

fn format_path(path: &str, check: bool) -> ExitCode {
    let mut files = vec![];
    if let Err(error) = collect_sources(Path::new(path), &mut files) {
        eprintln!("error: cannot discover sources in {path}: {error}");
        return ExitCode::from(66);
    }
    files.sort();
    let mut changed = 0;
    for file in &files {
        let source = match fs::read_to_string(file) {
            Ok(source) => source,
            Err(error) => {
                eprintln!("error: cannot read {}: {error}", file.display());
                return ExitCode::from(66);
            }
        };
        let formatted = nivren::formatter::format(&source);
        if source != formatted {
            changed += 1;
            if check {
                eprintln!("needs formatting: {}", file.display());
            } else if let Err(error) = fs::write(file, formatted) {
                eprintln!("error: cannot write {}: {error}", file.display());
                return ExitCode::from(73);
            }
        }
    }
    if check && changed > 0 {
        ExitCode::FAILURE
    } else {
        println!("formatted {} file(s)", files.len());
        ExitCode::SUCCESS
    }
}

fn document_project(path: &str) -> ExitCode {
    let manifest = match nivren::project::Manifest::load(Path::new(path)) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!(
                "{path}:{}:{}: error: {}",
                error.line, error.column, error.message
            );
            return ExitCode::from(65);
        }
    };
    if let Err(error) = verify_dependency_lock(&manifest) {
        report(path, "", &[error]);
        return ExitCode::from(65);
    }
    let program = match nivren::modules::load_project(&manifest.root, &manifest.entry_path()) {
        Ok(program) => program,
        Err(errors) => {
            report(path, "", &errors);
            return ExitCode::from(65);
        }
    };
    if let Err(errors) = nivren::typecheck::check(&program) {
        report(path, "", &errors);
        return ExitCode::from(65);
    }
    let directory = manifest.root.join("target/doc");
    if let Err(error) = fs::create_dir_all(&directory) {
        eprintln!("error: cannot create {}: {error}", directory.display());
        return ExitCode::from(73);
    }
    let output = directory.join("api.md");
    let docs = nivren::documentation::generate(&manifest.name, &manifest.version, &program);
    if let Err(error) = fs::write(&output, docs) {
        eprintln!("error: cannot write {}: {error}", output.display());
        return ExitCode::from(73);
    }
    println!("generated {}", output.display());
    ExitCode::SUCCESS
}

fn collect_sources(path: &Path, files: &mut Vec<std::path::PathBuf>) -> io::Result<()> {
    if path.is_file() {
        if path.extension().is_some_and(|extension| extension == "niv") {
            files.push(path.to_path_buf());
        }
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        if child.is_dir()
            && child
                .file_name()
                .is_some_and(|name| name != "target" && name != ".niv")
        {
            collect_sources(&child, files)?;
        } else if child
            .extension()
            .is_some_and(|extension| extension == "niv")
        {
            files.push(child);
        }
    }
    Ok(())
}

fn check_project(path: &str, write_lock: bool) -> ExitCode {
    let incremental = if write_lock {
        let manifest = match nivren::project::Manifest::load(Path::new(path)) {
            Ok(manifest) => manifest,
            Err(error) => {
                report(path, "", &[error]);
                return ExitCode::from(65);
            }
        };
        let fingerprint = match manifest.fingerprint() {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                report(path, "", &[error]);
                return ExitCode::from(65);
            }
        };
        let target = manifest.root.join("target");
        let cache = target.join(".nivren-fingerprint");
        let bundle = target.join(format!("{}.nivb", manifest.name));
        let cached = fs::read_to_string(&cache).is_ok_and(|value| value.trim() == fingerprint);
        let bundle_valid = fs::read(&bundle)
            .ok()
            .and_then(|bytes| nivren::bundle::decode(&bytes).ok())
            .is_some();
        if cached && bundle_valid {
            let lockfile = manifest.root.join(nivren::project::LOCKFILE_NAME);
            let expected_lock = match nivren::package::installed_lockfile(&manifest) {
                Ok(lock) => lock,
                Err(error) => {
                    report(path, "", &[error]);
                    return ExitCode::from(65);
                }
            };
            if fs::read_to_string(&lockfile).ok().as_deref() != Some(expected_lock.as_str())
                && let Err(error) = fs::write(&lockfile, expected_lock)
            {
                eprintln!("error: cannot write {}: {error}", lockfile.display());
                return ExitCode::from(73);
            }
            println!(
                "fresh {} {} ({})",
                manifest.name,
                manifest.version,
                bundle.display()
            );
            return ExitCode::SUCCESS;
        }
        Some(fingerprint)
    } else {
        None
    };
    let (manifest, chunk) = match compile_project(Path::new(path)) {
        Ok(result) => result,
        Err(errors) => {
            report(path, "", &errors);
            return ExitCode::from(65);
        }
    };
    let entry = manifest.entry_path();
    if write_lock {
        let lockfile = manifest.root.join(nivren::project::LOCKFILE_NAME);
        let expected_lock = match nivren::package::installed_lockfile(&manifest) {
            Ok(lock) => lock,
            Err(error) => {
                report(path, "", &[error]);
                return ExitCode::from(65);
            }
        };
        if let Err(error) = fs::write(&lockfile, expected_lock) {
            eprintln!("error: cannot write {}: {error}", lockfile.display());
            return ExitCode::from(73);
        }
        let target = manifest.root.join("target");
        if let Err(error) = fs::create_dir_all(&target) {
            eprintln!("error: cannot create {}: {error}", target.display());
            return ExitCode::from(73);
        }
        let bundle = target.join(format!("{}.nivb", manifest.name));
        let bytes = match nivren::bundle::encode(&chunk) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("error: cannot encode bundle: {error}");
                return ExitCode::from(70);
            }
        };
        if let Err(error) = write_atomic(&bundle, &bytes) {
            eprintln!("error: cannot write {}: {error}", bundle.display());
            return ExitCode::from(73);
        }
        let cache = target.join(".nivren-fingerprint");
        if let Err(error) = write_atomic(&cache, incremental.unwrap().as_bytes()) {
            eprintln!("error: cannot write {}: {error}", cache.display());
            return ExitCode::from(73);
        }
        println!(
            "built {} {} ({})",
            manifest.name,
            manifest.version,
            entry.display()
        );
    } else {
        println!("{}: ok", manifest.root.display());
    }
    ExitCode::SUCCESS
}

fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension(format!(
        "{}.tmp-{}",
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("file"),
        std::process::id()
    ));
    fs::write(&temporary, contents)?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)
}

fn test_path_with_time(path: &str, seconds: &str) -> ExitCode {
    match seconds.parse::<f64>() {
        Ok(seconds) => test_path(path, None, Some(seconds)),
        Err(_) => {
            eprintln!("error: deterministic test time must be a finite nonnegative number");
            ExitCode::from(64)
        }
    }
}

fn test_path(path: &str, snapshots: Option<bool>, deterministic_time: Option<f64>) -> ExitCode {
    let _clock = match deterministic_time.map(nivren::runtime::deterministic_clock) {
        Some(Ok(clock)) => Some(clock),
        Some(Err(error)) => {
            report(path, "", &[error]);
            return ExitCode::from(64);
        }
        None => None,
    };
    let manifest = match enclosing_manifest(Path::new(path)) {
        Ok(manifest) => manifest,
        Err(error) => {
            report(path, "", &[error]);
            return ExitCode::from(65);
        }
    };
    let mut files = vec![];
    if let Err(error) = collect_tests(Path::new(path), &mut files) {
        eprintln!("error: cannot discover tests in {path}: {error}");
        return ExitCode::from(66);
    }
    files.sort();
    if files.is_empty() {
        eprintln!("error: no *_test.niv files found in {path}");
        return ExitCode::from(66);
    }
    let mut failed = 0;
    for file in &files {
        let display = file.display().to_string();
        let source = match fs::read_to_string(file) {
            Ok(source) => source,
            Err(error) => {
                eprintln!("FAIL {display}: {error}");
                failed += 1;
                continue;
            }
        };
        match compile_file(file).and_then(|chunk| {
            let mut interpreter = manifest
                .as_ref()
                .map_or_else(Interpreter::new, project_interpreter);
            interpreter.enable_samples();
            interpreter
                .run_bytecode(&chunk)
                .map_err(|error| vec![error])
        }) {
            Ok(value) => {
                if let Some(accept) = snapshots {
                    let snapshot = PathBuf::from(format!("{display}.snap"));
                    let actual = format!("{value}\n");
                    if accept {
                        match write_atomic(&snapshot, actual.as_bytes()) {
                            Ok(()) => println!("PASS {display} (snapshot accepted)"),
                            Err(error) => {
                                println!("FAIL {display}");
                                eprintln!("error: cannot write {}: {error}", snapshot.display());
                                failed += 1;
                            }
                        }
                    } else {
                        match fs::read_to_string(&snapshot) {
                            Ok(expected) if expected == actual => println!("PASS {display}"),
                            Ok(expected) => {
                                println!("FAIL {display}");
                                eprintln!(
                                    "snapshot differs: {}\n  expected: {:?}\n  actual:   {:?}",
                                    snapshot.display(),
                                    expected.trim_end(),
                                    actual.trim_end()
                                );
                                failed += 1;
                            }
                            Err(error) => {
                                println!("FAIL {display}");
                                eprintln!(
                                    "snapshot missing: {} ({error}); review and run niv test --accept-snapshots {path}",
                                    snapshot.display()
                                );
                                failed += 1;
                            }
                        }
                    }
                } else {
                    println!("PASS {display}");
                }
            }
            Err(errors) => {
                println!("FAIL {display}");
                report(&display, &source, &errors);
                failed += 1;
            }
        }
    }
    println!("\n{} passed; {failed} failed", files.len() - failed);
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn enclosing_manifest(path: &Path) -> Result<Option<nivren::project::Manifest>, NivError> {
    let mut current = if path.is_file() {
        path.parent()
    } else {
        Some(path)
    };
    while let Some(directory) = current {
        let candidate = directory.join(nivren::project::MANIFEST_NAME);
        if candidate.is_file() {
            return nivren::project::Manifest::load(&candidate).map(Some);
        }
        current = directory.parent();
    }
    Ok(None)
}

fn collect_tests(path: &Path, files: &mut Vec<std::path::PathBuf>) -> io::Result<()> {
    if path.is_file() {
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with("_test.niv"))
        {
            files.push(path.to_path_buf());
        }
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        if child.is_dir() {
            collect_tests(&child, files)?;
        } else if child
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with("_test.niv"))
        {
            files.push(child);
        }
    }
    Ok(())
}

fn read_source(path: &str) -> Result<String, ExitCode> {
    fs::read_to_string(path).map_err(|error| {
        eprintln!("error: cannot read {path}: {error}");
        ExitCode::from(66)
    })
}

fn compile_file(path: &Path) -> Result<nivren::bytecode::Chunk, Vec<NivError>> {
    let program = nivren::modules::load(path)?;
    nivren::typecheck::check(&program)?;
    nivren::bytecode::compile(&program)
}

fn compile_project(
    path: &Path,
) -> Result<(nivren::project::Manifest, nivren::bytecode::Chunk), Vec<NivError>> {
    let manifest = nivren::project::Manifest::load(path).map_err(|error| vec![error])?;
    verify_dependency_lock(&manifest).map_err(|error| vec![error])?;
    let program = nivren::modules::load_project(&manifest.root, &manifest.entry_path())?;
    nivren::typecheck::check_with_edition(&program, manifest.edition)?;
    let chunk = nivren::bytecode::compile(&program)?;
    Ok((manifest, chunk))
}

fn verify_dependency_lock(manifest: &nivren::project::Manifest) -> Result<(), NivError> {
    if manifest.dependencies.is_empty() {
        return Ok(());
    }
    let expected = nivren::package::installed_lockfile(manifest)?;
    let actual = fs::read_to_string(manifest.root.join(nivren::project::LOCKFILE_NAME))
        .map_err(|_| NivError::new("dependency lockfile is missing; run 'niv install'", 1, 1))?;
    if actual != expected {
        return Err(NivError::new(
            "dependency lockfile is stale; run 'niv install'",
            1,
            1,
        ));
    }
    Ok(())
}

fn repl() -> ExitCode {
    println!(
        "Nivren {} — type :help for help, :quit to exit",
        nivren::VERSION
    );
    let mut interpreter = Interpreter::new();
    let mut line = String::new();
    loop {
        print!("> ");
        if io::stdout().flush().is_err() {
            return ExitCode::FAILURE;
        }
        line.clear();
        match io::stdin().read_line(&mut line) {
            Ok(0) => {
                println!();
                break;
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
        }
        match line.trim() {
            "" => continue,
            ":quit" | ":q" => break,
            ":help" => {
                println!("Enter Nivren code, or use :quit to exit.");
                continue;
            }
            _ => {}
        }
        let result = nivren::lexer::scan(&line)
            .and_then(nivren::parser::parse)
            .and_then(|program| nivren::typecheck::check(&program).map(|_| program))
            .and_then(|program| nivren::bytecode::compile(&program))
            .and_then(|chunk| {
                interpreter
                    .run_bytecode(&chunk)
                    .map_err(|error| vec![error])
            });
        match result {
            Ok(value) if value != Value::Null => println!("{value}"),
            Ok(_) => {}
            Err(errors) => report("<repl>", &line, &errors),
        }
        interpreter.reset_to_globals();
    }
    ExitCode::SUCCESS
}

fn lsp() -> ExitCode {
    match nivren::lsp::serve(io::stdin().lock(), io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("language server error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn report(path: &str, source: &str, errors: &[NivError]) {
    let lines: Vec<&str> = source.lines().collect();
    for error in errors {
        let code = error
            .code()
            .map(|code| format!("[{code}]"))
            .unwrap_or_default();
        eprintln!(
            "{path}:{}:{}: error{code}: {}",
            error.line, error.column, error.message
        );
        if let Some(line) = lines.get(error.line.saturating_sub(1)) {
            eprintln!(
                "  |\n{:>2} | {line}\n  | {}^",
                error.line,
                " ".repeat(error.column.saturating_sub(1))
            );
        }
        if let Some(suggestion) = error.suggestion() {
            eprintln!("  try: {suggestion}");
        }
        for frame in error.trace.iter().rev() {
            eprintln!(
                "  at {} ({}:{}:{})",
                frame.function, path, frame.line, frame.column
            );
        }
    }
}

fn help() {
    println!(
        "Nivren {}\n\nProject path:\n  niv new <project>\n  niv add <package> <version> [project]\n  niv dev [project]\n  niv test [--snapshots|--accept-snapshots] [path]\n  niv bench [--json <output.json>] [file.niv|file.nivb|project]\n  niv ship [project]\n  niv workspace <check|build|test|bench|ship> [workspace]\n\nBuild and inspect:\n  niv run [file.niv|file.nivb|project]\n  niv run --native [file.niv|file.nivb|project]\n  niv run --crash-report <output.json> <file|project>\n  niv check <file.niv|file.nivb|project>\n  niv build [project]\n  niv build --standalone [project]\n  niv build --standalone --native [project]\n  niv build --aot [project]\n  niv fmt [--check] <file|path>\n  niv doc [project]\n  niv package [project]\n  niv package verify <file.nivpkg>\n  niv disasm <file.niv|file.nivb|project>\n  niv explain [--no-optimize|--story] <file.niv|project>\n  niv record <file.niv> <trace.jsonl>\n  niv replay <file.niv> <trace.jsonl>\n  niv sourcemap <file.niv|file.nivb|project> <output.json>\n  niv debug <file.niv|file.nivb|project>\n  niv inspect <file.niv|file.nivb|project> <output.jsonl>\n  niv profile [--json <output.json>] <file.niv|file.nivb|project>\n  niv coverage [--json <output.json>] <file.niv|file.nivb|project>\n\nPackages, authority, and registry:\n  niv install <registry> [project]\n  niv install --trusted <https-registry> <root-key> [project]\n  niv install --offline [project]\n  niv cache <list|prune> [project]\n  niv authority <lock|check|report> [project]\n  niv registry search <query> <registry>\n  niv registry publish <file.nivpkg> <registry>\n  niv registry fetch <name> <version> <registry> <destination>\n  niv registry yank <name> <version> <registry>\n  niv registry unyank <name> <version> <registry>\n  niv registry envelope <package> <provenance> <authorization> <output>\n  niv registry sign-admin <yank|unyank> <name> <version> <generation> <issued> <expires> <reason-file> <root-secret-file> <output>\n  niv registry verify-admin <action> <root-key> <unix-time> <minimum-generation>\n  niv registry serve <registry> <bind-address> [minimum-generation]\n  niv registry verify-release <package> <provenance> <authorization> <status> <advisories> <root-key> <unix-time> <minimum-generation>\n  niv release check [repository]\n\nTools:\n  niv repl\n  niv lsp\n  niv dap\n  niv version\n  niv help",
        nivren::VERSION
    );
    println!(
        "\nSigned release channels:\n  niv release sign-channel <manifest.json> <secret-key-file> <signed.json>\n  niv release verify-channel <signed.json> <public-key-file> <unix-time> <minimum-generation> [expected-channel]

Registry trust operations:
  niv trust keygen <secret-output>
  niv trust authorize <publisher> <publisher-key-file> <repository> <workflow> <expires-unix> <root-secret-file> <output.json>
  niv trust attest <file.nivpkg> <publisher> <repository> <workflow> <commit> <publisher-secret-file> <output.json>
  niv trust sign-status <status.json> <root-secret-file> <output.json> [advisories.json]
  niv trust sign-advisory <advisory.json> <root-secret-file> <output.json>"
    );
    println!(
        "\nTest profiles:\n  niv test --property [path]\n  niv test --compat [path]\n  niv test --fuzz-smoke [path]\n  niv test --time <unix-seconds> [path]"
    );
    println!("\nDependency cache:\n  niv cache list [project]\n  niv cache prune [project]");
    println!(
        "\nRegistry recovery:\n  niv registry recover-admin <registry> <unix-time> <minimum-generation>"
    );
    println!("\nBinding generation:\n  niv bindgen c <schema.niv> <output.h>");
}
