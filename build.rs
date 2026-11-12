// Embed the application icon and version information into the executable.
//
// The Windows SDK is located by scanning the standard install directory instead
// of shelling out to `reg.exe`, so the build works in restricted environments.
// If no resource compiler is available the build still succeeds; the app simply
// keeps the default file icon.

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    #[cfg(windows)]
    {
        if let Some(rc) = find_rc() {
            env::set_var("RC", &rc);
            if let Err(error) = embed_resource::compile("assets/app.rc", embed_resource::NONE)
                .manifest_optional()
            {
                println!("cargo:warning=could not embed the Windows icon: {error:?}");
            }
        } else {
            println!("cargo:warning=rc.exe not found; building without a custom file icon");
        }
    }
}

fn find_rc() -> Option<PathBuf> {
    if let Ok(value) = env::var("RC") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Some(path);
        }
    }

    // Standard Windows SDK layout: ...\Windows Kits\10\bin\<version>\x64\rc.exe
    let base = PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(base)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.join("x64").join("rc.exe").is_file())
        .collect();
    versions.sort();
    versions.pop().map(|path| path.join("x64").join("rc.exe"))
}
