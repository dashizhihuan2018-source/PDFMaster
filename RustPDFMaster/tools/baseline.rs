//! Read-only XCTest contract inventory; does not copy source or user data.
use std::{collections::BTreeSet, fs, path::Path};
fn names(source: &str) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    for line in source.lines() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        if let Some(rest) = line.split("func ").nth(1) {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.starts_with("test") && rest[name.len()..].trim_start().starts_with('(') {
                result.insert(name);
            }
        }
    }
    result
}
fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
fn collect(root: &Path, dir: &Path, rows: &mut Vec<(String, String)>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_symlink() {
            continue;
        }
        let path = entry.path();
        if ty.is_dir() {
            collect(root, &path, rows)?;
        } else if path.extension().is_some_and(|x| x == "swift") {
            let source = fs::read_to_string(&path)?;
            let rel = path
                .strip_prefix(root)
                .expect("descendant")
                .to_string_lossy()
                .replace('\\', "/");
            for name in names(&source) {
                rows.push((rel.clone(), name));
            }
        }
    }
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = std::env::args()
        .nth(1)
        .ok_or("usage: baseline <legacy-project>")?;
    let root = Path::new(&arg).canonicalize()?;
    let mut rows = Vec::new();
    collect(&root, &root.join("Tests"), &mut rows)?;
    rows.sort();
    if rows.is_empty() {
        return Err("no XCTest baseline; refusing empty contract".into());
    }
    let mut ids = Vec::new();
    let mut items = Vec::new();
    for (path, name) in &rows {
        let id = format!(
            "{}::{name}",
            path.trim_start_matches("Tests/").trim_end_matches(".swift")
        );
        ids.push(quote(&id));
        items.push(format!("{{\"id\":{},\"description\":{},\"legacy_source\":{},\"macos\":{{\"status\":\"pending\",\"evidence\":[]}},\"windows\":{{\"status\":\"pending\",\"evidence\":[]}},\"ios\":{{\"status\":\"pending\",\"evidence\":[]}},\"human_review\":{{\"macos\":\"pending\",\"windows\":\"pending\",\"ios\":\"pending\"}}}}",quote(&id),quote(name),quote(path)));
    }
    println!(
        "{{\"schema_version\":1,\"version\":\"2.0.0-dev\",\"scope\":\"XCTest contracts only: visible UX and untested features need separate inventory; pending is no Rust parity proof.\",\"baseline_features\":[{}],\"features\":[{}]}}",
        ids.join(","),
        items.join(",\n")
    );
    eprintln!(
        "{} XCTest contracts inventoried; all platform states pending",
        rows.len()
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracts_test_declarations() {
        assert_eq!(
            names(" func testOpen() throws {}\nfunc helper() {}\nfunc testSave () {}"),
            BTreeSet::from(["testOpen".into(), "testSave".into()])
        );
    }
    #[test]
    fn skips_line_comments_and_nonfunctions() {
        assert!(names("// func testOld() {}\nlet test = 1\nfunc testValue: Int").is_empty());
    }
    #[test]
    fn escapes_json_paths() {
        assert_eq!(quote("a\\b\"c\n"), "\"a\\\\b\\\"c\\n\"");
    }
}
