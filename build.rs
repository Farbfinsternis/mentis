#[path = "src/icon.rs"]
mod icon;

fn main() {
    println!("cargo:rerun-if-env-changed=VULKAN_SDK");
    // The Vulkan module ggml-vulkan.dll needs the SDK (headers, glslc) to build. mentis.exe itself
    // does not link vulkan-1: dynamic-backends loads the module at runtime if present.
    if std::env::var_os("CARGO_FEATURE_VULKAN").is_some()
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
    {
        let sdk = std::env::var_os("VULKAN_SDK").expect(
            "VULKAN_SDK must point to the Vulkan SDK (or build with --no-default-features for CPU only)",
        );
        assert!(
            std::path::Path::new(&sdk)
                .join("Bin")
                .join("glslc.exe")
                .is_file(),
            "glslc.exe missing in the Vulkan SDK"
        );
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        resources();
    }
}

/// Program icon, manifest and version info for mentis.exe.
fn resources() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=mentis.manifest");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let ico = out.join("mentis.ico");
    std::fs::write(
        &ico,
        icon::ico(&[16, 20, 24, 32, 40, 48, 64, 256], icon::BLUE),
    )
    .unwrap();
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("mentis.manifest");

    let version = env!("CARGO_PKG_VERSION");
    let numbers: Vec<&str> = version.split(['.', '-']).take(3).collect();
    let numeric = format!("{},0", numbers.join(","));
    let path = |p: &std::path::Path| p.display().to_string().replace('\\', "\\\\");
    let rc = format!(
        r#"1 ICON "{ico}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "Mentis"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "mentis"
      VALUE "OriginalFilename", "mentis.exe"
      VALUE "ProductName", "Mentis"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        ico = path(&ico),
        manifest = path(&manifest),
    );
    let rc_path = out.join("mentis.rc");
    std::fs::write(&rc_path, rc).unwrap();
    embed_resource::compile(&rc_path, embed_resource::NONE)
        .manifest_required()
        .unwrap();
}
