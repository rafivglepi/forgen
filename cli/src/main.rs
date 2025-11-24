mod analysis;
mod models;
mod workspace;

use anyhow::{Context, Result};
use clap::Parser;
use notify_debouncer_mini::{new_debouncer, notify::*};
use ra_ap_paths::AbsPathBuf;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::time::Duration;

/// Forgen - Enhanced compile-time macro information with type awareness
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to Cargo.toml (defaults to ./Cargo.toml in current directory)
    #[arg(value_name = "MANIFEST")]
    manifest: Option<PathBuf>,

    /// Watch for file changes and re-analyze (development mode)
    #[arg(short, long)]
    watch: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    println!("🚀 Forgen - Enhanced Macro Compiler Info");
    println!("=========================================\n");

    // Default to current directory's Cargo.toml if not specified
    let manifest_path = args.manifest.unwrap_or_else(|| PathBuf::from("Cargo.toml"));

    println!("📦 Loading project: {}", manifest_path.display());

    // Convert to absolute path
    let manifest_path_abs = manifest_path.canonicalize()?;
    let manifest_path_str = manifest_path_abs
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Path is not valid UTF-8"))?;
    let manifest_path = AbsPathBuf::try_from(manifest_path_str)
        .map_err(|e| anyhow::anyhow!("Invalid path: {:?}", e))?;

    // Load the workspace
    let (mut host, mut vfs) = workspace::load_workspace(&manifest_path)?;

    // Get the project directory for watching
    let project_dir = manifest_path_abs.parent().unwrap().to_path_buf();

    if args.watch {
        println!("👀 Watch mode enabled - monitoring for changes...\n");
        println!("Press Ctrl+C to stop\n");

        // Initial analysis
        analyze_and_save(&host, &vfs, &project_dir, &manifest_path_abs)?;

        // Setup file watcher
        let (tx, rx) = channel();
        let mut debouncer = new_debouncer(Duration::from_millis(500), tx)?;

        // Watch the src directory
        let src_path = project_dir.join("src");
        debouncer
            .watcher()
            .watch(&src_path, RecursiveMode::Recursive)
            .with_context(|| format!("Failed to watch {:?}", src_path))?;

        println!("📁 Watching: {}\n", src_path.display());

        // Watch loop
        loop {
            match rx.recv() {
                Ok(Ok(events)) => {
                    // Check if any Rust files changed
                    let changed_files: Vec<_> = events
                        .iter()
                        .filter(|e| e.path.extension().and_then(|s| s.to_str()) == Some("rs"))
                        .map(|e| e.path.clone())
                        .collect();

                    if !changed_files.is_empty() {
                        println!("🔄 File change detected, re-analyzing...");

                        // Apply file changes incrementally (fast!)
                        match workspace::apply_file_changes(&mut host, &mut vfs, &changed_files) {
                            Ok(_) => {
                                // Re-analyze with the updated database
                                match analyze_and_save(
                                    &host,
                                    &vfs,
                                    &project_dir,
                                    &manifest_path_abs,
                                ) {
                                    Ok(_) => println!("✅ Re-analysis complete\n"),
                                    Err(e) => eprintln!("❌ Error during re-analysis: {}\n", e),
                                }
                            },
                            Err(e) => eprintln!("❌ Error applying file changes: {}\n", e),
                        }
                    }
                },
                Ok(Err(e)) => eprintln!("Watch error: {:?}", e),
                Err(e) => {
                    eprintln!("Channel error: {:?}", e);
                    break;
                },
            }
        }
    } else {
        // Single run mode
        analyze_and_save(&host, &vfs, &project_dir, &manifest_path_abs)?;
        println!("\n✨ Analysis complete!");
    }

    Ok(())
}

fn analyze_and_save(
    db: &ra_ap_ide_db::RootDatabase,
    vfs: &ra_ap_vfs::Vfs,
    project_dir: &PathBuf,
    manifest_path: &PathBuf,
) -> Result<()> {
    // Extract type information per file
    let (file_infos, crate_metadata) =
        analysis::extract_type_info_by_file(db, vfs, project_dir, manifest_path)?;

    // Create output directory
    let output_dir = project_dir.join("target");
    fs::create_dir_all(&output_dir)?;

    // Convert the map into a vec of FileTypeInfo
    let files: Vec<models::FileTypeInfo> = file_infos.into_values().collect();

    // Create the complete output structure
    let output = models::ForgenOutput {
        crates: crate_metadata,
        files,
    };

    // Save everything to a single .forgen.json file (minified)
    let output_file = output_dir.join(".forgen.json");
    let json = serde_json::to_string(&output)?;
    fs::write(&output_file, json)?;

    println!("💾 Saved type information to: {}", output_file.display());
    println!("✨ Analyzed {} files", output.files.len());

    Ok(())
}
