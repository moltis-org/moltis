#![allow(clippy::unwrap_used, clippy::expect_used)]
use {
    moltis_code_index::store::CodeIndexStore,
    std::{path::Path, sync::OnceLock},
};

fn main() {
    divan::main();
}

// ── Benchmark Configuration ─────────────────────────────────────────────────────

const MOLTIS_PROJECT_PATH: &str = "/mnt/fast_data/workspaces/moltis";
const PROJECT_ID: &str = "moltis";

const BENCHMARK_QUERIES: &[&str] = &[
    "password hashing and verification",
    "JWT token generation and validation",
    "database connection and transaction handling",
    "HTTP server initialization and routing",
    "configuration parsing and validation",
    "agent tool execution and error handling",
    "session storage and management",
    "embedding generation and vector operations",
    "WebSocket connection and event handling",
    "code indexing and search functionality",
];

// ── Shared async runtime (for search benchmarks) ────────────────────────────────

static TOKIO_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn tokio_rt() -> &'static tokio::runtime::Runtime {
    TOKIO_RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
    })
}

// ── Shared search index (built once) ────────────────────────────────────────────

struct SearchFixture {
    store: moltis_code_index::store_sqlite::SqliteCodeIndexStore,
    // Leak the tempdir so the DB file persists for the benchmark lifetime.
    _db_dir: tempfile::TempDir,
}

static SEARCH_FIXTURE: OnceLock<SearchFixture> = OnceLock::new();

fn search_fixture() -> &'static SearchFixture {
    SEARCH_FIXTURE.get_or_init(|| {
        let rt = tokio_rt();
        rt.block_on(async {
            let db_dir = tempfile::tempdir().unwrap();
            let db_path = db_dir.path().join("bench_code_index.db");
            let store = moltis_code_index::store_sqlite::SqliteCodeIndexStore::new(&db_path)
                .await
                .unwrap();

            store.initialize().await.unwrap();

            // Discover and filter files
            let project_dir = Path::new(MOLTIS_PROJECT_PATH);
            let tracked = moltis_code_index::discover::discover_tracked_files(project_dir).unwrap();
            let config = moltis_code_index::CodeIndexConfig::default();
            let filtered =
                moltis_code_index::filter::filter_tracked_files(project_dir, &tracked, &config)
                    .unwrap();

            // Chunk and index each file
            let chunker = moltis_code_index::chunker::CodeChunker::new(config.chunker());
            let mut file_count = 0usize;
            let mut chunk_count = 0usize;

            for file in &filtered {
                let content = tokio::fs::read_to_string(&file.path)
                    .await
                    .unwrap_or_default();
                let chunks = chunker.chunk(&content, &file.relative_path.display().to_string());

                if !chunks.is_empty() {
                    store
                        .upsert_chunks(
                            PROJECT_ID,
                            &file.relative_path.display().to_string(),
                            &chunks,
                        )
                        .await
                        .unwrap();
                    file_count += 1;
                    chunk_count += chunks.len();
                }
            }

            eprintln!(
                "[search fixture] indexed {file_count} files, {chunk_count} chunks into {}",
                db_path.display()
            );

            SearchFixture {
                store,
                _db_dir: db_dir,
            }
        })
    })
}

// ── File Discovery Benchmarks ──────────────────────────────────────────────────

/// Benchmark discovering git-tracked files in the moltis repo.
#[divan::bench]
fn discover_tracked_files() {
    let files = moltis_code_index::discover::discover_tracked_files(Path::new(MOLTIS_PROJECT_PATH));
    divan::black_box(files.unwrap());
}

/// Benchmark file discovery with default config.
#[divan::bench]
fn discover_and_filter_default_config() {
    let project_dir = Path::new(MOLTIS_PROJECT_PATH);

    let tracked = moltis_code_index::discover::discover_tracked_files(project_dir).unwrap();
    let config = moltis_code_index::CodeIndexConfig::default();
    let filtered =
        moltis_code_index::filter::filter_tracked_files(project_dir, &tracked, &config).unwrap();

    divan::black_box(filtered.len());
}

/// Benchmark file discovery with Rust-only extensions.
#[divan::bench]
fn discover_and_filter_rust_only() {
    let project_dir = Path::new(MOLTIS_PROJECT_PATH);

    let tracked = moltis_code_index::discover::discover_tracked_files(project_dir).unwrap();
    let mut config = moltis_code_index::CodeIndexConfig::default();
    config.extensions = vec!["rs".to_string()];
    let filtered =
        moltis_code_index::filter::filter_tracked_files(project_dir, &tracked, &config).unwrap();

    divan::black_box(filtered.len());
}

