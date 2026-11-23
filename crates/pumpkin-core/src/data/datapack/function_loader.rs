use std::collections::HashMap;
use std::fs;
use std::path::Path;

use tracing::warn;

/// Longest command line a function may contain, from vanilla's `CommandFunction.checkCommandLineLength`.
const MAX_COMMAND_LINE_LENGTH: usize = 2_000_000;

pub fn load_functions_from_dir<S: std::hash::BuildHasher>(
    namespace: &str,
    function_dir: &Path,
    functions: &mut HashMap<String, Vec<String>, S>,
) {
    if !function_dir.is_dir() {
        return;
    }
    load_functions_recursive(namespace, function_dir, function_dir, functions);
}

fn load_functions_recursive<S: std::hash::BuildHasher>(
    namespace: &str,
    base_dir: &Path,
    current_dir: &Path,
    functions: &mut HashMap<String, Vec<String>, S>,
) {
    let Ok(entries) = fs::read_dir(current_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            load_functions_recursive(namespace, base_dir, &path, functions);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mcfunction"))
            && let Ok(rel_path) = path.strip_prefix(base_dir)
        {
            let mut stem_path = rel_path.to_string_lossy().to_string();
            if let Some(s) = stem_path.strip_suffix(".mcfunction") {
                stem_path = s.to_string();
            }
            // Convert Windows backslashes to forward slashes if any
            let stem_path = stem_path.replace('\\', "/");
            let function_id = format!("{namespace}:{stem_path}");
            if let Ok(content) = fs::read_to_string(&path) {
                match parse_function_lines(&content) {
                    Ok(lines) => {
                        functions.insert(function_id, lines);
                    }
                    Err(e) => warn!("Failed to load function {function_id}: {e}"),
                }
            }
        }
    }
}

/// Splits a `.mcfunction` file into the command lines to run, like vanilla's
/// `CommandFunction.fromLines`: a line ending in `\` continues on the next line.
fn parse_function_lines(content: &str) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    let mut input = content.lines();
    while let Some(input_line) = input.next() {
        let mut line = input_line.trim().to_string();
        // Joining happens before comments are skipped, so a comment ending in `\`
        // swallows the next line too.
        while line.ends_with('\\') {
            let Some(next) = input.next() else {
                return Err("Line continuation at end of file".to_string());
            };
            line.pop();
            line.push_str(next.trim());
            check_command_line_length(&line)?;
        }
        check_command_line_length(&line)?;

        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match line.strip_prefix('/') {
            Some(command) => lines.push(command.to_string()),
            None => lines.push(line),
        }
    }
    Ok(lines)
}

fn check_command_line_length(line: &str) -> Result<(), String> {
    if line.len() > MAX_COMMAND_LINE_LENGTH {
        return Err(format!("Command too long: {} characters", line.len()));
    }
    Ok(())
}

pub fn load_function_tags_from_dir<S: std::hash::BuildHasher>(
    namespace: &str,
    tags_dir: &Path,
    function_tags: &mut HashMap<String, Vec<String>, S>,
) {
    let function_tags_dir = tags_dir.join("function");
    let function_tags_dir_plural = tags_dir.join("functions");
    for dir in [&function_tags_dir, &function_tags_dir_plural] {
        if dir.is_dir() {
            load_tags_recursive(namespace, dir, dir, function_tags);
        }
    }
}

fn load_tags_recursive<S: std::hash::BuildHasher>(
    namespace: &str,
    base_dir: &Path,
    current_dir: &Path,
    tags: &mut HashMap<String, Vec<String>, S>,
) {
    let Ok(entries) = fs::read_dir(current_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            load_tags_recursive(namespace, base_dir, &path, tags);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            && let Ok(rel_path) = path.strip_prefix(base_dir)
        {
            let mut stem_path = rel_path.to_string_lossy().to_string();
            if let Some(s) = stem_path.strip_suffix(".json") {
                stem_path = s.to_string();
            }
            let stem_path = stem_path.replace('\\', "/");
            let tag_id = format!("{namespace}:{stem_path}");
            if let Ok(content) = fs::read_to_string(&path)
                && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
                && let Some(values_arr) = val.get("values").and_then(serde_json::Value::as_array)
            {
                let list: Vec<String> = values_arr
                    .iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect();
                tags.entry(tag_id).or_default().extend(list);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_function_lines;

    #[test]
    fn parse_mcfunction_content() {
        let content = "# This is a comment\nsay Hello world\n/give @p diamond 1\n\n# Another comment\ngive @p stick 5\n";
        let lines = parse_function_lines(content).expect("parse function");

        assert_eq!(
            lines,
            vec![
                "say Hello world".to_string(),
                "give @p diamond 1".to_string(),
                "give @p stick 5".to_string()
            ]
        );
    }

    #[test]
    fn parse_line_continuations() {
        let content = "execute if score a b matches 1 \\\n    run say one\nsay \\\n  two \\\n three\n# comment \\\nsay hidden\nsay last\n";
        let lines = parse_function_lines(content).expect("parse function");

        assert_eq!(
            lines,
            vec![
                "execute if score a b matches 1 run say one".to_string(),
                "say two three".to_string(),
                "say last".to_string()
            ]
        );
        assert_eq!(
            parse_function_lines("say one\nsay \\\n").expect_err("continuation at end of file"),
            "Line continuation at end of file"
        );
    }
}
