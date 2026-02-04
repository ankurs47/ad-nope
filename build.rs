use std::env;
use std::fs;
use std::path::Path;

fn main() {
    // Rerun if ui/ changes
    println!("cargo:rerun-if-changed=ui/");

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("ui");
    let src_path = Path::new("ui");

    // Create destination directory
    if dest_path.exists() {
        fs::remove_dir_all(&dest_path).unwrap();
    }
    fs::create_dir_all(&dest_path).unwrap();

    let profile = env::var("PROFILE").unwrap();
    let should_minify = profile == "release";

    if src_path.exists() {
        for entry in fs::read_dir(src_path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();

            if path.is_file() {
                let file_name = path.file_name().unwrap();
                let dest_file = dest_path.join(file_name);

                if should_minify && path.extension().is_some_and(|e| e == "html") {
                    let source = fs::read(&path).unwrap();
                    let mut cfg = minify_html::Cfg::new();
                    cfg.minify_css = true;
                    cfg.minify_js = true;
                    cfg.keep_comments = false;

                    let minified = minify_html::minify(&source, &cfg);
                    fs::write(&dest_file, minified).unwrap();
                } else {
                    fs::copy(&path, &dest_file).unwrap();
                }
            }
        }
    }
}