// ── Content Hashing Benchmarks ─────────────────────────────────────────────────

fn get_filtered_files() -> Vec<moltis_code_index::types::FilteredFile> {
    let project_dir = Path::new(MOLTIS_PROJECT_PATH);
    let tracked = moltis_code_index::discover::discover_tracked_files(project_dir).unwrap();
    let config = moltis_code_index::CodeIndexConfig::default();
    moltis_code_index::filter::filter_tracked_files(project_dir, &tracked, &config).unwrap()
}

/// Benchmark computing content hash for a single file.
#[divan::bench]
fn content_hash_single_file() {
    let lib_rs_path = Path::new("/mnt/fast_data/workspaces/moltis/crates/code-index/src/lib.rs");
    let hash = moltis_code_index::filter::content_hash(lib_rs_path);
    divan::black_box(hash.unwrap());
}

/// Benchmark computing content hashes for multiple files.
#[divan::bench(args = [10, 50, 100, 500])]
fn content_hash_multiple_files(count: usize) {
    let filtered = get_filtered_files();
    let files_to_hash = filtered.iter().take(count);

    let mut hashes = Vec::new();
    for file in files_to_hash {
        if let Ok(hash) = moltis_code_index::filter::content_hash(&file.path) {
            hashes.push(hash);
        }
    }

    divan::black_box(hashes.len());
}

// ── Delta Computation Benchmarks ───────────────────────────────────────────────

/// Benchmark computing delta with empty previous snapshot (first run).
#[divan::bench]
fn compute_delta_first_run() {
    let project_dir = Path::new(MOLTIS_PROJECT_PATH);
    let config = moltis_code_index::CodeIndexConfig::default();
    let previous: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    let delta = moltis_code_index::delta::compute_delta(project_dir, &config, &previous);
    divan::black_box(delta.unwrap());
}

/// Benchmark computing delta with previous snapshot (incremental).
#[divan::bench]
fn compute_delta_incremental() {
    let project_dir = Path::new(MOLTIS_PROJECT_PATH);
    let config = moltis_code_index::CodeIndexConfig::default();

    let mut previous = std::collections::HashMap::new();
    let tracked = moltis_code_index::discover::discover_tracked_files(project_dir).unwrap();
    let filtered =
        moltis_code_index::filter::filter_tracked_files(project_dir, &tracked, &config).unwrap();

    for file in filtered.iter().take(filtered.len() / 2) {
        if let Ok(hash) = moltis_code_index::filter::content_hash(&file.path) {
            previous.insert(file.relative_path.to_string_lossy().into_owned(), hash);
        }
    }

    let delta = moltis_code_index::delta::compute_delta(project_dir, &config, &previous);
    divan::black_box(delta.unwrap());
}

// ── Config Operations ──────────────────────────────────────────────────────────

/// Benchmark creating default CodeIndexConfig.
#[divan::bench]
fn config_default() -> moltis_code_index::CodeIndexConfig {
    divan::black_box(moltis_code_index::CodeIndexConfig::default())
}

/// Benchmark config serialization/deserialization.
#[divan::bench]
fn config_serde_roundtrip() {
    let config = moltis_code_index::CodeIndexConfig::default();
    let json = serde_json::to_string(&config).unwrap();
    let _ = serde_json::from_str::<moltis_code_index::CodeIndexConfig>(&json).unwrap();
}

// ── Full Pipeline Benchmarks ───────────────────────────────────────────────────

/// Benchmark the full discover → filter → hash pipeline.
#[divan::bench]
fn full_discover_filter_hash_pipeline() {
    let project_dir = Path::new(MOLTIS_PROJECT_PATH);
    let config = moltis_code_index::CodeIndexConfig::default();

    let tracked = moltis_code_index::discover::discover_tracked_files(project_dir).unwrap();
    let filtered =
        moltis_code_index::filter::filter_tracked_files(project_dir, &tracked, &config).unwrap();

    let mut hashes = Vec::new();
    for file in filtered.iter().take(100) {
        if let Ok(hash) = moltis_code_index::filter::content_hash(&file.path) {
            hashes.push(hash);
        }
    }

    divan::black_box((tracked.len(), filtered.len(), hashes.len()));
}

