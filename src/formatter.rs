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
        let started_in_block_comment = block_comment_depth > 0;
        let (opens, closes, begins_with_close) = braces(line, &mut block_comment_depth);
        let line = if started_in_block_comment || line.starts_with("/*") {
            line.to_string()
        } else {
            normalize_line(line)
        };
        if begins_with_close {
            indent = indent.saturating_sub(1);
        }
        output.push_str(&"    ".repeat(indent));
        output.push_str(&line);
        output.push('\n');

        let accounted_close = usize::from(begins_with_close);
        indent = indent
            .saturating_add(opens)
            .saturating_sub(closes.saturating_sub(accounted_close));
    }
    output
}

fn normalize_line(line: &str) -> String {
    let (code, comment) = split_line_comment(line);
    let mut output = String::new();
    let mut chars = code.chars().peekable();
    let mut pending_space = false;
    let mut in_string = false;
    let mut escaped = false;
    while let Some(current) = chars.next() {
        if in_string {
            output.push(current);
            if escaped {
                escaped = false;
            } else if current == '\\' {
                escaped = true;
            } else if current == '"' {
                in_string = false;
            }
            continue;
        }
        if current == '"' {
            push_pending_space(&mut output, &mut pending_space);
            output.push(current);
            in_string = true;
            continue;
        }
        if current.is_whitespace() {
            pending_space = !output.is_empty();
            continue;
        }
        match current {
            ',' => {
                trim_spaces(&mut output);
                output.push(',');
                pending_space = true;
            }
            ';' => {
                trim_spaces(&mut output);
                output.push(';');
                pending_space = true;
            }
            '.' => {
                trim_spaces(&mut output);
                output.push('.');
                pending_space = false;
            }
            ':' => {
                trim_spaces(&mut output);
                output.push(':');
                pending_space = true;
            }
            '(' | '[' => {
                trim_spaces(&mut output);
                output.push(current);
                pending_space = false;
            }
            ')' | ']' => {
                trim_spaces(&mut output);
                output.push(current);
                pending_space = false;
            }
            '=' => {
                trim_spaces(&mut output);
                if !output.is_empty() {
                    output.push(' ');
                }
                output.push('=');
                if matches!(chars.peek(), Some('=' | '>')) {
                    output.push(chars.next().unwrap());
                }
                pending_space = true;
            }
            _ => {
                push_pending_space(&mut output, &mut pending_space);
                output.push(current);
            }
        }
    }
    trim_spaces(&mut output);
    if let Some(comment) = comment {
        if !output.is_empty() {
            output.push(' ');
        }
        output.push_str(comment);
    }
    output
}

fn split_line_comment(line: &str) -> (&str, Option<&str>) {
    let bytes = line.as_bytes();
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;
    while index + 1 < bytes.len() {
        let current = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if current == b'\\' {
                escaped = true;
            } else if current == b'"' {
                in_string = false;
            }
        } else if current == b'"' {
            in_string = true;
        } else if current == b'/' && bytes[index + 1] == b'/' {
            return (&line[..index], Some(&line[index..]));
        } else if current == b'/' && bytes[index + 1] == b'*' {
            return (line, None);
        }
        index += 1;
    }
    (line, None)
}

fn push_pending_space(output: &mut String, pending: &mut bool) {
    if *pending && !output.is_empty() && !output.ends_with([' ', '.', '(', '[']) {
        output.push(' ');
    }
    *pending = false;
}

fn trim_spaces(output: &mut String) {
    while output.ends_with(' ') {
        output.pop();
    }
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
