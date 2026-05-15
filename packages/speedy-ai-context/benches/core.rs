//! Microbenchmarks for the pure CPU-bound paths inside `speedy`.
//!
//! All benches are self-contained: they do not require Ollama, the daemon, or
//! the speedy binary. Embeddings are synthetic random vectors so similarity
//! scoring exercises the cosine path with realistic data shapes (`d=384` for
//! `all-minilm:l6-v2`, the default model).
//!
//! Run with `cargo bench -p speedy`.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use speedy::db::{ChunkRecord, SqliteVectorStore, VectorStore};
use speedy::document::Document;
use std::sync::Arc;
use tokio::runtime::Runtime;
use uuid::Uuid;

const EMBED_DIM: usize = 384;

fn rand_embedding(seed: u64) -> Vec<f32> {
    // Cheap deterministic PRNG so benches reproduce across runs.
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (0..EMBED_DIM)
        .map(|_| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((x >> 33) as f32 / u32::MAX as f32) * 2.0 - 1.0
        })
        .collect()
}

fn synthetic_text(lines: usize) -> String {
    (0..lines)
        .map(|i| format!("line {i}: lorem ipsum dolor sit amet consectetur adipiscing elit"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn bench_chunk_file(c: &mut Criterion) {
    let mut group = c.benchmark_group("chunk_file");
    for &lines in &[100usize, 1_000, 10_000] {
        let text = synthetic_text(lines);
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(lines), &text, |b, t| {
            b.iter(|| Document::chunk_file(t, 1000, 200));
        });
    }
    group.finish();
}

fn store_with_n_chunks(rt: &Runtime, n: usize) -> (tempfile::TempDir, Arc<SqliteVectorStore>) {
    let dir = tempfile::tempdir().expect("create tempdir");
    let store = rt
        .block_on(SqliteVectorStore::new(dir.path().to_str().unwrap()))
        .expect("create store");

    let records: Vec<ChunkRecord> = (0..n)
        .map(|i| ChunkRecord {
            id: Uuid::new_v4().to_string(),
            file_path: format!("file_{}.rs", i % 50),
            line: i,
            text: format!("chunk {i}"),
            hash: "h".to_string(),
            embedding: rand_embedding(i as u64),
            last_modified: "2026-05-14".to_string(),
        })
        .collect();
    rt.block_on(store.insert_chunks(&records)).expect("insert");
    (dir, store)
}

fn bench_similarity_search(c: &mut Criterion) {
    let rt = Runtime::new().expect("runtime");
    let mut group = c.benchmark_group("similarity_search");
    for &n in &[1_000usize, 10_000, 50_000] {
        let (_dir, store) = store_with_n_chunks(&rt, n);
        let q = rand_embedding(99_999);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &(store, q), |b, (s, q)| {
            b.to_async(&rt).iter(|| async {
                s.similarity_search(q, 5).await.unwrap();
            });
        });
    }
    group.finish();
}

fn bench_insert_chunks(c: &mut Criterion) {
    let rt = Runtime::new().expect("runtime");
    let mut group = c.benchmark_group("insert_chunks");
    // Lower bound so the bench finishes in reasonable time. The expensive part
    // is SQLite write amplification + cosine cache load, both already covered
    // by similarity_search; keep this just for regression tracking.
    group.sample_size(20);
    for &n in &[100usize, 1_000] {
        let records: Vec<ChunkRecord> = (0..n)
            .map(|i| ChunkRecord {
                id: Uuid::new_v4().to_string(),
                file_path: format!("f_{}.rs", i % 20),
                line: i,
                text: format!("chunk text {i}"),
                hash: "h".to_string(),
                embedding: rand_embedding(i as u64),
                last_modified: "2026-05-14".to_string(),
            })
            .collect();
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &records, |b, recs| {
            b.iter_custom(|iters| {
                let mut total = std::time::Duration::ZERO;
                for _ in 0..iters {
                    let dir = tempfile::tempdir().unwrap();
                    let store = rt
                        .block_on(SqliteVectorStore::new(dir.path().to_str().unwrap()))
                        .unwrap();
                    let start = std::time::Instant::now();
                    rt.block_on(store.insert_chunks(recs)).unwrap();
                    total += start.elapsed();
                }
                total
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_chunk_file, bench_similarity_search, bench_insert_chunks);
criterion_main!(benches);
