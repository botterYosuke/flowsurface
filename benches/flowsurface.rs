use criterion::{criterion_group, criterion_main, Criterion};

// ベンチマーク対象の関数をここに追加する。
// 例: チャート描画計算、ティックデータ処理、ローソク足集計など
//
// fn bench_example(c: &mut Criterion) {
//     c.bench_function("example", |b| b.iter(|| 1 + 1));
// }

fn placeholder(_c: &mut Criterion) {}

criterion_group!(benches, placeholder);
criterion_main!(benches);
