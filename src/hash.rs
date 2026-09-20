use serde::Serialize;

pub fn bytes(value: impl AsRef<[u8]>) -> String {
    blake3::hash(value.as_ref()).to_hex().to_string()
}

pub fn key<T: Serialize>(namespace: &str, value: &T) -> anyhow::Result<String> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"jevlint\0");
    hasher.update(namespace.as_bytes());
    hasher.update(b"\0");
    serde_json::to_writer(&mut hasher, value)?;
    Ok(hasher.finalize().to_hex().to_string())
}
