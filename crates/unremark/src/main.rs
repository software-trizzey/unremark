use clap::Parser as ClapParser;
use dotenv::dotenv;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use walkdir::WalkDir;
use log::{debug, info};
use env_logger;
use parking_lot;
use std::time::Instant;
use futures::future::join_all;
use tokio;

// Import from our library
use unremark::{
    Language,
    AnalysisResult,
    Cache,
    analyze_file,
};

#[derive(ClapParser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Remove redundant comments
    #[arg(long, default_value_t = false)]
    fix: bool,

    /// Ignore specific directories (comma-separated)
    #[arg(long, default_value = "venv,node_modules,.git,__pycache__")]
    ignore: String,

    /// Output results in JSON format
    #[arg(long, default_value_t = false)]
    json: bool,
}

// JSON output types for CLI
#[derive(serde::Serialize)]
struct JsonFileResult {
    path: String,
    redundant_comments: Vec<JsonCommentInfo>,
    errors: Vec<String>,
}

#[derive(serde::Serialize)]
struct JsonCommentInfo {
    text: String,
    line_number: usize,
    context: String,
}

#[derive(serde::Serialize)]
struct JsonOutput {
    total_files: usize,
    files_with_comments: usize,
    files_with_errors: usize,
    total_redundant_comments: usize,
    files: Vec<JsonFileResult>,
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    env_logger::init();
    let args = Args::parse();
    let ignore_dirs: Vec<&str> = args.ignore.split(',').collect();

    let start_time = Instant::now();
    info!("Analyzing files in: {}", args.path.display());

    let source_files: Vec<PathBuf> = WalkDir::new(&args.path)
        .into_iter()
        .filter_entry(|e| !is_ignored(e, &ignore_dirs))
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension()
                .and_then(|ext| ext.to_str())
                .and_then(Language::from_extension)
                .is_some()
        })
        .map(|e| e.path().to_owned())
        .collect();

    let total_files = source_files.len();
    debug!("Found {} files to analyze", total_files);

    let processed_files = Arc::new(AtomicUsize::new(0));
    let cache = Arc::new(parking_lot::RwLock::new(Cache::load()));
    
    // Create a vector of futures for each file analysis
    let futures: Vec<_> = source_files.iter()
        .map(|file| {
            let cache = Arc::clone(&cache);
            let processed_files = Arc::clone(&processed_files);
            let total_files = total_files;
            async move {
                let result = analyze_file(file, args.fix, &cache).await;
                let current = processed_files.fetch_add(1, Ordering::SeqCst) + 1;
                info!("Progress: [{}/{}] {}", current, total_files, file.display());
                result
            }
        })
        .collect();

    // Use tokio's join_all to run the futures concurrently
    let results = join_all(futures).await;

    // Save the cache after all processing
    cache.write().save();

    let duration = start_time.elapsed();
    info!("Analysis completed in {:.2} seconds", duration.as_secs_f64());

    print_summary(&results, args.json);
}

fn print_summary(results: &[AnalysisResult], json_output: bool) {
    if json_output {
        let json_results: Vec<JsonFileResult> = results.iter().map(|r| JsonFileResult {
            path: r.path.display().to_string(),
            redundant_comments: r.redundant_comments.iter().map(|c| JsonCommentInfo {
                text: c.text.clone(),
                line_number: c.line_number,
                context: c.context.clone(),
            }).collect(),
            errors: r.errors.clone(),
        }).collect();

        let files_with_comments = json_results.iter()
            .filter(|r| !r.redundant_comments.is_empty())
            .count();

        let files_with_errors = json_results.iter()
            .filter(|r| !r.errors.is_empty())
            .count();

        let total_redundant_comments = json_results.iter()
            .map(|r| r.redundant_comments.len())
            .sum();

        let output = JsonOutput {
            total_files: results.len(),
            files_with_comments,
            files_with_errors,
            total_redundant_comments,
            files: json_results,
        };

        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    } else {
        let files_with_comments = results.iter()
            .filter(|r| !r.redundant_comments.is_empty())
            .count();

        let total_redundant_comments: usize = results.iter()
            .map(|r| r.redundant_comments.len())
            .sum();

        println!("\nSummary:");
        println!("Total files analyzed: {}", results.len());
        println!("Files with redundant comments: {}", files_with_comments);
        println!("Total redundant comments found: {}", total_redundant_comments);

        if total_redundant_comments > 0 {
            println!("\nFiles with redundant comments:");
            for result in results.iter().filter(|r| !r.redundant_comments.is_empty()) {
                println!("\n{}", result.path.display());
                for comment in &result.redundant_comments {
                    println!("  Line {}: {}", comment.line_number, comment.text.trim());
                }
            }
        }
    }
}

fn is_ignored(entry: &walkdir::DirEntry, ignore_dirs: &[&str]) -> bool {
    entry.file_type().is_dir() && ignore_dirs.iter().any(|dir| {
        entry.file_name()
            .to_str()
            .map(|s| s == *dir)
            .unwrap_or(false)
    })
}