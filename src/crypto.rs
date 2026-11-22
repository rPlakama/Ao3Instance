//! Secret storage: AES-256-GCM with purpose binding (AAD).
//!
//! Ported from MUSIKA (`src/crypto.rs`) with two deliberate strengthenings:
//! each ciphertext is bound to its purpose, so a cookie jar cannot be decrypted
//! as a password even though both use the same key; and the key file's parent
//! directory is *validated*, never silently chmodded.
//!
//! File format: 32 random bytes, hex-encoded, mode `0600`, created once with
//! `create_new` (no overwrite race). Ciphertext format: `hex(nonce ‖ tag ‖ ct)`.
//!
//! Zeroization scope: key material and `SecretString` contents are wiped on drop.
//! `decrypt` returns an ordinary `String` because callers feed it to `serde_json`;
//! those callers must call [`wipe_string`] when done (the jar does).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

use openssl::rand::rand_bytes;
use openssl::symm::{Cipher, decrypt_aead, encrypt_aead};

use crate::error::{Error, Result};

pub const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// A racing reader may observe a zero-length key file between `create_new` and
/// `write_all`; retry briefly instead of failing to start.
const LOAD_RETRIES: u32 = 20;
const LOAD_RETRY_DELAY: Duration = Duration::from_millis(25);

/// Purpose strings. Every encryption names its use; decryption must name the same.
pub const AAD_JAR: &[u8] = b"ao3/jar";
pub const AAD_PASSWORD: &[u8] = b"ao3/password";

/// A 256-bit key for encrypting secrets before they are written to disk.
///
/// Generated on first use under `data_dir/key` (mode `0600`) and loaded once per
/// process by the caller. Zeroized on drop.
pub struct SecretKey([u8; KEY_LEN]);

impl Drop for SecretKey {
    fn drop(&mut self) {
        wipe_bytes(&mut self.0);
    }
}

impl SecretKey {
    /// Loads the key, creating it (and its parent directory, if absent) if needed.
    ///
    /// An existing parent directory is validated: it must be a real directory,
    /// owned by the current user, and not group/world-writable. It is never
    /// chmodded — tightening an unrelated directory is not ours to do.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| {
                Error::Io(format!("key file path {} must be absolute", path.display()))
            })?;

        if path.exists() {
            // Validate the directory and the key file's own permissions even when
            // the key already exists: a key inside a group-writable directory (or
            // a permissions-drifted file) is not trustworthy.
            verify_private_dir(parent)?;
            verify_private_key(path)?;
            return Self::load_with_retry(path);
        }
        ensure_private_parent(parent)?;

        let mut secret = Self([0u8; KEY_LEN]);
        rand_bytes(&mut secret.0).map_err(|e| Error::Crypto(format!("random key: {e}")))?;

        let mut encoded = hex_encode(&secret.0);
        let created = write_private(path, encoded.as_bytes());
        wipe_string(&mut encoded);
        match created? {
            true => Ok(secret),
            // Another process won the create race; use its key.
            false => Self::load_with_retry(path),
        }
    }

    /// Loads an existing key file. Strict: wrong length or bad hex is an error,
    /// never a silent regeneration (that would strand every stored secret).
    pub(crate) fn load_from(path: &Path) -> Result<Self> {
        let mut raw = fs::read(path)
            .map_err(|e| Error::Io(format!("read key file {}: {e}", path.display())))?;
        let decoded = std::str::from_utf8(&raw)
            .map_err(|_| Error::Crypto("key file is not valid UTF-8".into()))
            .and_then(|text| hex_decode(text.trim()));
        wipe_bytes(&mut raw);

        let mut bytes = decoded?;
        let key = <[u8; KEY_LEN]>::try_from(bytes.as_slice())
            .map(Self)
            .map_err(|_| {
                Error::Crypto(format!(
                    "key file {} must contain exactly {KEY_LEN} bytes",
                    path.display()
                ))
            });
        wipe_bytes(&mut bytes);
        key
    }

    /// Encrypts `plaintext` bound to `aad`; returns `hex(nonce ‖ tag ‖ ct)`.
    pub fn encrypt(&self, aad: &[u8], plaintext: &str) -> Result<String> {
        let mut nonce = [0u8; NONCE_LEN];
        rand_bytes(&mut nonce).map_err(|e| Error::Crypto(format!("random nonce: {e}")))?;

        let mut tag = [0u8; TAG_LEN];
        let ciphertext = encrypt_aead(
            Cipher::aes_256_gcm(),
            &self.0,
            Some(&nonce),
            aad,
            plaintext.as_bytes(),
            &mut tag,
        )
        .map_err(|e| Error::Crypto(format!("encrypt: {e}")))?;

        let mut blob = Vec::with_capacity(NONCE_LEN + TAG_LEN + ciphertext.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&tag);
        blob.extend_from_slice(&ciphertext);
        Ok(hex_encode(&blob))
    }

    /// Decrypts a blob produced by [`SecretKey::encrypt`] with the same `aad`.
    ///
    /// Returns an ordinary `String`: wipers must call [`wipe_string`].
    pub fn decrypt(&self, aad: &[u8], encoded: &str) -> Result<String> {
        let mut blob = hex_decode(encoded)?;
        let plaintext = (|| {
            if blob.len() < NONCE_LEN + TAG_LEN {
                return Err(Error::Crypto("stored secret is truncated".into()));
            }
            let (nonce, rest) = blob.split_at(NONCE_LEN);
            let (tag, ciphertext) = rest.split_at(TAG_LEN);
            let plaintext = decrypt_aead(
                Cipher::aes_256_gcm(),
                &self.0,
                Some(nonce),
                aad,
                ciphertext,
                tag,
            )
            .map_err(|_| Error::Crypto("secret failed authentication".into()))?;
            String::from_utf8(plaintext)
                .map_err(|_| Error::Crypto("decrypted secret is not valid UTF-8".into()))
        })();
        wipe_bytes(&mut blob);
        plaintext
    }

    fn load_with_retry(path: &Path) -> Result<Self> {
        let mut last = None;
        for _ in 0..LOAD_RETRIES {
            match Self::load_from(path) {
                Ok(key) => return Ok(key),
                Err(err) => last = Some(err),
            }
            sleep(LOAD_RETRY_DELAY);
        }
        Err(last.unwrap_or_else(|| {
            Error::Crypto(format!(
                "key file {} did not become readable",
                path.display()
            ))
        }))
    }
}

