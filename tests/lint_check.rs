use std::process::Command;

#[test]
fn test_clippy_no_warnings() {
    let status = Command::new("cargo")
        .args(&["clippy", "--all-targets", "--", "-D", "warnings"])
        .status()
        .expect("Failed to run cargo clippy");

    assert!(status.success(), "Clippy found warnings!");
}

#[test]
fn test_fmt_check() {
    let status = Command::new("cargo")
        .args(&["fmt", "--", "--check"])
        .status()
        .expect("Failed to run cargo fmt");

    assert!(
        status.success(),
        "Code is not formatted correctly! Run `cargo fmt`."
    );
}
