//! Performance benchmarks for Ryther Protocol.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::sync::Arc;

use ryther_core::crypto::threshold::{ShamirSecretSharing, ThresholdEncryptor, ThresholdParams};
use ryther_core::dag::store::DagStore;
use ryther_core::execution::mvcc::MultiVersionState;
use ryther_core::storage::jmt::JellyfishMerkleTree;
use ryther_core::types::event::{DagEvent, EncryptedBatch};
use ryther_core::types::transaction::DecryptedTransaction;
use ryther_core::types::ValidatorId;
use ryther_core::types::{keccak256, sha256, Address, BlsSignature, StateKey, U256};

/// Benchmark cryptographic operations.
fn bench_crypto(c: &mut Criterion) {
    let mut group = c.benchmark_group("crypto");

    let data = vec![0u8; 1024];

    group.bench_function("sha256_1kb", |b| b.iter(|| sha256(black_box(&data))));

    group.bench_function("keccak256_1kb", |b| b.iter(|| keccak256(black_box(&data))));

    // Threshold encryption
    let params = ThresholdParams::new(10, 7).unwrap();
    let mut encryptor = ThresholdEncryptor::new(params.clone());
    let validators: Vec<ValidatorId> = (0..10).map(|i| ValidatorId([i as u8; 48])).collect();
    let plaintext = vec![0u8; 256];

    group.bench_function("threshold_encrypt_256b", |b| {
        b.iter(|| encryptor.encrypt(black_box(&plaintext), black_box(&validators)))
    });

    // Shamir split
    let mut sss = ShamirSecretSharing::new(params);
    let secret = [0xABu8; 32];

    group.bench_function("shamir_split_10_shares", |b| {
        b.iter(|| sss.split(black_box(&secret), black_box(&validators)))
    });

    group.finish();
}

/// Benchmark MVCC state operations.
fn bench_mvcc(c: &mut Criterion) {
    let mut group = c.benchmark_group("mvcc");

    let state = Arc::new(MultiVersionState::new());

    // Write benchmark
    group.bench_function("write", |b| {
        let mut seq = 0u64;
        b.iter(|| {
            let key = StateKey::from_raw([seq as u8; 20], U256::from_u64(seq));
            state.write(black_box(key), black_box(seq), Some(U256::from_u64(seq)));
            seq += 1;
        })
    });

    // Populate state for read benchmark
    for i in 0u64..1000 {
        let key = StateKey::from_raw([i as u8; 20], U256::from_u64(i));
        state.write(key, i, Some(U256::from_u64(i)));
        state.mark_committed(&key, i);
    }

    // Read benchmark
    group.bench_function("read", |b| {
        let mut i = 0u64;
        b.iter(|| {
            let key = StateKey::from_raw([(i % 1000) as u8; 20], U256::from_u64(i % 1000));
            let _ = state.read(black_box(&key), black_box(i + 1000));
            i += 1;
        })
    });

    group.finish();
}

/// Benchmark DAG operations.
fn bench_dag(c: &mut Criterion) {
    let mut group = c.benchmark_group("dag");

    let store = DagStore::new();

    // Insert benchmark
    group.bench_function("insert_event", |b| {
        let mut round = 0u64;
        b.iter(|| {
            let event_id = sha256(&round.to_le_bytes());
            let event = DagEvent {
                id: event_id,
                creator: ValidatorId([round as u8; 48]),
                round,
                lamport_time: round,
                strong_parents: vec![],
                weak_parents: vec![],
                payload: EncryptedBatch::empty(),
                signature: BlsSignature::default(),
            };
            store.insert(event);
            round += 1;
        })
    });

    // Populate DAG for lookup benchmark
    for i in 0u64..1000 {
        let event_id = sha256(&i.to_le_bytes());
        let event = DagEvent {
            id: event_id,
            creator: ValidatorId([i as u8; 48]),
            round: i,
            lamport_time: i,
            strong_parents: vec![],
            weak_parents: vec![],
            payload: EncryptedBatch::empty(),
            signature: BlsSignature::default(),
        };
        store.insert(event);
    }

    // Get benchmark
    group.bench_function("get_event", |b| {
        let mut i = 0u64;
        b.iter(|| {
            let event_id = sha256(&(i % 1000).to_le_bytes());
            let _ = store.get(black_box(&event_id));
            i += 1;
        })
    });

    group.finish();
}

