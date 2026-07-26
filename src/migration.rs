pub fn migrate(source: &str, from: &str) -> Result<String, String> {
    match from {
        "0.2" => Ok(rename_identifier(source, "Number", "Int")),
        // These releases added tooling and runtime capabilities without changing
        // Edition 1 source syntax. Keeping the identity steps explicit makes the
        // supported upgrade window auditable and lets old projects advance one
        // release at a time without special cases in callers.
        "0.3" | "0.4" | "0.5" | "0.6" | "0.7" | "0.8" | "0.9" => Ok(source.to_string()),
        _ => Err(format!("unsupported source version '{from}'")),
    }
}

fn rename_identifier(source: &str, old: &str, new: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut output = String::with_capacity(source.len());
    let mut index = 0;
    let mut block_depth = 0usize;
    let mut in_string = false;
    while index < chars.len() {
        let current = chars[index];
        let next = chars.get(index + 1).copied();
        if block_depth > 0 {
            if current == '/' && next == Some('*') {
                block_depth += 1;
                output.push_str("/*");
                index += 2;
            } else if current == '*' && next == Some('/') {
                block_depth -= 1;
                output.push_str("*/");
                index += 2;
            } else {
                output.push(current);
                index += 1;
            }
            continue;
        }
        if in_string {
            output.push(current);
            index += 1;
            if current == '\\' && index < chars.len() {
                output.push(chars[index]);
                index += 1;
            } else if current == '"' {
                in_string = false;
            }
            continue;
        }
        if current == '"' {
            in_string = true;
            output.push(current);
            index += 1;
        } else if current == '/' && next == Some('/') {
            while index < chars.len() && chars[index] != '\n' {
                output.push(chars[index]);
                index += 1;
            }
        } else if current == '/' && next == Some('*') {
            block_depth = 1;
            output.push_str("/*");
            index += 2;
        } else if current.is_alphabetic() || current == '_' {
            let start = index;
            index += 1;
            while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
                index += 1;
            }
            let identifier: String = chars[start..index].iter().collect();
            output.push_str(if identifier == old { new } else { &identifier });
        } else {
            output.push(current);
            index += 1;
        }
    }
    output
}
