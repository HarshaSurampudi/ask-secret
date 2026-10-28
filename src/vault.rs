//! The encrypted vault: one file holding every scope (`shared` plus one per project).
//!
//! The file is XChaCha20-Poly1305 over JSON. The 32-byte key lives in the macOS
//! Keychain, or in a 0600 key file elsewhere (or wherever ASK_SECRET_KEY_FILE points).

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::project;

pub const SHARED: &str = "shared";
const MAGIC: &[u8] = b"ASKSECRET1\n";
const AAD: &[u8] = b"ask-secret vault v1";

#[derive(Serialize, Deserialize, Default)]
struct Data {
    version: u32,
    scopes: BTreeMap<String, Scope>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Scope {
    /// Where the project was last seen, for `ls`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub secrets: BTreeMap<String, Entry>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Entry {
    /// A value owned by this scope.
    Value { value: String, updated: u64 },
    /// Use the `shared` scope's value for the same key.
    Link,
}

pub struct Vault {
    path: PathBuf,
    key: [u8; 32],
    data: Data,
    _lock: File,
}

pub fn home() -> PathBuf {
    if let Some(h) = std::env::var_os("ASK_SECRET_HOME") {
        return PathBuf::from(h);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join(".local/share/ask-secret")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Vault {
    /// Opens (or creates) the vault, holding an exclusive lock until dropped.
    /// Don't keep one open while waiting on the user.
    pub fn open() -> Result<Self> {
        let dir = home();
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let lock = OpenOptions::new().create(true).write(true).mode(0o600).open(dir.join("vault.lock"))?;
        lock.lock_exclusive().context("locking the vault")?;

        let path = dir.join("vault.enc");
        let exists = path.exists();
        let key = master_key(&dir, exists)?;
        let mut data = if exists {
            let raw = fs::read(&path)?;
            decrypt(&key, &raw)?
        } else {
            Data { version: 1, ..Default::default() }
        };
        data.scopes.entry(SHARED.into()).or_insert_with(|| Scope {
            path: None,
            secrets: BTreeMap::new(),
        });
        Ok(Vault { path, key, data, _lock: lock })
    }

    pub fn save(&self) -> Result<()> {
        let plain = serde_json::to_vec(&self.data)?;
        let cipher = XChaCha20Poly1305::new((&self.key).into());
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let body = cipher
            .encrypt(&nonce, Payload { msg: &plain, aad: AAD })
            .map_err(|_| anyhow!("encrypting the vault failed"))?;
        let tmp = self.path.with_extension("enc.tmp");
        let mut f = OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(MAGIC)?;
        f.write_all(&nonce)?;
        f.write_all(&body)?;
        f.sync_all()?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    // MARK: scopes

    /// The project named by `--project`, or detected from the working directory.
    /// With `create`, a missing project is added.
    pub fn scope(&mut self, selector: Option<&str>, create: bool) -> Result<String> {
        let (name, path) = match selector {
            Some(sel) => (sel.to_string(), None),
            None => {
                let found = project::detect(&std::env::current_dir()?);
                if found.name == SHARED {
                    bail!("this folder is called \"shared\", which is reserved; pass --project NAME");
                }
                (found.name, Some(found.path))
            }
        };
        if selector.is_some() && name != SHARED && !create && !self.data.scopes.contains_key(&name) {
            bail!("no project named {name:?} (see `ask-secret ls`)");
        }
        self.ensure_project(&name, path.as_deref(), create);
        Ok(name)
    }

    /// Adds a project, or refreshes where it was last seen.
    pub fn ensure_project(&mut self, name: &str, path: Option<&str>, create: bool) {
        if let Some(s) = self.data.scopes.get_mut(name) {
            if path.is_some() {
                s.path = path.map(Into::into);
            }
        } else if create {
            self.data.scopes.insert(name.into(), Scope { path: path.map(Into::into), secrets: BTreeMap::new() });
        }
    }

    pub fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        if from == SHARED || to == SHARED {
            bail!("shared can't be renamed, and no project can be called shared");
        }
        if self.data.scopes.contains_key(to) {
            bail!("a project named {to:?} already exists");
        }
        let scope = self.data.scopes.remove(from).ok_or_else(|| anyhow!("no project named {from:?}"))?;
        self.data.scopes.insert(to.into(), scope);
        Ok(())
    }

    pub fn scopes(&self) -> impl Iterator<Item = (&String, &Scope)> {
        self.data.scopes.iter()
    }

    // MARK: secrets

    pub fn entry(&self, scope: &str, key: &str) -> Option<&Entry> {
        self.data.scopes.get(scope)?.secrets.get(key)
    }

    /// The value a key resolves to in a scope, following a link to `shared`.
    pub fn get(&self, scope: &str, key: &str) -> Option<&str> {
        match self.entry(scope, key)? {
            Entry::Value { value, .. } => Some(value),
            Entry::Link => match self.entry(SHARED, key)? {
                Entry::Value { value, .. } => Some(value),
                Entry::Link => None,
            },
        }
    }

    pub fn set(&mut self, scope: &str, key: &str, value: &str) {
        self.scope_mut(scope).secrets.insert(key.into(), Entry::Value { value: value.into(), updated: now() });
    }

    pub fn link(&mut self, scope: &str, key: &str) {
        self.scope_mut(scope).secrets.insert(key.into(), Entry::Link);
    }

    pub fn remove(&mut self, scope: &str, key: &str) -> bool {
        self.data.scopes.get_mut(scope).map(|s| s.secrets.remove(key).is_some()).unwrap_or(false)
    }

    /// Every key in a scope with its resolved value. Dangling links are left out.
    pub fn resolved(&self, scope: &str) -> BTreeMap<String, String> {
        let Some(s) = self.data.scopes.get(scope) else { return BTreeMap::new() };
        s.secrets.keys().filter_map(|k| Some((k.clone(), self.get(scope, k)?.to_string()))).collect()
    }

    /// (scope name/key, value) for every stored value, for leak scanning.
    pub fn all_values(&self) -> Vec<(String, String)> {
        let mut out = vec![];
        for (id, s) in &self.data.scopes {
            for (k, e) in &s.secrets {
                if let Entry::Value { value, .. } = e {
                    out.push((format!("{id}/{k}"), value.clone()));
                }
            }
        }
        out
    }

    fn scope_mut(&mut self, id: &str) -> &mut Scope {
        self.data.scopes.entry(id.into()).or_insert_with(|| Scope {
            path: None,
            secrets: BTreeMap::new(),
        })
    }
}

fn decrypt(key: &[u8; 32], raw: &[u8]) -> Result<Data> {
    if !raw.starts_with(MAGIC) || raw.len() < MAGIC.len() + 24 {
        bail!("the vault file is not an ask-secret vault");
    }
    let (nonce, body) = raw[MAGIC.len()..].split_at(24);
    let cipher = XChaCha20Poly1305::new(key.into());
    let plain = cipher
        .decrypt(XNonce::from_slice(nonce), Payload { msg: body, aad: AAD })
        .map_err(|_| anyhow!("can't decrypt the vault: wrong key, or the file is damaged"))?;
    Ok(serde_json::from_slice(&plain)?)
}

// MARK: master key

/// Loads the vault key. A new key is only made when there is no vault yet, so a
/// missing key never silently orphans existing secrets.
fn master_key(dir: &Path, vault_exists: bool) -> Result<[u8; 32]> {
    if let Some(p) = std::env::var_os("ASK_SECRET_KEY_FILE") {
        return file_key(Path::new(&p), vault_exists);
    }
    #[cfg(target_os = "macos")]
    {
        let _ = dir;
        keychain_key(vault_exists)
    }
    #[cfg(not(target_os = "macos"))]
    {
        file_key(&dir.join("vault.key"), vault_exists)
    }
}

fn new_key() -> [u8; 32] {
    XChaCha20Poly1305::generate_key(&mut OsRng).into()
}

fn parse_key(hex_text: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(hex_text.trim()).context("the vault key is not hex")?;
    bytes.try_into().map_err(|_| anyhow!("the vault key is not 32 bytes"))
}

fn file_key(path: &Path, vault_exists: bool) -> Result<[u8; 32]> {
    match fs::read_to_string(path) {
        Ok(text) => parse_key(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if vault_exists {
                bail!("the vault exists but its key file {} is missing", path.display());
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let key = new_key();
            let mut f = OpenOptions::new().create_new(true).write(true).mode(0o600).open(path)?;
            writeln!(f, "{}", hex::encode(key))?;
            Ok(key)
        }
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

#[cfg(target_os = "macos")]
fn keychain_key(vault_exists: bool) -> Result<[u8; 32]> {
    use security_framework::passwords::{get_generic_password, set_generic_password};
    const SERVICE: &str = "ask-secret";
    const ACCOUNT: &str = "vault-key";
    const NOT_FOUND: i32 = -25300;
    const INTERACTION_NOT_ALLOWED: i32 = -25308;

    match get_generic_password(SERVICE, ACCOUNT) {
        Ok(bytes) => parse_key(&String::from_utf8_lossy(&bytes)),
        Err(e) if e.code() == NOT_FOUND && !vault_exists => {
            let key = new_key();
            set_generic_password(SERVICE, ACCOUNT, hex::encode(key).as_bytes())
                .map_err(|e| anyhow!("saving the vault key to the Keychain failed: {e}"))?;
            Ok(key)
        }
        Err(e) if e.code() == NOT_FOUND => bail!("the vault exists but its key is missing from the Keychain"),
        Err(e) if e.code() == INTERACTION_NOT_ALLOWED => bail!(
            "the Keychain is locked (common over SSH). Run `security unlock-keychain` first, \
             or point ASK_SECRET_KEY_FILE at a key file"
        ),
        Err(e) => bail!("reading the vault key from the Keychain failed: {e}"),
    }
}
