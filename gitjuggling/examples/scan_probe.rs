// one-off probe: does discover::discover hang on a tree like the parity seed?
use std::path::Path;
fn main() {
    let t = std::env::args().nth(1).unwrap();
    println!("walking {t} ...");
    let now = std::time::Instant::now();
    let local = gitjuggling::discover::discover(Path::new(&t)).expect("discover");
    println!(
        "walk done in {:?}, {} repos",
        now.elapsed(),
        local.iter().count()
    );
    for (_, r) in local.iter() {
        let now = std::time::Instant::now();
        let d = gitjuggling::git::is_dirty(&r.path).unwrap();
        println!(
            "  is_dirty({}) = {d} in {:?}",
            r.path.display(),
            now.elapsed()
        );
    }
    println!("SCAN COMPLETE");
}
