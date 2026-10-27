//! Settings export / import: one passphrase-encrypted file that carries the
//! frontend's settings (localStorage, collected by the webview) and the config
//! files under app_config_dir. Secrets never enter it — they stay in the OS
//! credential store of the machine they were typed on.
//!
//! On-disk format (JSON):
//!   { "format": "aria-settings", "version": 1,
//!     "kdf": { "alg": "argon2id", "m_kib", "t", "p", "salt" },
//!     "nonce": b64, "ciphertext": b64 }
//! The key is Argon2id(passphrase, salt); the ciphertext is ChaCha20-Poly1305
//! over the JSON `Bundle`, with the header (format, version, kdf) as associated
//! data, so editing the parameters breaks the tag instead of weakening the file.
//!
//! Import is two-phase: `settings_open` only decrypts and validates, the
//! frontend confirms with the user, and only `settings_apply` writes.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager};
use zeroize::Zeroizing;

use crate::{collectors, wallpaper};

const FORMAT: &str = "aria-settings";
const VERSION: u32 = 1;
const KDF_ALG: &str = "argon2id";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const MIN_PASSPHRASE: usize = 10;

/// Real settings are a few hundred KB at most; anything far larger is not ours.
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Export cost: 128 MiB, 4 passes, one lane — twice the memory of RFC 9106's
/// memory-constrained profile. About 0.25 s in a release build, paid once per
/// export or import, and every passphrase guess costs the same.
const KDF_DEFAULT: Kdf = Kdf { m_kib: 128 * 1024, t: 4, p: 1 };
/// Bounds for a file we are asked to open: a crafted header must not be able
/// to make ARIA allocate gigabytes or spin for minutes.
const KDF_MAX: Kdf = Kdf { m_kib: 256 * 1024, t: 10, p: 4 };
const KDF_MIN_M_KIB: u32 = 8 * 1024;

/// Config files that travel, by bundle name. Anything else in the config dir
/// (and everything in the data dir: history, screen time) stays put.
const CONFIGS: &[(&str, &str)] = &[
    ("mail", "mail.json"),
    ("calendar", "calendar.json"),
    ("discord", "discord.json"),
    ("weather", "weather.json"),
    ("wallpaper", "wallpaper.json"),
];

#[derive(Clone, Copy)]
struct Kdf {
    m_kib: u32,
    t: u32,
    p: u32,
}

/// The authenticated-but-readable part of the file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Header {
    format: String,
    version: u32,
    kdf: KdfHeader,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct KdfHeader {
    alg: String,
    m_kib: u32,
    t: u32,
    p: u32,
    salt: String,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    #[serde(flatten)]
    header: Header,
    nonce: String,
    ciphertext: String,
}

/// What the encrypted payload decodes to.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct Bundle {
    /// ARIA version that wrote it, for the user and for future migrations.
    pub app: String,
    pub exported: String,
    /// localStorage key → raw stored string. The frontend owns the allowlist.
    pub storage: BTreeMap<String, String>,
    /// Config name (see `CONFIGS`) → file contents.
    pub config: BTreeMap<String, Value>,
}

fn derive_key(passphrase: &str, kdf: &KdfHeader) -> Result<Zeroizing<[u8; 32]>, String> {
    let salt = B64.decode(&kdf.salt).map_err(|_| "damaged file (salt)".to_string())?;
    let params = Params::new(kdf.m_kib, kdf.t, kdf.p, Some(32)).map_err(|e| e.to_string())?;
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), &salt, key.as_mut())
        .map_err(|e| e.to_string())?;
    Ok(key)
}

fn aad(header: &Header) -> Vec<u8> {
    // Re-serialised from the parsed struct, so key order in the file is irrelevant.
    serde_json::to_vec(header).expect("header serialises")
}

fn check_passphrase(passphrase: &str) -> Result<(), String> {
    if passphrase.chars().count() < MIN_PASSPHRASE {
        return Err(format!("passphrase must be at least {MIN_PASSPHRASE} characters"));
    }
    Ok(())
}

