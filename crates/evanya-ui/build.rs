fn main() {
    // slint-build names the translation domain after the package. The
    // catalogs are the application's, lang/<language>/LC_MESSAGES/evanya.po.
    // SAFETY: the build script is single-threaded and nothing reads the
    // environment concurrently.
    unsafe { std::env::set_var("CARGO_PKG_NAME", "evanya") };

    // The interface sources and catalogs live at the repository root.
    let manifest_dir = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by Cargo"),
    );
    let root = manifest_dir.join("../..");
    let out_dir =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));

    // Application controls are painted from primitives in app.slint. Fluent is
    // only the deterministic implementation for the remaining layout/scroll
    // infrastructure and never selects a platform widget toolkit.
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        // A source phrase has one meaning throughout this application. Keeping
        // catalogs context-free avoids duplicate entries for shared controls.
        .with_default_translation_context(slint_build::DefaultTranslationContext::None)
        // Bundle the small gettext catalogs so desktop and mobile builds use
        // the same translations without depending on a system gettext install.
        .with_bundled_translations(root.join("lang"));

    // Compile the non-visual tray separately from app.slint. Slint otherwise
    // registers app.slint's embedded font through the tray's shared
    // globals, which creates an unintended native WindowAdapter for a
    // SystemTrayIcon-rooted component. Keep the same style and translation
    // configuration for both compilation units.
    let tray_dependencies = slint_build::compile_with_output_path(
        root.join("ui/tray.slint"),
        out_dir.join("tray.rs"),
        config.clone(),
    )
    .expect("failed to compile Slint tray UI");
    for dependency in tray_dependencies {
        println!("cargo:rerun-if-changed={}", dependency.display());
    }

    slint_build::compile_with_config(root.join("ui/app.slint"), config)
        .expect("failed to compile Slint UI");
    println!("cargo:rerun-if-changed={}", root.join("lang").display());
}
