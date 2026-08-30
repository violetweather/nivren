pub fn format(source: &str) -> String {
    let mut output = String::new();
    let mut indent = 0usize;
    let mut block_comment_depth = 0usize;

    for raw_line in canonical_lines(source) {
        let line = raw_line.trim();
        debug_assert!(!line.is_empty());
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

fn canonical_lines(source: &str) -> Vec<String> {
    let characters = source.chars().collect::<Vec<_>>();
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut index = 0;
    let mut pending_space = false;
    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    let mut data_blocks = Vec::new();
    let mut previous_word = String::new();

    while index < characters.len() {
        let character = characters[index];
        let next = characters.get(index + 1).copied();
        if character.is_whitespace() {
            if character == '\n'
                && parentheses == 0
                && brackets == 0
                && !data_blocks.last().copied().unwrap_or(false)
                && next_non_whitespace(&characters, index + 1) != Some('{')
            {
                flush_line(&mut lines, &mut current);
                previous_word.clear();
            }
            pending_space = !current.is_empty();
            index += 1;
            continue;
        }
        if character == '/' && next == Some('/') {
            push_space(&mut current, &mut pending_space);
            while index < characters.len() && characters[index] != '\n' {
                current.push(characters[index]);
                index += 1;
            }
            flush_line(&mut lines, &mut current);
            pending_space = false;
            previous_word.clear();
            continue;
        }
        if character == '/' && next == Some('*') {
            flush_line(&mut lines, &mut current);
            let mut comment = String::new();
            let mut depth = 0usize;
            while index < characters.len() {
                let character = characters[index];
                let next = characters.get(index + 1).copied();
                comment.push(character);
                if character == '/' && next == Some('*') {
                    depth += 1;
                    comment.push('*');
                    index += 2;
                    continue;
                }
                if character == '*' && next == Some('/') {
                    depth = depth.saturating_sub(1);
                    comment.push('/');
                    index += 2;
                    if depth == 0 {
                        break;
                    }
                    continue;
                }
                index += 1;
            }
            for line in comment.lines() {
                let line = line.trim();
                if !line.is_empty() {
                    lines.push(line.to_string());
                }
            }
            pending_space = false;
            previous_word.clear();
            continue;
        }
        if character == '"' {
            push_space(&mut current, &mut pending_space);
            current.push(character);
            index += 1;
            let mut escaped = false;
            while index < characters.len() {
                let character = characters[index];
                current.push(character);
                index += 1;
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == '"' {
                    break;
                }
            }
            previous_word.clear();
            continue;
        }
        if character.is_alphabetic() || character == '_' {
            let start = index;
            index += 1;
            while index < characters.len()
                && (characters[index].is_alphanumeric() || characters[index] == '_')
            {
                index += 1;
            }
            let word = characters[start..index].iter().collect::<String>();
            if should_break_before(&word, &previous_word, &current, parentheses, brackets) {
                flush_line(&mut lines, &mut current);
                pending_space = false;
            }
            push_space(&mut current, &mut pending_space);
            current.push_str(&word);
            previous_word = word;
            continue;
        }
        match character {
            '{' => {
                let data_block = is_data_block_header(&current);
                push_space(&mut current, &mut pending_space);
                current.push('{');
                flush_line(&mut lines, &mut current);
                data_blocks.push(data_block);
                previous_word.clear();
            }
            '}' => {
                flush_line(&mut lines, &mut current);
                current.push('}');
                data_blocks.pop();
                previous_word.clear();
                pending_space = false;
            }
            ';' => {
                trim_spaces(&mut current);
                current.push(';');
                flush_line(&mut lines, &mut current);
                previous_word.clear();
                pending_space = false;
            }
            '(' => {
                push_space(&mut current, &mut pending_space);
                current.push(character);
                parentheses += 1;
                previous_word.clear();
            }
            ')' => {
                current.push(character);
                parentheses = parentheses.saturating_sub(1);
                previous_word.clear();
            }
            '[' => {
                push_space(&mut current, &mut pending_space);
                current.push(character);
                brackets += 1;
                previous_word.clear();
            }
            ']' => {
                current.push(character);
                brackets = brackets.saturating_sub(1);
                previous_word.clear();
            }
            _ => {
                push_space(&mut current, &mut pending_space);
                current.push(character);
                previous_word.clear();
            }
        }
        index += 1;
    }
    flush_line(&mut lines, &mut current);
    lines
}

fn next_non_whitespace(characters: &[char], mut index: usize) -> Option<char> {
    while index < characters.len() {
        if !characters[index].is_whitespace() {
            return Some(characters[index]);
        }
        index += 1;
    }
    None
}

fn is_data_block_header(header: &str) -> bool {
    let header = header.trim();
    header.contains(" holds")
        || header.starts_with("takes")
        || header.starts_with("expose")
        || header.ends_with(" with")
}

fn should_break_before(
    word: &str,
    previous_word: &str,
    current: &str,
    parentheses: usize,
    brackets: usize,
) -> bool {
    if current.trim().is_empty() || parentheses > 0 || brackets > 0 {
        return false;
    }
    if current.trim_start().starts_with('}') && !matches!(word, "with" | "otherwise" | "or") {
        return true;
    }
    if matches!(word, "takes" | "gives" | "needs") {
        return true;
    }
    matches!(
        word,
        "keep"
            | "change"
            | "prepare"
            | "when"
            | "each"
            | "repeat"
            | "using"
            | "show"
            | "expose"
            | "define"
            | "shape"
            | "choice"
            | "case"
            | "protocol"
            | "adopt"
            | "import"
    ) || (word == "give" && previous_word != "or")
}

fn push_space(output: &mut String, pending_space: &mut bool) {
    if *pending_space && !output.is_empty() && !output.ends_with([' ', '.', '(', '[']) {
        output.push(' ');
    }
    *pending_space = false;
}

fn flush_line(lines: &mut Vec<String>, current: &mut String) {
    let line = current.trim();
    if !line.is_empty() {
        lines.push(line.to_string());
    }
    current.clear();
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
                if keyword_precedes_group(&output) {
                    output.push(' ');
                }
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
                let joins_previous_operator = output.ends_with(['<', '>', '!']);
                if !output.is_empty() && !joins_previous_operator {
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

/// Words after which a bracket or parenthesis begins a value rather than a
/// call or index, so the canonical form keeps one separating space:
/// `set [1, 2]`, `in [items]`, `repeat (not done)`. A preceding dot means the
/// word is a member name such as `std.map.set` and takes no space.
fn keyword_precedes_group(output: &str) -> bool {
    let prefix = output.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_');
    if prefix.ends_with('.') {
        return false;
    }
    matches!(
        &output[prefix.len()..],
        "set"
            | "to"
            | "in"
            | "within"
            | "is"
            | "gives"
            | "give"
            | "or"
            | "and"
            | "carries"
            | "maybe"
            | "when"
            | "while"
            | "repeat"
            | "through"
            | "together"
            | "race"
            | "perform"
            | "start"
            | "wait"
    )
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