/// Benchmark Jellyfish Merkle Tree.
fn bench_jmt(c: &mut Criterion) {
    let mut group = c.benchmark_group("jmt");

    let mut tree = JellyfishMerkleTree::new();

    // Insert benchmark
    group.bench_function("insert", |b| {
        let mut i = 0u64;
        b.iter(|| {
            let key = StateKey::from_raw([i as u8; 20], U256::from_u64(i));
            tree.put(black_box(&key), black_box(&U256::from_u64(i)));
            i += 1;
        })
    });

    // Populate tree for read benchmark
    let mut tree = JellyfishMerkleTree::new();
    for i in 0u64..1000 {
        let key = StateKey::from_raw([i as u8; 20], U256::from_u64(i));
        tree.put(&key, &U256::from_u64(i));
    }

    // Get benchmark
    group.bench_function("get", |b| {
        let mut i = 0u64;
        b.iter(|| {
            let key = StateKey::from_raw([(i % 1000) as u8; 20], U256::from_u64(i % 1000));
            let _ = tree.get(black_box(&key));
            i += 1;
        })
    });

    // Proof generation
    group.bench_function("get_proof", |b| {
        let mut i = 0u64;
        b.iter(|| {
            let key = StateKey::from_raw([(i % 1000) as u8; 20], U256::from_u64(i % 1000));
            let _ = tree.get_proof(black_box(&key));
            i += 1;
        })
    });

    group.finish();
}

/// Benchmark transaction creation and hashing.
fn bench_transactions(c: &mut Criterion) {
    let mut group = c.benchmark_group("transactions");

    group.bench_function("create_tx", |b| {
        b.iter(|| DecryptedTransaction {
            from: Address([0xAA; 20]),
            to: Some(Address([0xBB; 20])),
            value: U256::from_u64(1_000_000_000_000_000_000),
            gas_limit: 21000,
            gas_price: U256::from_u64(1_000_000_000),
            nonce: 0,
            data: vec![],
            source_commitment: [0; 32],
            sequence_number: 0,
        })
    });

    // Hash transaction
    let tx = DecryptedTransaction {
        from: Address([0xAA; 20]),
        to: Some(Address([0xBB; 20])),
        value: U256::from_u64(1_000_000_000_000_000_000),
        gas_limit: 21000,
        gas_price: U256::from_u64(1_000_000_000),
        nonce: 0,
        data: vec![0u8; 100],
        source_commitment: [0; 32],
        sequence_number: 0,
    };
    let tx_bytes = bincode::serialize(&tx).unwrap();

    group.bench_function("hash_tx", |b| b.iter(|| sha256(black_box(&tx_bytes))));

    group.finish();
}

/// Throughput simulation - batch of transactions.
fn bench_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput");
    group.sample_size(10);

    for batch_size in [100, 1000, 10000].iter() {
        group.bench_with_input(
            BenchmarkId::new("batch_hash", batch_size),
            batch_size,
            |b, &size| {
                let txs: Vec<_> = (0..size)
                    .map(|i| {
                        let tx = DecryptedTransaction {
                            from: Address([i as u8; 20]),
                            to: Some(Address([0xBB; 20])),
                            value: U256::from_u64(i as u64),
                            gas_limit: 21000,
                            gas_price: U256::from_u64(1_000_000_000),
                            nonce: 0,
                            data: vec![],
                            source_commitment: [0; 32],
                            sequence_number: i as u64,
                        };
                        bincode::serialize(&tx).unwrap()
                    })
                    .collect();

                b.iter(|| {
                    for tx in &txs {
                        let _ = sha256(black_box(tx));
                    }
                })
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_crypto,
    bench_mvcc,
    bench_dag,
    bench_jmt,
    bench_transactions,
    bench_throughput,
);

criterion_main!(benches);
