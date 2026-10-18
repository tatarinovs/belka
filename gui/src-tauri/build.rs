fn main() {
    // The cdylib is for Android; on Windows nothing links against it, and
    // the MSVC linker's note about its import library, localized and so not
    // filtered by rustc, shows up as a warning in every release build.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-cdylib-link-arg=/NOIMPLIB");
        println!("cargo:rustc-cdylib-link-arg=/NOEXP");
    }
    tauri_build::build()
}