/// Benchmark full delta computation with snapshot.
#[divan::bench]
fn full_delta_computation_with_snapshot() {
    let project_dir = Path::new(MOLTIS_PROJECT_PATH);
    let config = moltis_code_index::CodeIndexConfig::default();

    let mut previous: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let tracked = moltis_code_index::discover::discover_tracked_files(project_dir).unwrap();
    let filtered =
        moltis_code_index::filter::filter_tracked_files(project_dir, &tracked, &config).unwrap();

    for file in filtered.iter().take(filtered.len() / 2) {
        if let Ok(hash) = moltis_code_index::filter::content_hash(&file.path) {
            previous.insert(file.relative_path.to_string_lossy().into_owned(), hash);
        }
    }

    let delta = moltis_code_index::delta::compute_delta(project_dir, &config, &previous);
    divan::black_box(delta.unwrap());
}

// ═══════════════════════════════════════════════════════════════════════════════
// SEARCH COMPARISON BENCHMARKS
// ═══════════════════════════════════════════════════════════════════════════════
//
// Three backends compared:
//   1. FTS5 (builtin) — in-process SQLite FTS5 over pre-chunked content
//   2. grep-searcher — in-process regex over raw files (no index, no subprocess)
//   3. ripgrep CLI — subprocess `rg` (realistic "no index" deployment baseline)
//
// Fairness notes:
//   - grep-searcher eliminates subprocess overhead, isolating pure search cost
//   - FTS5 has upfront index build cost (~1.5K files, ~12K chunks) but zero
//     per-query indexing — measures warm-cache search speed
//   - ripgrep CLI includes process spawn + gitignore traversal overhead
//   - All three have different semantics: FTS5 is tokenized BM25, grep-searcher
//     and ripgrep are literal regex. Results will differ in count and ranking.
// ═══════════════════════════════════════════════════════════════════════════════

// ── FTS5 (builtin) ─────────────────────────────────────────────────────────────

/// Benchmark a single FTS5 keyword search query.
#[divan::bench(args = BENCHMARK_QUERIES)]
fn fts5_keyword_search(query: &&str) {
    let rt = tokio_rt();
    let fixture = search_fixture();

    let results = rt.block_on(async {
        fixture
            .store
            .search_keyword(PROJECT_ID, query, 20)
            .await
            .unwrap()
    });

    divan::black_box(results);
}

/// Benchmark running all FTS5 queries sequentially.
#[divan::bench]
fn fts5_all_queries() {
    let rt = tokio_rt();
    let fixture = search_fixture();

    let total = rt.block_on(async {
        let mut total = 0usize;
        for query in BENCHMARK_QUERIES {
            let results = fixture
                .store
                .search_keyword(PROJECT_ID, query, 20)
                .await
                .unwrap();
            total += results.len();
        }
        total
    });

    divan::black_box(total);
}

// ── grep-searcher (in-process, no index) ───────────────────────────────────────

/// In-process regex search using the same `grep-searcher` crate that ripgrep uses.
/// This eliminates subprocess spawn overhead for a fairer pure-search comparison.
fn grep_searcher_count(project_dir: &Path, query: &str) -> usize {
    use {
        grep_regex::RegexMatcher,
        grep_searcher::{Searcher, sinks::UTF8},
    };

    // Build a case-insensitive regex from the query words (any word matches)
    let words: Vec<&str> = query.split_whitespace().collect();
    let pattern = words.join("|");
    let matcher = RegexMatcher::new(&pattern).unwrap();
    let mut searcher = Searcher::new();

    let mut count = 0usize;
    let result = searcher.search_path(
        &matcher,
        project_dir,
        UTF8(|_buf, _line| {
            count += 1;
            Ok(true)
        }),
    );

    match result {
        Ok(()) => count,
        Err(_) => 0,
    }
}

/// Benchmark a single in-process grep-searcher query.
#[divan::bench(args = BENCHMARK_QUERIES)]
fn grep_searcher(query: &&str) {
    let project_dir = Path::new(MOLTIS_PROJECT_PATH);
    let matches = grep_searcher_count(project_dir, query);
    divan::black_box(matches);
}

/// Benchmark running all grep-searcher queries sequentially.
#[divan::bench]
fn grep_searcher_all_queries() {
    let project_dir = Path::new(MOLTIS_PROJECT_PATH);
    let mut total = 0usize;
    for query in BENCHMARK_QUERIES {
        total += grep_searcher_count(project_dir, query);
    }
    divan::black_box(total);
}

// ── ripgrep CLI (subprocess, no index) ─────────────────────────────────────────

/// Run `rg` as a subprocess — realistic "no index" deployment baseline.
/// Includes process spawn, gitignore traversal, and regex overhead.
fn rg_subprocess_count(project_dir: &Path, query: &str) -> usize {
    // Split multi-word queries into OR terms
    let or_query = query.replace(" ", " OR ");
    let output = std::process::Command::new("rg")
        .args(["--no-heading", "--count", &or_query])
        .arg(project_dir)
        .output()
        .expect("rg should be available");

    if !output.status.success() {
        return 0;
    }

    let total: usize = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split(':').nth(1)?.parse::<usize>().ok())
        .sum();

    total
}

