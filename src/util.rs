use std::collections::HashSet;

pub fn hostname() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

pub fn url_encode_path(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

pub fn url_encode_form(s: &str) -> String {
    urlencoding::encode(s).replace("%20", "+")
}

#[allow(dead_code)]
pub fn dedup(vec: &mut Vec<String>) {
    let mut seen = HashSet::new();
    vec.retain(|s| seen.insert(s.clone()));
}
