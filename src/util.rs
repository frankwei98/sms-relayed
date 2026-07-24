pub fn hostname() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

pub fn url_encode_path(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}
