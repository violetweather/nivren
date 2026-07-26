use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn edition_one_black_box_conformance_vectors() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("../conformance/edition1-baseline.json")).unwrap();
    let cases = cases.as_array().unwrap();
    let directory = std::env::temp_dir().join(format!(
        "nivren-conformance-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::create_dir_all(&directory).unwrap();
    for case in cases {
        run_case(case, &directory);
    }
    fs::remove_dir_all(directory).unwrap();
}

fn run_case(case: &serde_json::Value, directory: &std::path::Path) {
    let name = case["name"].as_str().unwrap();
    let project = directory.join(name);
    fs::create_dir_all(&project).unwrap();
    let source = project.join("case.niv");
    if let Some(contents) = case.get("source").and_then(serde_json::Value::as_str) {
        fs::write(&source, contents).unwrap();
    }
    if let Some(files) = case.get("files").and_then(serde_json::Value::as_object) {
        for (path, contents) in files {
            let path = project.join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, contents.as_str().unwrap()).unwrap();
        }
    }
    let arguments = case["arguments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|argument| {
            let argument = argument.as_str().unwrap();
            if argument == "{source}" {
                source.to_string_lossy().into_owned()
            } else if argument == "{project}" {
                project.to_string_lossy().into_owned()
            } else {
                argument.to_string()
            }
        })
        .collect::<Vec<_>>();
    let output = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_niv")))
        .args(arguments)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        case["status"].as_i64().map(|status| status as i32),
        "status mismatch for {name}: stderr={} ",
        String::from_utf8_lossy(&output.stderr)
    );
    if let Some(expected) = case.get("stdout").and_then(serde_json::Value::as_str) {
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            expected,
            "stdout mismatch for {name}"
        );
    }
    if let Some(expected) = case
        .get("stderr_contains")
        .and_then(serde_json::Value::as_str)
    {
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "stderr mismatch for {name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
