fn main() {
    #[cfg(windows)]
    cc::Build::new()
        .cpp(true)
        .file("native/process_loopback.cpp")
        .flag_if_supported("/std:c++17")
        .compile("process_loopback_native");

    tauri_build::build()
}
