#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{path::Path, sync::OnceLock};

use {
    divan::Bencher,
    moltis_memory::{
        schema::ChunkRow,
        store::{MemoryStore, MergeStrategy},
        store_sqlite::SqliteMemoryStore,
    },
    rand::{RngExt, SeedableRng, rngs::StdRng},
};

fn main() {
    // Initialize the zvec global library once up front so its startup message
    // doesn't interleave with divan's results table mid-run.
    #[cfg(feature = "zvec")]
    ensure_zvec_init();
    divan::main();
}

// ═══════════════════════════════════════════════════════════════════════════════
// Shared Infrastructure
// ═══════════════════════════════════════════════════════════════════════════════

const VECTOR_DIM: usize = 768;
const INGEST_COUNT: usize = 1000;
const SEARCH_COUNT: usize = 10_000;
const CACHE_COUNT: usize = 2000;

/// Fixed seed so every run generates the same dataset/query vectors, making
/// benchmark results reproducible across runs and machines.
const RNG_SEED: u64 = 0xBEEF_CAFE_1234_5678;

static TOKIO_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn tokio_rt() -> &'static tokio::runtime::Runtime {
    TOKIO_RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
    })
}

// ── Shared helpers ────────────────────────────────────────────────────────────

fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn random_embedding(rng: &mut StdRng, dim: usize) -> Vec<f32> {
    let mut v: Vec<f32> = (0..dim).map(|_| rng.random()).collect();
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

fn bench_file() -> moltis_memory::schema::FileRow {
    moltis_memory::schema::FileRow {
        path: "bench/memory.md".into(),
        source: "bench".into(),
        hash: "bench-file-hash".into(),
        mtime: 1_000_000,
        size: 42,
    }
}

fn make_chunk(id: &str, text: &str, embedding: Option<&[f32]>) -> ChunkRow {
    ChunkRow {
        id: id.into(),
        path: "bench/memory.md".into(),
        source: "bench".into(),
        start_line: 1,
        end_line: 1,
        hash: format!("hash-{id}"),
        model: "bench-model".into(),
        text: text.into(),
        embedding: embedding.map(vec_to_blob),
        updated_at: "2026-01-01T00:00:00Z".into(),
    }
}

fn make_chunks(n: usize, dim: usize) -> Vec<ChunkRow> {
    let words = [
        "alpha", "beta", "gamma", "delta", "epsilon", "lambda", "sigma", "omega", "phi", "theta",
        "kappa", "rho", "tau", "xi",
    ];
    let mut rng = StdRng::seed_from_u64(RNG_SEED);
    (0..n)
        .map(|i| {
            let i1 = rng.random_range(0..words.len());
            let i2 = rng.random_range(0..words.len());
            let i3 = rng.random_range(0..words.len());
            let text = format!(
                "chunk {i}: memory test chunk with keywords {} {} {} and some filler text for realistic FTS indexing behavior",
                words[i1], words[i2], words[i3]
            );
            make_chunk(
                &format!("chunk-{i}"),
                &text,
                Some(&random_embedding(&mut rng, dim)),
            )
        })
        .collect()
}

fn query_embedding(dim: usize) -> Vec<f32> {
    // Distinct seed from the dataset so the query vector isn't one of the
    // stored chunks; fixed so it stays identical across iterations and runs.
    let mut rng = StdRng::seed_from_u64(RNG_SEED.wrapping_add(1));
    random_embedding(&mut rng, dim)
}

// ── SQLite helpers ────────────────────────────────────────────────────────────

async fn create_sqlite_disk(db_path: &Path) -> SqliteMemoryStore {
    use sqlx::sqlite::SqliteConnectOptions;
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    moltis_memory::run_migrations(&pool).await.unwrap();
    SqliteMemoryStore::new(pool)
}

async fn create_sqlite_in_memory() -> SqliteMemoryStore {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    moltis_memory::run_migrations(&pool).await.unwrap();
    SqliteMemoryStore::new(pool)
}

// ── Zvec helpers ──────────────────────────────────────────────────────────────

#[cfg(feature = "zvec")]
static ZVEC_LIB_INIT: OnceLock<()> = OnceLock::new();

/// Ensure zvec global library is initialized exactly once.
/// Uses `moltis_memory_zvec::initialize()` which bootstraps both the
/// zvec C library and a temporary collection (immediately dropped).
#[cfg(feature = "zvec")]
fn ensure_zvec_init() {
    ZVEC_LIB_INIT.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let collection =
            moltis_memory_zvec::initialize(dir.path(), Some(VECTOR_DIM as u32)).unwrap();
        drop(collection);
        eprintln!("[zvec] global library initialized");
    });
}

