//! pfcli — test/dev harness for photoforge-core (no Tauri needed).
//!
//! Usage:
//!     pfcli index <directory> [--db <path>]
//!     pfcli dupes [--db <path>]
//!     pfcli near-dupes [--db <path>] [--k <n>] [--method naive|bktree]
//!     pfcli organize <target_root> [--db <path>] [--apply] [--copy]
//!     pfcli classify [--db <path>]
//!     pfcli accuracy <labels.csv> [--db <path>]
//!     pfcli undo [--count <n>] [--db <path>]
//!
//! `organize` defaults to dry-run (prints the plan); pass --apply to execute.
//! The stopwatch prints wall time for every command (for quick perf checks).

use photoforge_core::dedupe::{self, NearMethod};
use photoforge_core::{classify, db, organize};
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str = "usage: pfcli <index|dupes|near-dupes|organize|classify|accuracy|undo> [args]
    index <directory> [--db <path>]
    dupes [--db <path>]
    near-dupes [--db <path>] [--k <n>] [--method naive|bktree]
    organize <target_root> [--db <path>] [--apply] [--copy]
    classify [--db <path>]
    accuracy <labels.csv> [--db <path>]
    undo [--count <n>] [--db <path>]";

/// Hand-rolled flag parsing (keeping the dependency tree lean — no clap).
struct Args {
    positional: Vec<String>,
    flags: std::collections::HashMap<String, String>,
}

fn parse_args(raw: Vec<String>) -> Args {
    let mut positional = Vec::new();
    let mut flags = std::collections::HashMap::new();
    let mut it = raw.into_iter().peekable();
    while let Some(a) = it.next() {
        if let Some(name) = a.strip_prefix("--") {
            let value = match it.peek() {
                Some(v) if !v.starts_with("--") => it.next().unwrap_or_default(),
                _ => "true".to_string(),
            };
            flags.insert(name.to_string(), value);
        } else {
            positional.push(a);
        }
    }
    Args { positional, flags }
}

fn print_groups(groups: &[dedupe::DupeGroup], kind: &str) {
    let wasted: i64 = groups.iter().map(|g| g.wasted_bytes).sum();
    println!(
        "{} {kind} duplicate group(s), {:.1} MB reclaimable",
        groups.len(),
        wasted as f64 / 1_048_576.0
    );
    for (i, g) in groups.iter().enumerate() {
        println!(
            "group {} — {} files, {:.1} MB wasted:",
            i + 1,
            g.files.len(),
            g.wasted_bytes as f64 / 1_048_576.0
        );
        for f in &g.files {
            println!("    {:>10} B  {}", f.size, f.path);
        }
    }
}

fn run() -> Result<(), String> {
    let mut raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.is_empty() {
        return Err(USAGE.to_string());
    }
    let cmd = raw.remove(0);
    let args = parse_args(raw);
    let db_path = args
        .flags
        .get("db")
        .cloned()
        .unwrap_or_else(|| "photoforge.db".to_string());
    let mut conn = db::open(&db_path).map_err(|e| format!("open catalog {db_path}: {e}"))?;

    let started = Instant::now();
    match cmd.as_str() {
        "index" => {
            let root = args.positional.first().ok_or(USAGE)?;
            let stats =
                photoforge_core::index_directory(&mut conn, root).map_err(|e| e.to_string())?;
            println!(
                "indexed {root} -> {db_path}\n  scanned {} | added {} | skipped {} | errors {}",
                stats.scanned, stats.added, stats.skipped, stats.errors
            );
        }
        "dupes" => {
            let groups = dedupe::find_exact_duplicates(&conn).map_err(|e| e.to_string())?;
            print_groups(&groups, "exact");
        }
        "near-dupes" => {
            let k: u32 = args
                .flags
                .get("k")
                .map(|s| s.parse())
                .transpose()
                .map_err(|_| "--k must be a number")?
                .unwrap_or(5);
            // Naive is the default: benchmarks show it beats the BK-tree on
            // 64-bit dHashes at any realistic library size (see dedupe.rs docs).
            let method = match args.flags.get("method").map(String::as_str) {
                Some("bktree") => NearMethod::BkTree,
                _ => NearMethod::Naive,
            };
            let groups =
                dedupe::find_near_duplicates(&conn, k, method).map_err(|e| e.to_string())?;
            print_groups(&groups, &format!("near (k={k})"));
        }
        "organize" => {
            let target = args.positional.first().ok_or(USAGE)?;
            let plan = organize::plan_organize(&conn, target).map_err(|e| e.to_string())?;
            if args.flags.contains_key("apply") {
                let mode = if args.flags.contains_key("copy") {
                    organize::OrganizeMode::Copy
                } else {
                    organize::OrganizeMode::Move
                };
                let stats =
                    organize::apply_organize(&mut conn, &plan, mode).map_err(|e| e.to_string())?;
                println!("applied: {} moved/copied, {} errors", stats.moved, stats.errors);
            } else {
                for p in &plan {
                    println!("{}  ->  {}", p.src, p.dst);
                }
                println!("dry run: {} file(s) would move. Pass --apply to execute.", plan.len());
            }
        }
        "classify" => {
            let stats = classify::classify_all(&mut conn).map_err(|e| e.to_string())?;
            println!(
                "classified: {} photo | {} non_photo | {} ambiguous",
                stats.photo, stats.non_photo, stats.ambiguous
            );
        }
        "accuracy" => {
            let csv = args.positional.first().ok_or(USAGE)?;
            let rep = classify::accuracy_report(&conn, csv).map_err(|e| e.to_string())?;
            println!(
                "{} labeled | {} correct | {} wrong | {} undecided | {} missing | accuracy {:.1}%",
                rep.total,
                rep.correct,
                rep.wrong,
                rep.undecided,
                rep.missing,
                rep.accuracy * 100.0
            );
        }
        "undo" => {
            let count: usize = args
                .flags
                .get("count")
                .map(|s| s.parse())
                .transpose()
                .map_err(|_| "--count must be a number")?
                .unwrap_or(1);
            let stats = organize::undo_last(&mut conn, count).map_err(|e| e.to_string())?;
            println!("undo: {} restored, {} errors", stats.moved, stats.errors);
        }
        _ => return Err(USAGE.to_string()),
    }
    println!("({:.2?})", started.elapsed());
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}