/// Benchmark a single ripgrep subprocess query.
#[divan::bench(args = BENCHMARK_QUERIES)]
fn ripgrep_subprocess(query: &&str) {
    let project_dir = Path::new(MOLTIS_PROJECT_PATH);
    let matches = rg_subprocess_count(project_dir, query);
    divan::black_box(matches);
}

/// Benchmark running all ripgrep subprocess queries sequentially.
#[divan::bench]
fn ripgrep_subprocess_all_queries() {
    let project_dir = Path::new(MOLTIS_PROJECT_PATH);
    let mut total = 0usize;
    for query in BENCHMARK_QUERIES {
        total += rg_subprocess_count(project_dir, query);
    }
    divan::black_box(total);
}

// ── QMD (external sidecar, gated on availability) ──────────────────────────────
//
// QMD is an external binary (https://github.com/tobi/qmd) that provides hybrid
// search (BM25 + vector + optional LLM reranking) via a sidecar process.
// These benchmarks are skipped entirely if QMD is not installed.

static QMD_AVAILABLE: OnceLock<bool> = OnceLock::new();

fn qmd_available() -> bool {
    *QMD_AVAILABLE.get_or_init(|| {
        std::process::Command::new("qmd")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

#[cfg(feature = "qmd")]
mod qmd_benches {
    use super::*;

    struct QmdFixture {
        manager: moltis_qmd::QmdManager,
    }

    static QMD_FIXTURE: OnceLock<QmdFixture> = OnceLock::new();

    fn qmd_fixture() -> Option<&'static QmdFixture> {
        if !qmd_available() {
            return None;
        }

        QMD_FIXTURE
            .get_or_init(|| {
                let rt = tokio_rt();
                rt.block_on(async {
                    let project_dir = Path::new(MOLTIS_PROJECT_PATH);
                    let config = moltis_qmd::QmdManagerConfig {
                        collections: std::collections::HashMap::from([(
                            PROJECT_ID.to_string(),
                            moltis_qmd::QmdCollection {
                                path: project_dir.to_path_buf(),
                                glob: "**/*.{rs,toml,json,md,css,js,html,sql}".to_string(),
                            },
                        )]),
                        max_results: 20,
                        timeout_ms: 30_000,
                        work_dir: project_dir.to_path_buf(),
                        ..Default::default()
                    };
                    let manager = moltis_qmd::QmdManager::new(config);

                    if manager.is_available().await {
                        // Build the index (first run)
                        match manager.refresh_index(true).await {
                            Ok(()) => eprintln!("[qmd fixture] index built successfully"),
                            Err(e) => eprintln!("[qmd fixture] index build failed: {e}"),
                        }
                    } else {
                        eprintln!("[qmd fixture] QMD binary not available, skipping");
                    }

                    QmdFixture { manager }
                })
            })
            .into()
    }

    /// Benchmark a single QMD hybrid search query (no rerank).
    #[divan::bench(args = BENCHMARK_QUERIES)]
    fn qmd_hybrid_search(query: &&str) {
        let Some(fixture) = qmd_fixture() else {
            return;
        };
        let rt = tokio_rt();
        let results = rt.block_on(async {
            fixture
                .manager
                .hybrid_search(query, 20, false)
                .await
                .unwrap_or_default()
        });
        divan::black_box(results);
    }

    /// Benchmark a single QMD keyword-only search query.
    #[divan::bench(args = BENCHMARK_QUERIES)]
    fn qmd_keyword_search(query: &&str) {
        let Some(fixture) = qmd_fixture() else {
            return;
        };
        let rt = tokio_rt();
        let results = rt.block_on(async {
            fixture
                .manager
                .keyword_search(query, 20)
                .await
                .unwrap_or_default()
        });
        divan::black_box(results);
    }

    /// Benchmark running all QMD hybrid queries sequentially.
    #[divan::bench]
    fn qmd_hybrid_all_queries() {
        let Some(fixture) = qmd_fixture() else {
            return;
        };
        let rt = tokio_rt();
        let total = rt.block_on(async {
            let mut total = 0usize;
            for query in BENCHMARK_QUERIES {
                let results = fixture
                    .manager
                    .hybrid_search(query, 20, false)
                    .await
                    .unwrap_or_default();
                total += results.len();
            }
            total
        });
        divan::black_box(total);
    }
}
