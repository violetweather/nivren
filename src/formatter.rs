pub fn format(source: &str) -> String {
    let mut output = String::new();
    let mut indent = 0usize;
    let mut block_comment_depth = 0usize;
    let mut previous_blank = false;

    for raw_line in source.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            if !output.is_empty() && !previous_blank {
                output.push('\n');
            }
            previous_blank = true;
            continue;
        }
        previous_blank = false;
        let (opens, closes, begins_with_close) = braces(line, &mut block_comment_depth);
        if begins_with_close {
            indent = indent.saturating_sub(1);
        }
        output.push_str(&"    ".repeat(indent));
        output.push_str(line);
        output.push('\n');

        let accounted_close = usize::from(begins_with_close);
        indent = indent
            .saturating_add(opens)
            .saturating_sub(closes.saturating_sub(accounted_close));
    }
    output
}

fn braces(line: &str, block_depth: &mut usize) -> (usize, usize, bool) {
    let chars: Vec<char> = line.chars().collect();
    let mut index = 0;
    let mut opens = 0;
    let mut closes = 0;
    let mut in_string = false;
    let mut escaped = false;
    let mut first_code = None;
    while index < chars.len() {
        let current = chars[index];
        let next = chars.get(index + 1).copied();
        if *block_depth > 0 {
            if current == '/' && next == Some('*') {
                *block_depth += 1;
                index += 2;
                continue;
            }
            if current == '*' && next == Some('/') {
                *block_depth -= 1;
                index += 2;
                continue;
            }
            index += 1;
            continue;
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if current == '\\' {
                escaped = true;
            } else if current == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if current == '/' && next == Some('/') {
            break;
        }
        if current == '/' && next == Some('*') {
            *block_depth = 1;
            index += 2;
            continue;
        }
        if current == '"' {
            first_code.get_or_insert(current);
            in_string = true;
        } else if !current.is_whitespace() {
            first_code.get_or_insert(current);
            if current == '{' {
                opens += 1;
            } else if current == '}' {
                closes += 1;
            }
        }
        index += 1;
    }
    (opens, closes, first_code == Some('}'))
}
