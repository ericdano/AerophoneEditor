//! For Windows builds only: embed the app icon and version details in the
//! .exe (shown in Explorer, the taskbar and Properties ▸ Details).
//! Cross-compiling from macOS needs mingw-w64 (`brew install mingw-w64`).

use std::{env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=assets/icons/logo.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let mut parts: Vec<u16> = version.split('.').filter_map(|p| p.parse().ok()).collect();
    parts.resize(4, 0);
    let numeric = parts
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let icon = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("assets/icons/logo.ico")
        .display()
        .to_string()
        .replace('\\', "/");

    let rc = format!(
        r#"1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "Eric Dannewitz"
      VALUE "FileDescription", "Aerophone Editor - scene and tone editor for the Roland Aerophone Pro AE-30"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "aerophoneeditor"
      VALUE "LegalCopyright", "Copyright 2027 Eric Dannewitz. PolyForm Strict 1.0.0."
      VALUE "OriginalFilename", "AerophoneEditor.exe"
      VALUE "ProductName", "Aerophone Editor"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let out = Path::new(&env::var("OUT_DIR").unwrap()).join("aerophoneeditor.rc");
    fs::write(&out, rc).unwrap();
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_required()
        .expect("could not embed the Windows icon (is mingw-w64 installed?)");
}
