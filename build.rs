//! Gives the Windows .exe its icon and description. (macOS gets its icon from
//! the .app bundle the release workflow builds.)

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("FileDescription", "Blacksmith — 工程文件医生");
        res.set("ProductName", "Blacksmith");
        res.compile().expect("embedding the Windows icon");
    }
}
