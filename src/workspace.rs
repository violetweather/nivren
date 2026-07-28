use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::NivError;
use crate::project::Manifest;

pub const WORKSPACE_NAME: &str = "niv-workspace.toml";
const MAX_MEMBERS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    pub root: PathBuf,
    pub members: Vec<Manifest>,
}

impl Workspace {
    pub fn load(path: &Path) -> Result<Self, NivError> {
        let manifest_path = if path.is_dir() {
            path.join(WORKSPACE_NAME)
        } else {
            path.to_path_buf()
        };
        let root = manifest_path
            .parent()
            .unwrap_or(Path::new("."))
            .canonicalize()
            .map_err(|error| workspace_error(format!("cannot resolve workspace: {error}"), 1))?;
        let source = fs::read_to_string(&manifest_path).map_err(|error| {
            workspace_error(
                format!("cannot read {}: {error}", manifest_path.display()),
                1,
            )
        })?;
        Self::parse(&source, root)
    }

    pub fn parse(source: &str, root: PathBuf) -> Result<Self, NivError> {
        let mut in_workspace = false;
        let mut raw_members = None;
        for (index, raw_line) in source.lines().enumerate() {
            let line_number = index + 1;
            let line = raw_line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                if line != "[workspace]" || in_workspace {
                    return Err(workspace_error(
                        "workspace manifest only permits one [workspace] section",
                        line_number,
                    ));
                }
                in_workspace = true;
                continue;
            }
            if !in_workspace {
                return Err(workspace_error(
                    "workspace values must be inside [workspace]",
                    line_number,
                ));
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| workspace_error("expected members = \"path,...\"", line_number))?;
            if key.trim() != "members" || raw_members.is_some() {
                return Err(workspace_error(
                    "workspace requires one members value",
                    line_number,
                ));
            }
            raw_members = Some(
                quoted(value.trim())
                    .ok_or_else(|| workspace_error("members must be quoted", line_number))?,
            );
        }
        let raw_members = raw_members.ok_or_else(|| workspace_error("members is required", 1))?;
        let paths = raw_members
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if paths.is_empty() || paths.len() > MAX_MEMBERS {
            return Err(workspace_error(
                "workspace must contain 1 through 256 members",
                1,
            ));
        }
        let mut member_paths = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut members = Vec::with_capacity(paths.len());
        for value in paths {
            let relative = Path::new(value);
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|part| !matches!(part, Component::Normal(_)))
            {
                return Err(workspace_error(
                    format!("workspace member '{value}' must be a normalized relative path"),
                    1,
                ));
            }
            if !member_paths.insert(relative.to_path_buf()) {
                return Err(workspace_error(
                    format!("duplicate workspace member '{value}'"),
                    1,
                ));
            }
            let member_root = root.join(relative).canonicalize().map_err(|error| {
                workspace_error(
                    format!("cannot resolve workspace member '{value}': {error}"),
                    1,
                )
            })?;
            if !member_root.starts_with(&root) {
                return Err(workspace_error(
                    format!("workspace member '{value}' escapes the workspace root"),
                    1,
                ));
            }
            let manifest = Manifest::load(&member_root)?;
            if !names.insert(manifest.name.clone()) {
                return Err(workspace_error(
                    format!("duplicate workspace package name '{}'", manifest.name),
                    1,
                ));
            }
            members.push(manifest);
        }
        let members = dependency_order(members)?;
        Ok(Self { root, members })
    }
}

