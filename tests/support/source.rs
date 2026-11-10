//! Reading the crate's own sources, for the tests that guard production code (the literal
//! check, the globals check). Tests may hold anything verbatim, so they look only at the lines
//! outside `#[cfg(test)]` items and outside comments.

use std::path::{Path, PathBuf};

/// Every `.rs` file under `src/`, as `(path with forward slashes, text)`.
pub fn production_sources() -> Vec<(String, String)> {
    let mut files = Vec::new();
    sources(Path::new("src"), &mut files);
    assert!(files.len() > 20, "found the sources");
    files.sort();
    files
        .into_iter()
        .map(|file| {
            let text = std::fs::read_to_string(&file).unwrap();
            (file.to_string_lossy().replace('\\', "/"), text)
        })
        .collect()
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The production lines of a source file: `#[cfg(test)]` items (a module, a function, a
/// constant) and comment lines are left out. `(line number, text)`.
pub fn production_lines(text: &str) -> Vec<(usize, &str)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim_start().starts_with("#[cfg(test)]") {
            // Skip the item: to its closing brace, or to the `;` of a one-line item.
            let (mut depth, mut opened) = (0i32, false);
            let mut j = i + 1;
            while j < lines.len() {
                let line = lines[j];
                depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
                opened |= line.contains('{');
                if (opened && depth <= 0) || (!opened && line.trim_end().ends_with(';')) {
                    break;
                }
                j += 1;
            }
            i = j + 1;
            continue;
        }
        if !lines[i].trim_start().starts_with("//") {
            out.push((i + 1, lines[i]));
        }
        i += 1;
    }
    out
}
