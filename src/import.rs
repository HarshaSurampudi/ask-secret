//! `import`: move plaintext .env files into the vault.
//!
//! A key with the same value in two or more projects becomes one `shared` entry that
//! each project links to. Output never includes values.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::{dotenv, project, vault::Vault, vault::SHARED};

const SKIP_DIRS: &[&str] = &["node_modules", ".git", "target", ".build", ".venv", "venv", "dist", "build", ".next"];

fn is_env_file(name: &str) -> bool {
    let templates = [".example", ".sample", ".template", ".dist", ".defaults"];
    (name == ".env" || name.starts_with(".env.")) && !templates.iter().any(|t| name.ends_with(t))
}

struct Found {
    file: PathBuf,
    project: String,
    project_path: String,
    values: BTreeMap<String, String>,
}

pub fn import(dir: &Path, depth: usize, apply: bool, delete: bool) -> Result<()> {
    let mut found = vec![];
    let walker = walkdir::WalkDir::new(dir).max_depth(depth).into_iter().filter_entry(|e| {
        !(e.file_type().is_dir() && SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
    });
    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() || !is_env_file(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
        let values = dotenv::parse(&text);
        if values.is_empty() {
            continue;
        }
        let p = project::detect(entry.path().parent().unwrap_or(dir));
        found.push(Found {
            file: entry.path().to_path_buf(),
            project: p.name,
            project_path: p.path,
            values,
        });
    }
    if found.is_empty() {
        println!("No .env files with values under {}.", dir.display());
        return Ok(());
    }

    // Which (key, value) pairs appear in more than one project.
    let mut seen: BTreeMap<(&str, &str), BTreeSet<&str>> = BTreeMap::new();
    for f in &found {
        for (k, v) in &f.values {
            seen.entry((k, v)).or_default().insert(&f.project);
        }
    }
    let shared_pairs: BTreeSet<(&str, &str)> =
        seen.iter().filter(|(_, projects)| projects.len() > 1).map(|(pair, _)| *pair).collect();

    let mut vault = Vault::open()?;
    let mut conflicts = 0;
    let mut per_project: BTreeMap<String, (String, usize, usize)> = BTreeMap::new();
    for f in &found {
        vault.ensure_project(&f.project, Some(&f.project_path), true);
        let stats = per_project.entry(f.project.clone()).or_insert((f.project.clone(), 0, 0));
        for (k, v) in &f.values {
            let share = shared_pairs.contains(&(k.as_str(), v.as_str()))
                && vault.get(SHARED, k).map_or(true, |existing| existing == v);
            if share {
                vault.set(SHARED, k, v);
                vault.link(&f.project, k);
                stats.2 += 1;
            } else if vault.entry(&f.project, k).is_some() && vault.get(&f.project, k) != Some(v.as_str()) {
                // Two files in one project disagree (say .env and .env.local); keep the first.
                conflicts += 1;
            } else {
                vault.set(&f.project, k, v);
                stats.1 += 1;
            }
        }
    }

    println!("Found {} .env file(s) in {} project(s):", found.len(), per_project.len());
    for (name, own, linked) in per_project.values() {
        println!("  {name}: {own} key(s) of its own, {linked} linked to shared");
    }
    let shared_keys: BTreeSet<&str> = shared_pairs.iter().map(|(k, _)| *k).collect();
    if !shared_keys.is_empty() {
        println!("Shared (same value in several projects): {}", shared_keys.into_iter().collect::<Vec<_>>().join(", "));
    }
    if conflicts > 0 {
        println!("{conflicts} key(s) had different values in the same project; the first one was kept.");
    }
    if !apply {
        println!("\nDry run, nothing saved. Run again with --yes to import (add --delete to remove the files after).");
        return Ok(());
    }
    vault.save()?;
    println!("\nImported into the vault.");
    if delete {
        for f in &found {
            std::fs::remove_file(&f.file)?;
            println!("  deleted {}", f.file.display());
        }
    } else {
        println!("The .env files are still there; use `ask-secret run -- <cmd>` and delete them when ready.");
    }
    Ok(())
}