fn dependency_order(members: Vec<Manifest>) -> Result<Vec<Manifest>, NivError> {
    let positions = members
        .iter()
        .enumerate()
        .map(|(index, member)| (member.name.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let mut indegree = vec![0usize; members.len()];
    let mut dependents = vec![Vec::new(); members.len()];
    for (member_index, member) in members.iter().enumerate() {
        for (dependency, required_version) in &member.dependencies {
            let Some(&dependency_index) = positions.get(dependency) else {
                continue;
            };
            let actual_version = &members[dependency_index].version;
            if required_version != actual_version {
                return Err(workspace_error(
                    format!(
                        "workspace package '{}' requires {dependency} {required_version}, but the member version is {actual_version}",
                        member.name
                    ),
                    1,
                ));
            }
            indegree[member_index] = indegree[member_index].saturating_add(1);
            dependents[dependency_index].push(member_index);
        }
    }

    let mut ordered_indices = Vec::with_capacity(members.len());
    let mut emitted = vec![false; members.len()];
    while ordered_indices.len() < members.len() {
        let Some(next) = (0..members.len()).find(|&index| !emitted[index] && indegree[index] == 0)
        else {
            let cycle = members
                .iter()
                .enumerate()
                .filter(|(index, _)| !emitted[*index])
                .map(|(_, member)| member.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(workspace_error(
                format!("workspace dependency cycle includes: {cycle}"),
                1,
            ));
        };
        emitted[next] = true;
        ordered_indices.push(next);
        for &dependent in &dependents[next] {
            indegree[dependent] = indegree[dependent].saturating_sub(1);
        }
    }

    let mut members = members.into_iter().map(Some).collect::<Vec<_>>();
    Ok(ordered_indices
        .into_iter()
        .map(|index| members[index].take().expect("member is emitted once"))
        .collect())
}

fn quoted(value: &str) -> Option<String> {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .filter(|value| !value.contains(['"', '\n', '\r']))
        .map(str::to_string)
}

fn workspace_error(message: impl Into<String>, line: usize) -> NivError {
    NivError::new(message, line, 1)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::Workspace;

    #[test]
    fn workspace_members_are_bounded_ordered_and_unique() {
        let root = std::env::temp_dir().join(format!(
            "nivren-workspace-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        for name in ["core", "app"] {
            let member = root.join(name);
            fs::create_dir_all(member.join("src")).unwrap();
            fs::write(
                member.join("niv.toml"),
                format!(
                    "[package]\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"src/main.niv\"\n"
                ),
            )
            .unwrap();
            fs::write(member.join("src/main.niv"), "42").unwrap();
        }
        let parsed = Workspace::parse(
            "[workspace]\nmembers = \"core, app\"\n",
            root.canonicalize().unwrap(),
        )
        .unwrap();
        assert_eq!(
            parsed
                .members
                .iter()
                .map(|member| member.name.as_str())
                .collect::<Vec<_>>(),
            ["core", "app"]
        );
        assert!(
            Workspace::parse(
                "[workspace]\nmembers = \"core, core\"\n",
                root.canonicalize().unwrap(),
            )
            .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspace_orders_internal_dependencies_and_rejects_cycles_or_version_drift() {
        let root = std::env::temp_dir().join(format!(
            "nivren-workspace-graph-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        for (name, dependencies) in [
            ("core", ""),
            ("service", "\n[dependencies]\ncore = \"1.0.0\"\n"),
            ("app", "\n[dependencies]\nservice = \"1.0.0\"\n"),
        ] {
            let member = root.join(name);
            fs::create_dir_all(member.join("src")).unwrap();
            fs::write(
                member.join("niv.toml"),
                format!(
                    "[package]\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"src/main.niv\"\n{dependencies}"
                ),
            )
            .unwrap();
            fs::write(member.join("src/main.niv"), "42").unwrap();
        }
        let parsed = Workspace::parse(
            "[workspace]\nmembers = \"app, service, core\"\n",
            root.canonicalize().unwrap(),
        )
        .unwrap();
        assert_eq!(
            parsed
                .members
                .iter()
                .map(|member| member.name.as_str())
                .collect::<Vec<_>>(),
            ["core", "service", "app"]
        );

        fs::write(
            root.join("core/niv.toml"),
            "[package]\nname = \"core\"\nversion = \"1.0.0\"\nentry = \"src/main.niv\"\n\n[dependencies]\napp = \"1.0.0\"\n",
        )
        .unwrap();
        let error = Workspace::parse(
            "[workspace]\nmembers = \"app, service, core\"\n",
            root.canonicalize().unwrap(),
        )
        .unwrap_err();
        assert!(error.message.contains("dependency cycle"));

        fs::write(
            root.join("core/niv.toml"),
            "[package]\nname = \"core\"\nversion = \"2.0.0\"\nentry = \"src/main.niv\"\n",
        )
        .unwrap();
        let error = Workspace::parse(
            "[workspace]\nmembers = \"app, service, core\"\n",
            root.canonicalize().unwrap(),
        )
        .unwrap_err();
        assert!(error.message.contains("member version is 2.0.0"));
        fs::remove_dir_all(root).unwrap();
    }
}
