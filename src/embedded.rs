pub fn get_bundle(arch: &str) -> Option<&'static [u8]> {
    match arch {
        "linux/amd64" => Some(include_bytes!(
            "../bundles/openssh-bundle-linux/amd64.tar.xz"
        )),
        "linux/arm64" => Some(include_bytes!(
            "../bundles/openssh-bundle-linux/arm64.tar.xz"
        )),
        _ => None,
    }
}
