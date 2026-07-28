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

fn main() -> ExitCode {
    if let Some(code) = run_embedded_application() {
        return code;
    }
    let arguments: Vec<String> = env::args().skip(1).collect();
    match arguments.as_slice() {
        [] => repl(),
        [command] if command == "repl" => repl(),
        [command] if command == "lsp" => lsp(),
        [command] if command == "version" || command == "--version" || command == "-V" => {
            println!("Nivren {}", nivren::VERSION);
            ExitCode::SUCCESS
        }
        [command] if command == "help" || command == "--help" || command == "-h" => {
            help();
            ExitCode::SUCCESS
        }
        [command] if command == "run" => run_file("."),
        [command, path] if command == "run" => run_file(path),
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
        [command, path] if command == "check" => check_file(path),
        [command] if command == "build" => build_project("."),
        [command, flag] if command == "build" && flag == "--standalone" => build_standalone("."),
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
        [command, action] if command == "release" && action == "check" => release_check("."),
        [command, action, path] if command == "release" && action == "check" => release_check(path),
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
        [command] if command == "test" => test_path("tests/niv", None),
        [command, flag] if command == "test" && flag == "--snapshots" => {
            test_path("tests/niv", Some(false))
        }
        [command, flag, path] if command == "test" && flag == "--snapshots" => {
            test_path(path, Some(false))
        }
        [command, flag] if command == "test" && flag == "--accept-snapshots" => {
            test_path("tests/niv", Some(true))
        }
        [command, flag, path] if command == "test" && flag == "--accept-snapshots" => {
            test_path(path, Some(true))
        }
        [command, path] if command == "test" => test_path(path, None),
        [path] if path.ends_with(".niv") => run_file(path),
        [path] if path.ends_with(".nivb") => run_file(path),
        _ => {
            eprintln!("error: invalid command\n");
            help();
            ExitCode::from(64)
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
    let result = nivren::bundle::decode(&application.bundle)
        .and_then(|chunk| project_interpreter(&manifest).run_bytecode(&chunk));
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

fn project_interpreter(manifest: &nivren::project::Manifest) -> Interpreter {
    let interpreter = Interpreter::new()
        .with_capabilities(manifest.capabilities.iter().cloned())
        .with_capability_scopes(manifest.capability_scopes.clone());
    let interpreter = if let Some(limit) = manifest.instruction_limit {
        interpreter.with_instruction_limit(limit)
    } else {
        interpreter
    };
    if let Some(limit) = manifest.memory_limit {
        interpreter.with_memory_limit(limit)
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
                "1.0 release gate passed: {} pilots, {} conformance cases",
                audit.pilots, audit.conformance_cases
            );
            ExitCode::SUCCESS
        }
        Ok(audit) => {
            eprintln!("1.0 release gate blocked:");
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
    let application = match nivren::standalone::attach(&current, &bundle, &manifest.source()) {
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
        let extension = if cfg!(windows) { "obj" } else { "o" };
        let output = directory.join(format!("{name}.{extension}"));
        if let Err(error) = write_atomic(&output, &bytes) {
            eprintln!("error: cannot write {}: {error}", output.display());
            return ExitCode::from(73);
        }
        println!("aot {}", output.display());
        emitted += 1;
    }
    if emitted == 0 {
        eprintln!("error: no pure explicitly typed Int functions are eligible for native AOT");
        return ExitCode::from(65);
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
            "define main() gives Null {\n    show(\"Welcome to Nivren\")\n    give none\n}\n\nmain()\n",
        )?;
        fs::write(
            root.join("tests/niv/main_test.niv"),
            "define answer() gives Int { give 42 }\nassert(answer() == 42, \"answer\")\n",
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
        let tested = test_path(&tests.display().to_string(), None);
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
    }
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

fn test_path(path: &str, snapshots: Option<bool>) -> ExitCode {
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
            manifest
                .as_ref()
                .map_or_else(Interpreter::new, project_interpreter)
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
    nivren::typecheck::check(&program)?;
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
        eprintln!(
            "{path}:{}:{}: error: {}",
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
        "Nivren {}\n\nProject path:\n  niv new <project>\n  niv add <package> <version> [project]\n  niv dev [project]\n  niv test [--snapshots|--accept-snapshots] [path]\n  niv ship [project]\n\nBuild and inspect:\n  niv run [file.niv|file.nivb|project]\n  niv run --crash-report <output.json> <file|project>\n  niv check <file.niv|file.nivb|project>\n  niv build [project]\n  niv build --standalone [project]\n  niv build --aot [project]\n  niv fmt [--check] <file|path>\n  niv doc [project]\n  niv package [project]\n  niv package verify <file.nivpkg>\n  niv disasm <file.niv|file.nivb|project>\n  niv explain [--no-optimize] <file.niv|project>\n  niv sourcemap <file.niv|file.nivb|project> <output.json>\n  niv debug <file.niv|file.nivb|project>\n  niv inspect <file.niv|file.nivb|project> <output.jsonl>\n  niv profile [--json <output.json>] <file.niv|file.nivb|project>\n  niv coverage [--json <output.json>] <file.niv|file.nivb|project>\n\nPackages and registry:\n  niv install <registry> [project]\n  niv install --trusted <https-registry> <root-key> [project]\n  niv registry search <query> <registry>\n  niv registry publish <file.nivpkg> <registry>\n  niv registry fetch <name> <version> <registry> <destination>\n  niv registry envelope <package> <provenance> <authorization> <output>\n  niv registry serve <registry> <bind-address> [minimum-generation]\n  niv registry verify-release <package> <provenance> <authorization> <status> <advisories> <root-key> <unix-time> <minimum-generation>\n  niv release check [repository]\n\nTools:\n  niv repl\n  niv lsp\n  niv version\n  niv help",
        nivren::VERSION
    );
    println!("\nBinding generation:\n  niv bindgen c <schema.niv> <output.h>");
}
