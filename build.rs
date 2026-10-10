use std::fs;
use std::path::Path;

/// Embeds the display firmware when it has been built (`cargo build --profile dist` in
/// `firmware/`). Without it the app still builds; it just cannot install firmware.
fn embed_firmware() {
    println!("cargo:rustc-check-cfg=cfg(embedded_firmware)");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("firmware");
    let elf = root.join("target/riscv32imac-unknown-none-elf/dist/firmware");
    let manifest = root.join("Cargo.toml");
    println!("cargo:rerun-if-changed={}", elf.display());
    println!("cargo:rerun-if-changed={}", manifest.display());

    let version = fs::read_to_string(&manifest).ok().and_then(|text| {
        let line = text.lines().find(|line| line.trim_start().starts_with("version"))?;
        Some(line.split('"').nth(1)?.to_string())
    });
    if let (true, Some(version)) = (elf.is_file(), version) {
        println!("cargo:rustc-cfg=embedded_firmware");
        println!("cargo:rustc-env=COH3_FIRMWARE_ELF={}", elf.display());
        println!("cargo:rustc-env=COH3_FIRMWARE_VERSION={version}");
    }
}

fn main() {
    embed_firmware();

    // Icon and version details shown by Explorer, the Start menu and "Installed apps".
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=installer/coh3-display.ico");
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("installer/coh3-display.ico");
        resource.set("ProductName", "CoH3 Display");
        resource.set("FileDescription", "CoH3 Display");
        // Current-style buttons and text boxes in the dialogs, and sharp text on scaled screens.
        resource.set_manifest(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0"
                        processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    </windowsSettings>
  </application>
</assembly>"#,
        );
        if let Err(err) = resource.compile() {
            println!("cargo:warning=could not embed the icon: {err}");
        }
    }
}
