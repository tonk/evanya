use cfg_aliases::cfg_aliases;

fn main() {
    // Shared code selects behaviour by capability, never by naming a mobile
    // operating system. A new platform joins these groups here (and in the
    // core's build.rs) instead of editing call sites. See platform/README.md.
    cfg_aliases! {
        mobile: { any(target_os = "android", target_os = "ios", target_env = "ohos") },
        desktop: { not(mobile) },
        // OpenHarmony reports target_os = "linux" without a desktop session,
        // portals, X11 or a Secret Service.
        linux_desktop: { all(desktop, target_os = "linux", not(target_env = "ohos")) },
        glibc: { all(target_os = "linux", target_env = "gnu") },
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        cc::Build::new()
            .file("platform/ios/Documents.m")
            .file("platform/ios/PdfPreview.m")
            .file("platform/ios/AppIcon.m")
            .flag("-fobjc-arc")
            .compile("evanya_documents");
        println!("cargo:rustc-link-lib=framework=UIKit");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rerun-if-changed=platform/ios/Documents.m");
        println!("cargo:rerun-if-changed=platform/ios/PdfPreview.m");
        println!("cargo:rerun-if-changed=platform/ios/AppIcon.m");
    }
    if std::env::var_os("CARGO_CFG_TARGET_OS").as_deref() == Some(std::ffi::OsStr::new("windows")) {
        winresource::WindowsResource::new()
            .set_icon("resources/app-icon/evanya.ico")
            .compile()
            .expect("failed to embed the Windows application icon");
        println!("cargo:rerun-if-changed=resources/app-icon/evanya.ico");
    }
}