#[cfg(feature = "zvec")]
fn create_zvec_store(db_path: &Path) -> moltis_memory_zvec::ZvecMemoryStore {
    ensure_zvec_init();
    let collection =
        moltis_memory_zvec::open_or_create_collection(db_path, Some(VECTOR_DIM as u32)).unwrap();
    let cache_path = {
        let mut p = db_path.to_path_buf();
        p.as_mut_os_string().push(".cache");
        p
    };
    let cache = moltis_memory_zvec::RedbCache::new(&cache_path).unwrap();
    moltis_memory_zvec::ZvecMemoryStore::new(collection, cache)
}

/// Batch-insert chunks for zvec, which has a per-call limit.
#[cfg(feature = "zvec")]
const ZVEC_UPSERT_BATCH: usize = 500;

#[cfg(feature = "zvec")]
async fn zvec_batched_upsert(store: &moltis_memory_zvec::ZvecMemoryStore, chunks: &[ChunkRow]) {
    use moltis_memory::store::MemoryStore;
    store.upsert_file(&bench_file()).await.unwrap();
    for batch in chunks.chunks(ZVEC_UPSERT_BATCH) {
        store.upsert_chunks(batch).await.unwrap();
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Shared fixtures (lazily built once, reused across benchmark iterations)
// ═══════════════════════════════════════════════════════════════════════════════

struct SearchFixtureSqlite {
    store: SqliteMemoryStore,
    _db_dir: tempfile::TempDir,
}

static SEARCH_FIXTURE_SQLITE: OnceLock<SearchFixtureSqlite> = OnceLock::new();

fn search_fixture_sqlite() -> &'static SearchFixtureSqlite {
    SEARCH_FIXTURE_SQLITE.get_or_init(|| {
        let rt = tokio_rt();
        rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("bench_search.db");
            let store = create_sqlite_disk(&db_path).await;
            store.upsert_file(&bench_file()).await.unwrap();
            let chunks = make_chunks(SEARCH_COUNT, VECTOR_DIM);
            store.upsert_chunks(&chunks).await.unwrap();
            SearchFixtureSqlite {
                store,
                _db_dir: dir,
            }
        })
    })
}

#[cfg(feature = "zvec")]
struct SearchFixtureZvec {
    store: moltis_memory_zvec::ZvecMemoryStore,
    _db_dir: tempfile::TempDir,
}

#[cfg(feature = "zvec")]
static SEARCH_FIXTURE_ZVEC: OnceLock<SearchFixtureZvec> = OnceLock::new();

#[cfg(feature = "zvec")]
fn search_fixture_zvec() -> &'static SearchFixtureZvec {
    SEARCH_FIXTURE_ZVEC.get_or_init(|| {
        let rt = tokio_rt();
        rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("collection");
            let store = create_zvec_store(&db_path);
            let chunks = make_chunks(SEARCH_COUNT, VECTOR_DIM);
            zvec_batched_upsert(&store, &chunks).await;
            let coll = store.collection_arc();
            moltis_memory_zvec::flush_collection(&coll).unwrap();
            SearchFixtureZvec {
                store,
                _db_dir: dir,
            }
        })
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
// BENCHMARK 1: Ingest 1000 chunks
// ═══════════════════════════════════════════════════════════════════════════════

