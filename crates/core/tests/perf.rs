//! 集計の速さを確かめる。`CCV_PERF_REPO` に大きなリポジトリを指定して実行する。
//! CCV_PERF_REPO=$PWD/target/stress/r10k cargo test --release -p ccv-core --test perf -- --ignored --nocapture
use ccv_core::index::build_index;
use std::time::Instant;

#[test]
#[ignore = "大きなリポジトリが要る"]
fn indexes_ten_thousand_files_within_target() {
    let repo = std::env::var("CCV_PERF_REPO").expect("CCV_PERF_REPO");
    let start = Instant::now();
    let index = build_index(std::path::Path::new(&repo));
    let elapsed = start.elapsed();
    println!("files={} edges={} elapsed={:?}", index.files.len(), index.edges.len(), elapsed);
    assert!(index.files.len() >= 10_000);
    assert!(elapsed.as_secs() < 10, "{elapsed:?}");
}