fn seal_with(plaintext: &[u8], passphrase: &str, kdf: Kdf) -> Result<String, String> {
    check_passphrase(passphrase)?;
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut salt).map_err(|e| e.to_string())?;
    getrandom::getrandom(&mut nonce).map_err(|e| e.to_string())?;
    let header = Header {
        format: FORMAT.into(),
        version: VERSION,
        kdf: KdfHeader {
            alg: KDF_ALG.into(),
            m_kib: kdf.m_kib,
            t: kdf.t,
            p: kdf.p,
            salt: B64.encode(salt),
        },
    };
    let key = derive_key(passphrase, &header.kdf)?;
    let ciphertext = ChaCha20Poly1305::new(Key::from_slice(key.as_ref()))
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad: &aad(&header) })
        .map_err(|_| "encryption failed".to_string())?;
    let env = Envelope { header, nonce: B64.encode(nonce), ciphertext: B64.encode(ciphertext) };
    serde_json::to_string_pretty(&env).map_err(|e| e.to_string())
}

fn seal(plaintext: &[u8], passphrase: &str) -> Result<String, String> {
    seal_with(plaintext, passphrase, KDF_DEFAULT)
}

fn open(file: &str, passphrase: &str) -> Result<Zeroizing<Vec<u8>>, String> {
    let env: Envelope =
        serde_json::from_str(file).map_err(|_| "not an ARIA settings file".to_string())?;
    let h = &env.header;
    if h.format != FORMAT {
        return Err("not an ARIA settings file".into());
    }
    if h.version != VERSION {
        return Err(format!("settings file version {} is not supported; update ARIA", h.version));
    }
    let k = &h.kdf;
    if k.alg != KDF_ALG
        || !(KDF_MIN_M_KIB..=KDF_MAX.m_kib).contains(&k.m_kib)
        || !(1..=KDF_MAX.t).contains(&k.t)
        || !(1..=KDF_MAX.p).contains(&k.p)
    {
        return Err("settings file has unsupported encryption parameters".into());
    }
    let nonce = B64.decode(&env.nonce).map_err(|_| "damaged file (nonce)".to_string())?;
    if nonce.len() != NONCE_LEN {
        return Err("damaged file (nonce)".into());
    }
    let ciphertext = B64.decode(&env.ciphertext).map_err(|_| "damaged file".to_string())?;
    let key = derive_key(passphrase, k)?;
    // AEAD cannot tell a wrong key from a modified file, and neither can we.
    ChaCha20Poly1305::new(Key::from_slice(key.as_ref()))
        .decrypt(Nonce::from_slice(&nonce), Payload { msg: &ciphertext, aad: &aad(h) })
        .map(Zeroizing::new)
        .map_err(|_| "wrong passphrase, or the file has been modified".to_string())
}

/// Parse a config through the type its collector reads, so unknown fields are
/// dropped and malformed ones rejected before anything reaches the disk.
fn validate_config(name: &str, value: Value) -> Result<Value, String> {
    fn through<T: serde::de::DeserializeOwned + Serialize>(v: Value) -> Result<Value, String> {
        let typed: T = serde_json::from_value(v).map_err(|e| e.to_string())?;
        serde_json::to_value(typed).map_err(|e| e.to_string())
    }
    let checked = match name {
        "mail" => through::<collectors::email::MailConfig>(value),
        "calendar" => through::<collectors::calendar::CalendarConfig>(value),
        "discord" => through::<collectors::discord::DiscordConfig>(value),
        "weather" => through::<collectors::weather::WeatherConfig>(value),
        "wallpaper" => {
            let cfg: wallpaper::WallpaperConfig =
                serde_json::from_value(value).map_err(|e| e.to_string())?;
            serde_json::to_value(cfg.importable()).map_err(|e| e.to_string())
        }
        _ => return Err("unknown config".into()),
    };
    checked.map_err(|e| format!("{name}: {e}"))
}

/// Keep only what this build knows how to apply; report what was dropped.
fn validate_bundle(mut bundle: Bundle) -> (Bundle, Vec<String>) {
    let mut skipped = Vec::new();
    bundle.storage.retain(|k, _| {
        let ok = k.starts_with("aria-");
        if !ok {
            skipped.push(k.clone());
        }
        ok
    });
    let config = std::mem::take(&mut bundle.config);
    for (name, value) in config {
        match validate_config(&name, value) {
            Ok(v) => {
                bundle.config.insert(name, v);
            }
            Err(e) => skipped.push(e),
        }
    }
    (bundle, skipped)
}

fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn read_configs(dir: &Path) -> BTreeMap<String, Value> {
    CONFIGS
        .iter()
        .filter_map(|(name, file)| {
            let raw = std::fs::read_to_string(dir.join(file)).ok()?;
            let value = serde_json::from_str(&raw).ok()?;
            Some((name.to_string(), validate_config(name, value).ok()?))
        })
        .collect()
}

/// Owner-only, and never a half-written file at `path`: write beside it, then
/// rename over it.
fn write_private(path: &Path, contents: &[u8]) -> Result<(), String> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let result = opts
        .open(&tmp)
        .and_then(|mut f| f.write_all(contents).and_then(|_| f.sync_all()))
        .and_then(|_| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn settings_export(
    app: AppHandle,
    path: String,
    passphrase: String,
    storage: BTreeMap<String, String>,
) -> Result<(), String> {
    let passphrase = Zeroizing::new(passphrase);
    let bundle = Bundle {
        app: app.package_info().version.to_string(),
        exported: chrono::Local::now().to_rfc3339(),
        storage,
        config: read_configs(&config_dir(&app)?),
    };
    let plain = Zeroizing::new(serde_json::to_vec(&bundle).map_err(|e| e.to_string())?);
    let file = seal(&plain, &passphrase)?;
    write_private(Path::new(&path), file.as_bytes())
}

#[derive(Serialize)]
pub struct Opened {
    bundle: Bundle,
    /// Entries this build refused, so the UI can say what didn't come across.
    skipped: Vec<String>,
}

#[tauri::command]
pub fn settings_open(path: String, passphrase: String) -> Result<Opened, String> {
    let passphrase = Zeroizing::new(passphrase);
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_FILE_BYTES {
        return Err("file is too large to be ARIA settings".into());
    }
    let file = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let plain = open(&file, &passphrase)?;
    let bundle: Bundle =
        serde_json::from_slice(&plain).map_err(|_| "damaged settings payload".to_string())?;
    let (bundle, skipped) = validate_bundle(bundle);
    Ok(Opened { bundle, skipped })
}

/// Replace the config files with the imported set. A config the bundle does
/// not carry is removed, so the result matches the exporting machine. Inputs
/// are validated again: the webview is not trusted to have passed them on
/// unchanged.
#[tauri::command]
pub fn settings_apply(app: AppHandle, config: BTreeMap<String, Value>) -> Result<(), String> {
    let mut checked = BTreeMap::new();
    for (name, value) in config {
        checked.insert(name.clone(), validate_config(&name, value)?);
    }
    let dir = config_dir(&app)?;
    for (name, file) in CONFIGS {
        let path = dir.join(file);
        match checked.get(*name) {
            Some(v) => {
                let json = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
                write_private(&path, json.as_bytes())?;
            }
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    wallpaper::init(&app);
    collectors::email::email_refresh();
    collectors::calendar::cal_refresh();
    collectors::weather::weather_refresh();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // The cheapest parameters `open` accepts, so the tests stay fast.
    const FAST: Kdf = Kdf { m_kib: KDF_MIN_M_KIB, t: 1, p: 1 };
    const PASS: &str = "correct horse battery";

    fn sealed(plain: &[u8]) -> String {
        seal_with(plain, PASS, FAST).unwrap()
    }

    fn tamper(file: &str, f: impl FnOnce(&mut Value)) -> String {
        let mut v: Value = serde_json::from_str(file).unwrap();
        f(&mut v);
        v.to_string()
    }

    #[test]
    fn round_trips_with_the_right_passphrase() {
        let file = sealed(b"{\"a\":1}");
        assert_eq!(open(&file, PASS).unwrap().as_slice(), b"{\"a\":1}");
    }

    #[test]
    fn plaintext_is_not_in_the_file() {
        let file = sealed(b"imap.seznam.cz secret-notes");
        assert!(!file.contains("seznam"));
        assert!(!file.contains("secret-notes"));
    }

    #[test]
    fn every_export_uses_a_fresh_salt_and_nonce() {
        let a: Value = serde_json::from_str(&sealed(b"x")).unwrap();
        let b: Value = serde_json::from_str(&sealed(b"x")).unwrap();
        assert_ne!(a["kdf"]["salt"], b["kdf"]["salt"]);
        assert_ne!(a["nonce"], b["nonce"]);
    }

    #[test]
    fn wrong_passphrase_is_rejected() {
        let file = sealed(b"x");
        let err = open(&file, "wrong passphrase!").unwrap_err();
        assert!(err.contains("wrong passphrase"));
    }

    #[test]
    fn a_modified_ciphertext_is_rejected() {
        let file = sealed(b"some settings");
        let bad = tamper(&file, |v| {
            let mut ct = B64.decode(v["ciphertext"].as_str().unwrap()).unwrap();
            ct[0] ^= 1;
            v["ciphertext"] = B64.encode(ct).into();
        });
        assert!(open(&bad, PASS).is_err());
    }

    #[test]
    fn the_header_is_authenticated() {
        // With the right key, a changed header still fails: the AAD binds it,
        // not only the key derivation.
        let env: Envelope = serde_json::from_str(&sealed(b"x")).unwrap();
        let key = derive_key(PASS, &env.header.kdf).unwrap();
        let mut header = env.header.clone();
        header.version += 1;
        let nonce = B64.decode(&env.nonce).unwrap();
        let ct = B64.decode(&env.ciphertext).unwrap();
        let cipher = ChaCha20Poly1305::new(Key::from_slice(key.as_ref()));
        let ok = cipher.decrypt(Nonce::from_slice(&nonce), Payload { msg: &ct, aad: &aad(&env.header) });
        let bad = cipher.decrypt(Nonce::from_slice(&nonce), Payload { msg: &ct, aad: &aad(&header) });
        assert!(ok.is_ok());
        assert!(bad.is_err());
    }

    #[test]
    fn hostile_kdf_parameters_are_refused_before_hashing() {
        let file = sealed(b"x");
        let huge = tamper(&file, |v| v["kdf"]["m_kib"] = (4u64 * 1024 * 1024).into());
        assert!(open(&huge, PASS).unwrap_err().contains("unsupported"));
        let weak = tamper(&file, |v| v["kdf"]["m_kib"] = 8.into());
        assert!(open(&weak, PASS).unwrap_err().contains("unsupported"));
    }

    #[test]
    fn a_newer_version_asks_for_an_update() {
        let file = sealed(b"x");
        let newer = tamper(&file, |v| v["version"] = 2.into());
        assert!(open(&newer, PASS).unwrap_err().contains("update ARIA"));
    }

    #[test]
    fn short_passphrases_are_refused() {
        assert!(seal_with(b"x", "short", FAST).is_err());
    }

    #[test]
    fn default_parameters_are_within_the_import_bounds() {
        assert!(KDF_DEFAULT.m_kib >= KDF_MIN_M_KIB && KDF_DEFAULT.m_kib <= KDF_MAX.m_kib);
        assert!(KDF_DEFAULT.t <= KDF_MAX.t && KDF_DEFAULT.p <= KDF_MAX.p);
    }

    #[test]
    fn validation_drops_unknown_fields_and_configs() {
        let mut bundle = Bundle::default();
        bundle.storage.insert("aria-theme".into(), "nord".into());
        bundle.storage.insert("evil".into(), "x".into());
        bundle.config.insert(
            "mail".into(),
            serde_json::json!({ "accounts": [], "extra": "dropped" }),
        );
        bundle.config.insert("passwd".into(), serde_json::json!({}));
        let (out, skipped) = validate_bundle(bundle);
        assert_eq!(out.storage.keys().collect::<Vec<_>>(), ["aria-theme"]);
        assert_eq!(out.config["mail"], serde_json::json!({ "accounts": [] }));
        assert!(!out.config.contains_key("passwd"));
        assert_eq!(skipped.len(), 2);
    }

    #[test]
    fn a_malformed_config_is_skipped_not_written() {
        let mut bundle = Bundle::default();
        bundle.config.insert("weather".into(), serde_json::json!({ "place": 5 }));
        let (out, skipped) = validate_bundle(bundle);
        assert!(out.config.is_empty());
        assert!(skipped[0].starts_with("weather:"));
    }

    #[test]
    fn write_private_is_owner_only_and_leaves_no_part_file() {
        let dir = std::env::temp_dir().join(format!("aria-settings-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.aria");
        write_private(&path, b"one").unwrap();
        write_private(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert!(!dir.join("out.aria.part").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
