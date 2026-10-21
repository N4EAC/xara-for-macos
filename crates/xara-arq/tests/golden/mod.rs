use std::path::Path;

pub fn digest(b: &[u8]) -> String {
    let h = b.iter().fold(0x811c_9dc5u32, |h, &x| (h ^ u32::from(x)).wrapping_mul(0x0100_0193));
    format!("<{} bytes {h:08x}>", b.len())
}

pub fn check(file: &str, got: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(file);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    if got == want {
        return;
    }
    let new = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{file}.new"));
    std::fs::write(&new, got).unwrap();
    let (g, w): (Vec<&str>, Vec<&str>) = (got.lines().collect(), want.lines().collect());
    let i = g.iter().zip(&w).take_while(|(a, b)| a == b).count();
    let case = g[..i].iter().rev().find(|l| l.starts_with("## ")).unwrap_or(&"");
    panic!(
        "{file} differs at line {} ({case})\n got: {}\nwant: {}\nnew transcript: {}",
        i + 1,
        clip(g.get(i).unwrap_or(&"<end>")),
        clip(w.get(i).unwrap_or(&"<end>")),
        new.display()
    );
}

fn clip(s: &str) -> String {
    s.chars().take(400).collect()
}
