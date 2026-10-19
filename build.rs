fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        for (binary, description) in [
            ("StrajaBackupSQL", "StrajaBackupSQL"),
            ("StrajaBackupSQL-Agent", "StrajaBackupSQL Agent"),
        ] {
            let output =
                std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join(binary);
            std::fs::create_dir_all(&output).expect("Resource directory");
            winresource::WindowsResource::new()
                .set_output_directory(output.to_str().unwrap())
                .set_icon("installer/assets/straja-sql.ico")
                .set("ProductName", "StrajaBackupSQL")
                .set("FileDescription", description)
                .set("InternalName", binary)
                .set("OriginalFilename", &format!("{binary}.exe"))
                .compile_for(binary)
                .expect("Windows resources");
        }
    }
}
