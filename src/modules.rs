use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::ast::Stmt;
use crate::error::NivError;
use crate::project::{MANIFEST_NAME, Manifest};

pub fn load(entry: &Path) -> Result<Vec<Stmt>, Vec<NivError>> {
    Loader::new(None).file(entry)
}

pub fn load_project(root: &Path, entry: &Path) -> Result<Vec<Stmt>, Vec<NivError>> {
    let root = root.canonicalize().map_err(|error| {
        vec![NivError::new(
            format!("cannot resolve project root '{}': {error}", root.display()),
            1,
            1,
        )]
    })?;
    Loader::new(Some(root)).file(entry)
}

struct Loader {
    cache: HashMap<PathBuf, Vec<Stmt>>,
    stack: Vec<PathBuf>,
    root: Option<PathBuf>,
}

impl Loader {
    fn new(root: Option<PathBuf>) -> Self {
        Self {
            cache: HashMap::new(),
            stack: vec![],
            root,
        }
    }

    fn file(&mut self, path: &Path) -> Result<Vec<Stmt>, Vec<NivError>> {
        let canonical = path.canonicalize().map_err(|error| {
            vec![NivError::new(
                format!("cannot resolve module '{}': {error}", path.display()),
                1,
                1,
            )]
        })?;
        if self
            .root
            .as_ref()
            .is_some_and(|root| !canonical.starts_with(root))
        {
            return Err(vec![NivError::new(
                format!(
                    "module '{}' is outside the project root",
                    canonical.display()
                ),
                1,
                1,
            )]);
        }
        if let Some(start) = self.stack.iter().position(|active| active == &canonical) {
            let mut cycle: Vec<String> = self.stack[start..]
                .iter()
                .map(|item| item.display().to_string())
                .collect();
            cycle.push(canonical.display().to_string());
            return Err(vec![NivError::new(
                format!("use cycle: {}", cycle.join(" -> ")),
                1,
                1,
            )]);
        }
        if let Some(program) = self.cache.get(&canonical) {
            return Ok(program.clone());
        }

        let source = fs::read_to_string(&canonical).map_err(|error| {
            vec![NivError::new(
                format!("cannot read module '{}': {error}", canonical.display()),
                1,
                1,
            )]
        })?;
        let tokens = crate::lexer::scan(&source).map_err(|mut errors| {
            prefix_errors(&mut errors, &canonical);
            errors
        })?;
        let parsed = crate::parser::parse(tokens).map_err(|mut errors| {
            prefix_errors(&mut errors, &canonical);
            errors
        })?;

        self.stack.push(canonical.clone());
        let mut imported_paths = HashSet::new();
        let mut program = vec![];
        for statement in parsed {
            if let Stmt::Import { path, span } = statement {
                let (module_path, dependency_name) =
                    self.resolve_import(&canonical, &path, span)?;
                let resolved = module_path.canonicalize().map_err(|error| {
                    vec![NivError::new(
                        format!("cannot resolve module '{}': {error}", module_path.display()),
                        span.line,
                        span.column,
                    )]
                })?;
                if !imported_paths.insert(resolved.clone()) {
                    continue;
                }
                let loaded = self.file(&resolved).map_err(|mut errors| {
                    for error in &mut errors {
                        error.message = format!(
                            "used at {}:{}:{}: {}",
                            canonical.display(),
                            span.line,
                            span.column,
                            error.message
                        );
                    }
                    errors
                })?;
                let name = dependency_name
                    .or_else(|| module_name(&resolved))
                    .ok_or_else(|| {
                        vec![NivError::new(
                            "module filename must be a valid identifier",
                            span.line,
                            span.column,
                        )]
                    })?;
                let (body, exports) = split_exports(loaded);
                program.push(Stmt::Module {
                    name,
                    body,
                    exports,
                    span,
                });
            } else {
                program.push(statement);
            }
        }
        self.stack.pop();
        self.cache.insert(canonical, program.clone());
        Ok(program)
    }

    fn resolve_import(
        &self,
        importer: &Path,
        import: &str,
        span: crate::ast::Span,
    ) -> Result<(PathBuf, Option<String>), Vec<NivError>> {
        let Some(name) = import.strip_prefix('@') else {
            return Ok((
                importer.parent().unwrap_or(Path::new(".")).join(import),
                None,
            ));
        };
        if name.is_empty()
            || !name.bytes().enumerate().all(|(index, byte)| {
                if index == 0 {
                    byte.is_ascii_alphabetic() || byte == b'_'
                } else {
                    byte.is_ascii_alphanumeric() || byte == b'_'
                }
            })
        {
            return Err(vec![NivError::new(
                "package uses must have the form '@identifier'",
                span.line,
                span.column,
            )]);
        }
        let root = self.root.as_ref().ok_or_else(|| {
            vec![NivError::new(
                "package uses require a Nivren project",
                span.line,
                span.column,
            )]
        })?;
        let owner = manifest_ancestor(importer, root).ok_or_else(|| {
            vec![NivError::new(
                "cannot determine the using package",
                span.line,
                span.column,
            )]
        })?;
        let owner_manifest = Manifest::load(&owner).map_err(|error| vec![error])?;
        let version = owner_manifest.dependencies.get(name).ok_or_else(|| {
            vec![NivError::new(
                format!("package '{name}' is not declared in [dependencies]"),
                span.line,
                span.column,
            )]
        })?;
        let dependency_root = root.join(".niv/deps").join(format!("{name}-{version}"));
        let dependency = Manifest::load(&dependency_root).map_err(|_| {
            vec![NivError::new(
                format!("package '{name}' {version} is not installed; run 'niv install'"),
                span.line,
                span.column,
            )]
        })?;
        if dependency.name != name || &dependency.version != version {
            return Err(vec![NivError::new(
                format!("installed package '{name}' has the wrong identity"),
                span.line,
                span.column,
            )]);
        }
        Ok((dependency.entry_path(), Some(name.to_string())))
    }
}

fn manifest_ancestor(importer: &Path, root: &Path) -> Option<PathBuf> {
    let mut current = importer.parent()?;
    loop {
        if current.join(MANIFEST_NAME).is_file() {
            return Some(current.to_path_buf());
        }
        if current == root {
            return None;
        }
        current = current.parent()?;
        if !current.starts_with(root) {
            return None;
        }
    }
}

fn split_exports(program: Vec<Stmt>) -> (Vec<Stmt>, Vec<String>) {
    let mut body = vec![];
    let mut exports = vec![];
    for statement in program {
        if let Stmt::Export { names, .. } = statement {
            exports.extend(names);
        } else {
            body.push(statement);
        }
    }
    (body, exports)
}

fn module_name(path: &Path) -> Option<String> {
    let name = path.file_stem()?.to_str()?.to_string();
    let mut bytes = name.bytes();
    let first = bytes.next()?;
    (first.is_ascii_alphabetic() && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
        .then_some(name)
}

fn prefix_errors(errors: &mut [NivError], path: &Path) {
    for error in errors {
        error.message = format!("{}: {}", path.display(), error.message);
    }
}