/// A string whose contents must never reach a log line.
///
/// `Debug` and `Display` both print `[redacted]`; the value is zeroized on drop.
/// Callers must use [`SecretString::expose`] deliberately.
pub struct SecretString {
    inner: String,
}

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            inner: value.into(),
        }
    }

    /// Borrow the plaintext. The call site is where the leak risk lives.
    pub fn expose(&self) -> &str {
        &self.inner
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        wipe_string(&mut self.inner);
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

impl std::fmt::Display for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

/// Best-effort wipe of a heap string. NUL bytes are valid UTF-8, so the `String`
/// invariant holds while the bytes are overwritten and the value is then dropped.
pub(crate) fn wipe_string(text: &mut String) {
    // SAFETY: `as_mut_vec` is the documented way to mutate a String's bytes; we
    // only write zeros (valid UTF-8) and drop immediately.
    let bytes = unsafe { text.as_mut_vec() };
    wipe_bytes(bytes);
}

fn wipe_bytes(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        // SAFETY: `byte` is a valid, aligned mutable reference into `bytes`.
        // The volatile write stops the optimizer from eliding the wipe.
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

fn ensure_private_parent(dir: &Path) -> Result<()> {
    if dir.exists() {
        return verify_private_dir(dir);
    }
    fs::create_dir_all(dir)
        .map_err(|e| Error::Io(format!("create data dir {}: {e}", dir.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
            .map_err(|e| Error::Io(format!("chmod data dir {}: {e}", dir.display())))?;
    }
    Ok(())
}

#[cfg(unix)]
fn verify_private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let meta = fs::symlink_metadata(dir)
        .map_err(|e| Error::Io(format!("stat data dir {}: {e}", dir.display())))?;
    if !meta.file_type().is_dir() {
        return Err(Error::Io(format!(
            "data dir {} is not a directory (symlinks are refused)",
            dir.display()
        )));
    }
    // SAFETY: `geteuid(2)` takes no pointers, has no preconditions, and always
    // succeeds returning the effective user ID.
    let euid = unsafe { libc::geteuid() };
    if meta.uid() != euid {
        return Err(Error::Io(format!(
            "data dir {} is not owned by the current user",
            dir.display()
        )));
    }
    if meta.mode() & 0o022 != 0 {
        return Err(Error::Io(format!(
            "data dir {} is group/world writable",
            dir.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn verify_private_key(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let meta = fs::symlink_metadata(path)
        .map_err(|e| Error::Io(format!("stat key file {}: {e}", path.display())))?;
    if !meta.file_type().is_file() {
        return Err(Error::Io(format!(
            "key file {} is not a regular file (symlinks are refused)",
            path.display()
        )));
    }
    // SAFETY: `geteuid(2)` takes no pointers, has no preconditions, and always
    // succeeds returning the effective user ID.
    let euid = unsafe { libc::geteuid() };
    if meta.uid() != euid {
        return Err(Error::Io(format!(
            "key file {} is not owned by the current user",
            path.display()
        )));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(Error::Io(format!(
            "key file {} is group/world accessible (expected 0600)",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn verify_private_key(path: &Path) -> Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        Err(Error::Io(format!(
            "key file {} is not a regular file",
            path.display()
        )))
    }
}

#[cfg(not(unix))]
fn verify_private_dir(dir: &Path) -> Result<()> {
    if dir.is_dir() {
        Ok(())
    } else {
        Err(Error::Io(format!(
            "data dir {} is not a directory",
            dir.display()
        )))
    }
}

fn write_private(path: &Path, data: &[u8]) -> Result<bool> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(data)
                .map_err(|e| Error::Io(format!("write key file {}: {e}", path.display())))?;
            file.sync_all()
                .map_err(|e| Error::Io(format!("sync key file {}: {e}", path.display())))?;
            Ok(true)
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(err) => Err(Error::Io(format!(
            "create key file {}: {err}",
            path.display()
        ))),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(text: &str) -> Result<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Err(Error::Crypto("hex input has an odd length".into()));
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks(2) {
        let hi = hex_nibble(pair[0])?;
        let lo = hex_nibble(pair[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(Error::Crypto("hex input has a non-hex character".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A fresh private directory per test; the key lives *inside* it, so no test
    /// ever chmods `$TMPDIR` (the previous version did, and failed under /tmp).
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ao3i-key-{}-{}-{}",
            std::process::id(),
            tag,
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn key(tag: &str) -> (SecretKey, PathBuf, PathBuf) {
        let dir = temp_dir(tag);
        let path = dir.join("key");
        let key = SecretKey::load_or_create(&path).unwrap();
        (key, dir, path)
    }

    fn cleanup(dir: PathBuf) {
        drop(fs::remove_dir_all(dir));
    }

    #[test]
    fn round_trip_with_same_aad() {
        let (key, dir, path) = key("roundtrip");
        let cipher = key.encrypt(AAD_JAR, "hello").unwrap();
        assert_ne!(cipher, "hello");
        assert_eq!(key.decrypt(AAD_JAR, &cipher).unwrap(), "hello");
        assert!(path.exists());
        cleanup(dir);
    }

    #[test]
    fn purpose_binding_prevents_cross_use() {
        let (key, dir, _) = key("aad");
        let cipher = key.encrypt(AAD_JAR, "cookie").unwrap();
        assert!(matches!(
            key.decrypt(AAD_PASSWORD, &cipher),
            Err(Error::Crypto(_))
        ));
        cleanup(dir);
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let (key, dir, _) = key("tamper");
        let cipher = key.encrypt(AAD_JAR, "hello").unwrap();
        let mut chars: Vec<char> = cipher.chars().collect();
        let idx = chars.len() - 1;
        chars[idx] = if chars[idx] == '0' { '1' } else { '0' };
        let tampered: String = chars.into_iter().collect();
        assert!(key.decrypt(AAD_JAR, &tampered).is_err());
        cleanup(dir);
    }

    #[test]
    fn nonces_differ_between_encryptions() {
        let (key, dir, _) = key("nonce");
        assert_ne!(
            key.encrypt(AAD_JAR, "same").unwrap(),
            key.encrypt(AAD_JAR, "same").unwrap()
        );
        cleanup(dir);
    }

    #[test]
    fn key_file_is_reusable_and_private() {
        let (key, dir, path) = key("reuse");
        let cipher = key.encrypt(AAD_JAR, "persisted").unwrap();
        drop(key);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode & 0o077, 0, "key file must not be group/world readable");
        }

        let reloaded = SecretKey::load_or_create(&path).unwrap();
        assert_eq!(reloaded.decrypt(AAD_JAR, &cipher).unwrap(), "persisted");
        cleanup(dir);
    }

    #[test]
    fn relative_key_paths_are_refused() {
        let err = match SecretKey::load_or_create(std::path::Path::new("key")) {
            Err(err) => err,
            Ok(_) => panic!("a relative key path must be refused"),
        };
        assert!(matches!(err, Error::Io(_)), "got {err:?}");
    }

    #[test]
    fn bad_key_files_are_rejected_not_regenerated() {
        let dir = temp_dir("bad");
        let bad_hex = dir.join("bad-hex");
        fs::write(&bad_hex, "zzzz").unwrap();
        assert!(matches!(
            SecretKey::load_from(&bad_hex),
            Err(Error::Crypto(_))
        ));

        let short = dir.join("short");
        fs::write(&short, "0011").unwrap();
        assert!(matches!(
            SecretKey::load_from(&short),
            Err(Error::Crypto(_))
        ));

        let empty = dir.join("empty");
        fs::write(&empty, "").unwrap();
        assert!(matches!(
            SecretKey::load_from(&empty),
            Err(Error::Crypto(_))
        ));
        cleanup(dir);
    }

    #[test]
    fn group_writable_data_dir_is_refused() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = temp_dir("loose");
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).unwrap();
            // SecretKey deliberately has no Debug (it holds key material), so
            // unwrap_err/unwrap cannot be used here.
            let err = match SecretKey::load_or_create(&dir.join("key")) {
                Err(err) => err,
                Ok(_) => panic!("group-writable data dir must be refused"),
            };
            assert!(matches!(err, Error::Io(_)), "got {err:?}");
            // The directory must not have been chmodded by us.
            let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o777, "existing dir must not be tightened");
            cleanup(dir);
        }
    }

    #[test]
    fn existing_key_in_a_group_writable_dir_is_refused() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = temp_dir("loose-existing");
            let key_path = dir.join("key");
            let valid = "01".repeat(KEY_LEN);
            fs::write(&key_path, valid).unwrap();
            fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600)).unwrap();
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).unwrap();

            let err = match SecretKey::load_or_create(&key_path) {
                Err(err) => err,
                Ok(_) => panic!("loose directory must be refused even when the key exists"),
            };
            assert!(matches!(err, Error::Io(_)), "got {err:?}");
            cleanup(dir);
        }
    }

    #[test]
    fn permissions_drifted_key_file_is_refused() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = temp_dir("loose-key");
            let key_path = dir.join("key");
            let _key = SecretKey::load_or_create(&key_path).unwrap();
            fs::set_permissions(&key_path, fs::Permissions::from_mode(0o644)).unwrap();

            let err = match SecretKey::load_or_create(&key_path) {
                Err(err) => err,
                Ok(_) => panic!("0644 key file must be refused"),
            };
            assert!(matches!(err, Error::Io(_)), "got {err:?}");
            cleanup(dir);
        }
    }

    #[test]
    fn partially_written_key_is_waited_for() {
        let dir = temp_dir("race");
        let path = dir.join("key");
        let valid = "01".repeat(KEY_LEN);

        // Simulate the create_new loser observing a zero-length file first.
        // The file must already be 0600 or the (correct) permission check fires.
        fs::write(&path, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let writer_path = path.clone();
        let writer = std::thread::spawn(move || {
            sleep(Duration::from_millis(10));
            fs::write(writer_path, valid).unwrap();
        });

        let key = SecretKey::load_or_create(&path).unwrap();
        writer.join().unwrap();
        assert_eq!(key.encrypt(AAD_JAR, "x").unwrap().len() % 2, 0);
        cleanup(dir);
    }

    #[test]
    fn truncated_secret_is_rejected() {
        let (key, dir, _) = key("truncated");
        assert!(matches!(key.decrypt(AAD_JAR, "00"), Err(Error::Crypto(_))));
        cleanup(dir);
    }

    #[test]
    fn secret_string_never_prints() {
        let secret = SecretString::new("hunter2");
        assert_eq!(format!("{secret:?}"), "[redacted]");
        assert_eq!(format!("{secret}"), "[redacted]");
        assert_eq!(secret.expose(), "hunter2");
    }

    #[test]
    fn hex_round_trip() {
        let bytes: Vec<u8> = (0u8..=255).collect();
        assert_eq!(hex_decode(&hex_encode(&bytes)).unwrap(), bytes);
        assert!(hex_decode("abc").is_err());
        assert!(hex_decode("gg").is_err());
    }
}
