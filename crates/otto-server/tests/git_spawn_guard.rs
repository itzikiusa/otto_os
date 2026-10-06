//! S2-11 guard: daemon code outside `otto-git` never spawns a bare `git`.
//! A bare `Command::new("git")` skips the hardening every daemon git needs
//! (repo config must not pick a program for the unconfined daemon to run:
//! fsmonitor, hooks) — use `otto_git::LocalGit`, or
//! `otto_git::hardened_command()` / `hardened_std_command()` where LocalGit
//! doesn't fit. Test modules (everything from a file's first `#[cfg(test)]`)
//! are exempt: they build fixture repos.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The file's lines up to its first inline `#[cfg(test)] mod … {` (a
/// `#[cfg(test)] mod x;` declaration does not end production code).
fn production_lines(text: &str) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    for (i, l) in lines.iter().enumerate() {
        if l.trim() != "#[cfg(test)]" {
            continue;
        }
        let next = lines[i + 1..]
            .iter()
            .find(|n| !n.trim_start().starts_with("#["));
        if next.is_some_and(|n| n.starts_with("mod ") && n.trim_end().ends_with('{')) {
            return lines[..i].to_vec();
        }
    }
    lines
}

#[test]
fn no_bare_git_spawns() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    for c in std::fs::read_dir(crates).unwrap().flatten() {
        if c.file_name() == "otto-git" {
            continue;
        }
        rust_files(&c.path().join("src"), &mut files);
    }
    assert!(files.len() > 50, "scanned too few files: {}", files.len());
    let mut hits = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        for (i, line) in production_lines(&text).iter().enumerate() {
            if line.contains("Command::new(\"git\")") {
                hits.push(format!("{}:{}", f.display(), i + 1));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "bare git spawns outside otto-git (use otto_git::hardened_command / LocalGit):\n{}",
        hits.join("\n")
    );
}