#[divan::bench]
fn sqlite_ingest_1000(bencher: Bencher) {
    let rt = tokio_rt();
    // Dataset is generated once, outside the timed region, so we measure store
    // init + ingest — not RNG/normalization. On-disk to match zvec's storage
    // medium (apples-to-apples, not RAM-vs-disk).
    let chunks = make_chunks(INGEST_COUNT, VECTOR_DIM);
    bencher.bench(|| {
        rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("bench.db");
            let store = create_sqlite_disk(&db_path).await;
            store.upsert_file(&bench_file()).await.unwrap();
            store.upsert_chunks(&chunks).await.unwrap();
        });
    });
}

#[cfg(feature = "zvec")]
#[divan::bench]
fn zvec_ingest_1000(bencher: Bencher) {
    let rt = tokio_rt();
    let chunks = make_chunks(INGEST_COUNT, VECTOR_DIM);
    bencher.bench(|| {
        rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("collection");
            let store = create_zvec_store(&db_path);
            zvec_batched_upsert(&store, &chunks).await;
            drop(store);
        });
    });
}

// ═══════════════════════════════════════════════════════════════════════════════
// BENCHMARK 2: Vector search (cosine top-10)
// ═══════════════════════════════════════════════════════════════════════════════

#[divan::bench]
fn sqlite_vector_search_top10(bencher: Bencher) {
    let rt = tokio_rt();
    let fixture = search_fixture_sqlite();
    let query = query_embedding(VECTOR_DIM);
    bencher.bench(|| {
        let results = rt.block_on(async {
            fixture
                .store
                .hybrid_search(&query, "", 1.0, 0.0, MergeStrategy::Weighted, 10)
                .await
                .unwrap()
        });
        divan::black_box(results);
    });
}

#[cfg(feature = "zvec")]
#[divan::bench]
fn zvec_vector_search_top10(bencher: Bencher) {
    let rt = tokio_rt();
    let fixture = search_fixture_zvec();
    let query = query_embedding(VECTOR_DIM);
    bencher.bench(|| {
        let results = rt.block_on(async {
            fixture
                .store
                .hybrid_search(&query, "", 1.0, 0.0, MergeStrategy::Weighted, 10)
                .await
                .unwrap()
        });
        divan::black_box(results);
    });
}

// ═══════════════════════════════════════════════════════════════════════════════
// BENCHMARK 3: Keyword search (FTS top-10)
// ═══════════════════════════════════════════════════════════════════════════════

#[divan::bench]
fn sqlite_keyword_search_top10(bencher: Bencher) {
    let rt = tokio_rt();
    let fixture = search_fixture_sqlite();
    let query = "alpha gamma sigma";
    bencher.bench(|| {
        let results = rt.block_on(async {
            fixture
                .store
                .hybrid_search(&[], query, 0.0, 1.0, MergeStrategy::Weighted, 10)
                .await
                .unwrap()
        });
        divan::black_box(results);
    });
}

#[cfg(feature = "zvec")]
#[divan::bench]
fn zvec_keyword_search_top10(bencher: Bencher) {
    let rt = tokio_rt();
    let fixture = search_fixture_zvec();
    let query = "alpha gamma sigma";
    bencher.bench(|| {
        let results = rt.block_on(async {
            fixture
                .store
                .hybrid_search(&[], query, 0.0, 1.0, MergeStrategy::Weighted, 10)
                .await
                .unwrap()
        });
        divan::black_box(results);
    });
}

// ═══════════════════════════════════════════════════════════════════════════════
// BENCHMARK 4: Hybrid search (RRF top-10)
// ═══════════════════════════════════════════════════════════════════════════════

#[divan::bench]
fn sqlite_hybrid_search_rrf_top10(bencher: Bencher) {
    let rt = tokio_rt();
    let fixture = search_fixture_sqlite();
    let query = "alpha gamma sigma";
    let q_emb = query_embedding(VECTOR_DIM);
    bencher.bench(|| {
        let results = rt.block_on(async {
            fixture
                .store
                .hybrid_search(&q_emb, query, 1.0, 1.0, MergeStrategy::Rrf { k: 60 }, 10)
                .await
                .unwrap()
        });
        divan::black_box(results);
    });
}

