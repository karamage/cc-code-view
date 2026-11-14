//! 保存された設計図の図を、すべて解析して配置できるか確かめる。
//! `CCV_SAMPLES=<blueprint の保存先> cargo test -p ccv-core --test diagram_samples -- --ignored --nocapture`
use ccv_core::diagram::{layout, parse, scene::FixedMeasure};

#[test]
#[ignore]
fn saved_diagrams_parse_and_lay_out() {
    let Ok(dir) = std::env::var("CCV_SAMPLES") else { return };
    let mut failures = 0;
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap();
        for d in v["diagrams"].as_array().unwrap() {
            let src = d["mermaid"].as_str().unwrap();
            match parse(src) {
                Ok(g) => {
                    let s = layout(&g, &FixedMeasure);
                    println!(
                        "ok {:?} {} nodes={} {}x{}",
                        entry.file_name(),
                        d["title"],
                        s.nodes.len(),
                        s.width as i32,
                        s.height as i32
                    );
                }
                Err(e) => {
                    failures += 1;
                    println!("FAIL {:?} {}: {e}\n{src}", entry.file_name(), d["title"]);
                }
            }
        }
    }
    assert_eq!(failures, 0);
}
