//! Print the auto-detect features of each image in a directory.
fn main() {
    let dir = std::env::args().nth(1).expect("usage: import_features DIR");
    let mut paths: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        let Ok(img) = image::open(&p) else { continue };
        let img = img.to_rgba8();
        let f = acidtrip_io::import::features(&img);
        let a = acidtrip_io::import::analyze(&img);
        println!(
            "{:<22} ink {:.3}  flat {:.2}  chroma {:5.1}  paper {:.2}  few {:.2}  outlined {:.2}  pixel {}",
            p.file_stem().unwrap().to_string_lossy(),
            f.ink,
            f.flat,
            f.chroma,
            f.paper,
            f.few,
            f.outlined,
            a.pixel_art
        );
    }
}