#[cfg(feature = "zvec")]
#[divan::bench]
fn zvec_hybrid_search_rrf_top10(bencher: Bencher) {
    let rt = tokio_rt();
    let fixture = search_fixture_zvec();
    let query = "alpha gamma sigma";
    let q_emb = query_embedding(VECTOR_DIM);
    bencher.bench(|| {
        let results = rt.block_on(async {
            fixture
                .store
                .hybrid_search(&q_emb, query, 1.0, 1.0, MergeStrategy::Rrf { k: 60 }, 10)
                .await
                .unwrap()
        });
        divan::black_box(results);
    });
}

// ═══════════════════════════════════════════════════════════════════════════════
// BENCHMARK 5: Cache lookup (N keys)
//
// Asymmetry note: SQLite's `get_cached_embedding` is a pure SELECT, while the
// redb-backed zvec cache opens a WRITE transaction on every hit to bump the
// LRU access timestamp (see memory-zvec/src/cache.rs). So zvec_cache_lookup
// measures read+write and sqlite_cache_lookup measures read-only. This is the
// real production cost in both cases; keep it in mind when comparing.
// ═══════════════════════════════════════════════════════════════════════════════

#[divan::bench]
fn sqlite_cache_lookup(bencher: Bencher) {
    let rt = tokio_rt();
    let store = rt.block_on(async { create_sqlite_in_memory().await });
    let mut rng = StdRng::seed_from_u64(RNG_SEED);
    let emb = random_embedding(&mut rng, 4);
    let hashes: Vec<String> = (0..CACHE_COUNT).map(|i| format!("hash-{i}")).collect();
    let entries: Vec<moltis_memory::store::CacheEntry<'_>> = hashes
        .iter()
        .map(|h| moltis_memory::store::CacheEntry {
            provider: "bench",
            model: "model",
            provider_key: "key",
            hash: h.as_str(),
            embedding: &emb,
        })
        .collect();
    rt.block_on(async { store.put_cached_embeddings_batch(&entries).await.unwrap() });

    bencher.bench(|| {
        let total = rt.block_on(async {
            let mut found = 0usize;
            for h in &hashes {
                if store
                    .get_cached_embedding("bench", "model", h)
                    .await
                    .unwrap()
                    .is_some()
                {
                    found += 1;
                }
            }
            found
        });
        divan::black_box(total);
    });
}

#[cfg(feature = "zvec")]
#[divan::bench]
fn zvec_cache_lookup(bencher: Bencher) {
    let rt = tokio_rt();
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("collection");
    let cache_path = dir.path().join("cache.redb");

    ensure_zvec_init();
    let collection =
        moltis_memory_zvec::open_or_create_collection(&db_path, Some(VECTOR_DIM as u32)).unwrap();
    let cache = moltis_memory_zvec::RedbCache::new(&cache_path).unwrap();
    let store = moltis_memory_zvec::ZvecMemoryStore::with_cache(collection, cache);

    let mut rng = StdRng::seed_from_u64(RNG_SEED);
    let emb = random_embedding(&mut rng, 4);
    let hashes: Vec<String> = (0..CACHE_COUNT).map(|i| format!("hash-{i}")).collect();
    let entries: Vec<moltis_memory::store::CacheEntry<'_>> = hashes
        .iter()
        .map(|h| moltis_memory::store::CacheEntry {
            provider: "bench",
            model: "model",
            provider_key: "key",
            hash: h.as_str(),
            embedding: &emb,
        })
        .collect();
    rt.block_on(async { store.put_cached_embeddings_batch(&entries).await.unwrap() });

    bencher.bench(|| {
        let total = rt.block_on(async {
            let mut found = 0usize;
            for h in &hashes {
                if store
                    .get_cached_embedding("bench", "model", h)
                    .await
                    .unwrap()
                    .is_some()
                {
                    found += 1;
                }
            }
            found
        });
        divan::black_box(total);
    });
}
