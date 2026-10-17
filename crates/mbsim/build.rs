//! On macOS, embeds an Info.plist in the executable (section `__TEXT,__info_plist`): it gives the
//! reason for microphone access that macOS shows when the host microphone is switched on, which
//! a command-line tool needs when it is not started from an app that has one (Terminal, VS Code).

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>io.github.matthiashertelarm.mbsim</string>
	<key>CFBundleName</key>
	<string>mbsim</string>
	<key>CFBundleShortVersionString</key>
	<string>{version}</string>
	<key>CFBundleVersion</key>
	<string>{version}</string>
	<key>NSMicrophoneUsageDescription</key>
	<string>The simulated micro:bit hears this computer's microphone while its host microphone input is switched on.</string>
</dict>
</plist>
"#
    );
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("Info.plist");
    std::fs::write(&out, plist).unwrap();
    for arg in ["-sectcreate", "__TEXT", "__info_plist", out.to_str().expect("UTF-8 build path")] {
        println!("cargo:rustc-link-arg-bin=mbsim=-Xlinker");
        println!("cargo:rustc-link-arg-bin=mbsim={arg}");
    }
}
