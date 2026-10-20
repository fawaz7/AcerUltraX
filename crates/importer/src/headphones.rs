//! Headphone models: Windows' DTS app lets the user pick their headphones, which loads a matching headphone EQ
//! into the DTS engine. The EQs and their names come from the user's Acer downloads:
//!
//! - the DTS driver package: `dts_apo4_oem_offline_hptuning_acer.db` (EQs);
//! - the DTS app packages ("XPERI DTS Utility"): `offlinedatabase_ui_*.db` (names, plus Bluetooth/USB device
//!   names for automatic matching), `offlinedatabase_tuning_*.db` (more EQs) and the app's DLLs.
//!
//! The EQs are stored encrypted; the app decrypts them with a key inside its own DLL. The importer finds that
//! key in the user's copy of the DLL (it is not part of AcerUltraX) and writes the plain EQs to
//! `dts/headphones/`, with `catalog.json` listing the models.

use std::collections::BTreeMap;
use std::path::Path;

use aes::Aes128;
use aes::cipher::{BlockCipherDecrypt, KeyInit};
use serde::{Deserialize, Serialize};

use crate::sources::Entry;
use crate::{Error, io};

/// Size of a headphone EQ: one DTS engine tuning blob.
const EQ_SIZE: usize = 0x2d20;
pub(crate) const KEY_DLLS: &[&str] = &["DTSSoundUnbound2.dll", "DtsInc.DtsXUltra.dll", "DTSApo4xWinRTComponent.dll"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Headphone {
    pub id: String,
    pub brand: String,
    pub name: String,
    /// e.g. "Over-Ear", "In-Ear".
    pub style: String,
    /// Connections it has an EQ for: "line-out" (3.5 mm), "bluetooth", "usb".
    pub routes: Vec<String>,
    /// Device names it is known by over Bluetooth / USB (for automatic matching).
    #[serde(default)]
    pub bluetooth_names: Vec<String>,
    #[serde(default)]
    pub usb_names: Vec<String>,
}

impl Headphone {
    /// File name of its EQ for a connection and rate family (`48k`/`44k`), relative to the DTS tuning directory.
    pub fn eq_file(&self, route: &str, rate_family: &str) -> String {
        format!("headphones/profile_{}_{route}_{rate_family}.bin", self.id)
    }

    /// The connection to use for an output: its own if there is an EQ for it, else the wired one, else any.
    pub fn route_for<'a>(&'a self, wanted: &'a str) -> Option<&'a str> {
        let has = |r: &str| self.routes.iter().any(|x| x == r);
        [wanted, "line-out"].into_iter().find(|r| has(r)).or(self.routes.first().map(String::as_str))
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Catalog {
    pub headphones: Vec<Headphone>,
}

impl Catalog {
    pub fn path(dts_dir: &Path) -> std::path::PathBuf {
        dts_dir.join("headphones/catalog.json")
    }

    pub fn load(dts_dir: &Path) -> Catalog {
        std::fs::read(Self::path(dts_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn get(&self, id: &str) -> Option<&Headphone> {
        self.headphones.iter().find(|h| h.id == id)
    }

    /// A model whose Bluetooth or USB device name matches `device_name` exactly (ignoring case).
    pub fn match_device(&self, device_name: &str) -> Option<&Headphone> {
        let n = device_name.trim();
        self.headphones.iter().find(|h| h.bluetooth_names.iter().chain(&h.usb_names).any(|x| x.eq_ignore_ascii_case(n)))
    }
}

/// 16-character strings in a DLL (UTF-16 or ASCII): candidate AES-128 keys.
pub(crate) fn key_candidates(dll: &[u8]) -> Vec<[u8; 16]> {
    let printable = |c: u8| (0x21..=0x7e).contains(&c);
    let mut out = Vec::new();
    // UTF-16LE: 16 printable characters followed by a NUL character.
    let mut i = 0;
    while i + 34 <= dll.len() {
        if (0..16).all(|k| printable(dll[i + 2 * k]) && dll[i + 2 * k + 1] == 0) && dll[i + 32] == 0 && dll[i + 33] == 0
        {
            out.push(std::array::from_fn(|k| dll[i + 2 * k]));
            i += 32;
        } else {
            i += 1;
        }
    }
    // ASCII: 16 printable bytes followed by NUL, not preceded by another printable byte.
    for i in 1..dll.len().saturating_sub(17) {
        if !printable(dll[i - 1]) && (0..16).all(|k| printable(dll[i + k])) && dll[i + 16] == 0 {
            out.push(dll[i..i + 16].try_into().unwrap());
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// AES-128-ECB with PKCS#7 padding; `None` unless the result is a complete EQ.
fn decrypt(key: &[u8; 16], data: &[u8]) -> Option<Vec<u8>> {
    aes_ecb_pkcs7(key, data).filter(|p| p.len() == EQ_SIZE)
}

/// AES-128-ECB decryption with PKCS#7 padding (how DTS's app stores its data); `None` if the padding is wrong.
pub(crate) fn aes_ecb_pkcs7(key: &[u8; 16], data: &[u8]) -> Option<Vec<u8>> {
    if data.len() % 16 != 0 || data.is_empty() {
        return None;
    }
    let c = Aes128::new(key.into());
    let mut p = data.to_vec();
    for block in p.chunks_exact_mut(16) {
        c.decrypt_block(block.try_into().unwrap());
    }
    let pad = *p.last()? as usize;
    if !(1..=16).contains(&pad) || !p[p.len() - pad..].iter().all(|&b| b as usize == pad) {
        return None;
    }
    p.truncate(p.len() - pad);
    Some(p)
}

/// Open a database found in the sources (sqlite needs a file).
fn open_db(e: &Entry, tmp: &Path) -> Result<rusqlite::Connection, Error> {
    let bytes = e.read().map_err(io(Path::new(&e.describe())))?;
    let p = tmp.join(format!("{}-{}", bytes.len(), e.name()));
    std::fs::write(&p, &bytes).map_err(io(&p))?;
    rusqlite::Connection::open_with_flags(&p, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|err| Error::Manifest(format!("{}: {err}", e.describe())))
}

struct Asset {
    uri: String,
    id: String,
    route: String,
    data: Vec<u8>,
}

fn assets(db: &rusqlite::Connection) -> rusqlite::Result<Vec<Asset>> {
    let mut q = db.prepare(
        "SELECT a.Uri, a.Uuid, lower(r.Value), a.Data FROM Asset a JOIN AudioRouteEnum r ON r.Id = a.AudioRouteEnum",
    )?;
    let rows = q.query_map([], |r| Ok(Asset { uri: r.get(0)?, id: r.get(1)?, route: r.get(2)?, data: r.get(3)? }))?;
    rows.collect()
}

#[derive(Default)]
struct Names {
    brand: String,
    name: String,
    style: String,
    bluetooth: Vec<String>,
    usb: Vec<String>,
}

fn names(db: &rusqlite::Connection, out: &mut BTreeMap<String, Names>) -> rusqlite::Result<()> {
    let mut q = db.prepare(
        "SELECT p.Uuid, coalesce(b.Name, ''), p.Name, coalesce(p.SubType, '') FROM Product p LEFT JOIN Brand b ON b.Id = p.BrandId",
    )?;
    for r in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))? {
        let (id, brand, name, style) = r?;
        let n = out.entry(id).or_default();
        if n.name.is_empty() {
            (n.brand, n.name, n.style) = (brand, name, style);
        }
    }
    for (table, bt) in [("BluetoothLookup", true), ("USBLookup", false)] {
        let Ok(mut q) = db.prepare(&format!("SELECT Uuid, Name FROM {table}")) else { continue };
        for r in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (id, name) = r?;
            let n = out.entry(id).or_default();
            let list = if bt { &mut n.bluetooth } else { &mut n.usb };
            if !list.contains(&name) {
                list.push(name);
            }
        }
    }
    Ok(())
}

/// Import headphone models from the sources into `dts_dir/headphones`. Returns how many models were imported
/// (0 when the sources hold no headphone data, which is not an error).
pub fn import(entries: &[Entry], dts_dir: &Path) -> Result<usize, Error> {
    let lower = |e: &Entry| e.name().to_ascii_lowercase();
    let tuning: Vec<&Entry> = entries
        .iter()
        .filter(|e| {
            let n = lower(e);
            n.ends_with(".db") && (n.contains("hptuning") || n.starts_with("offlinedatabase_tuning"))
        })
        .collect();
    if tuning.is_empty() {
        return Ok(0);
    }
    let ui: Vec<&Entry> =
        entries.iter().filter(|e| lower(e).starts_with("offlinedatabase_ui") && lower(e).ends_with(".db")).collect();
    let dlls: Vec<&Entry> =
        entries.iter().filter(|e| KEY_DLLS.iter().any(|d| d.eq_ignore_ascii_case(e.name()))).collect();

    let tmp = std::env::temp_dir().join(format!("acerultrax-import-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(io(&tmp))?;
    let result = (|| {
        let mut all_names = BTreeMap::new();
        for e in &ui {
            names(&open_db(e, &tmp)?, &mut all_names)
                .map_err(|err| Error::Manifest(format!("{}: {err}", e.describe())))?;
        }
        // Acer's own database first: where both have a model, its EQ is the one Acer's app would use.
        let mut tuning = tuning.clone();
        tuning.sort_by_key(|e| !lower(e).contains("hptuning"));
        let mut eqs: BTreeMap<String, Asset> = BTreeMap::new();
        for e in &tuning {
            for a in assets(&open_db(e, &tmp)?).map_err(|err| Error::Manifest(format!("{}: {err}", e.describe())))? {
                eqs.entry(a.uri.clone()).or_insert(a);
            }
        }
        let Some(sample) = eqs.values().next() else { return Ok(0) };
        let key = dlls
            .iter()
            .filter_map(|e| e.read().ok())
            .flat_map(|b| key_candidates(&b))
            .find(|k| decrypt(k, &sample.data).is_some())
            .ok_or_else(|| {
                Error::Manifest(
                    "headphone EQs found, but not the DTS app (\"XPERI DTS Utility\") needed to read them".into(),
                )
            })?;

        let out = dts_dir.join("headphones");
        let staging = dts_dir.join("headphones.new");
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).map_err(io(&staging))?;
        let mut models: BTreeMap<String, Headphone> = BTreeMap::new();
        for a in eqs.values() {
            let Some(n) = all_names.get(&a.id).filter(|n| !n.name.is_empty()) else { continue };
            let Some(plain) = decrypt(&key, &a.data) else { continue };
            let file = staging.join(format!("profile_{}", a.uri.trim_start_matches("profile_")));
            std::fs::write(&file, plain).map_err(io(&file))?;
            let h = models.entry(a.id.clone()).or_insert_with(|| Headphone {
                id: a.id.clone(),
                brand: n.brand.clone(),
                name: n.name.clone(),
                style: n.style.clone(),
                routes: Vec::new(),
                bluetooth_names: n.bluetooth.clone(),
                usb_names: n.usb.clone(),
            });
            if !h.routes.contains(&a.route) {
                h.routes.push(a.route.clone());
            }
        }
        // Keep only models with both rates for each connection they list.
        let mut list: Vec<Headphone> = models
            .into_values()
            .filter_map(|mut h| {
                h.routes.retain(|r| {
                    ["44k", "48k"].iter().all(|f| staging.join(format!("profile_{}_{r}_{f}.bin", h.id)).exists())
                });
                h.routes.sort();
                (!h.routes.is_empty()).then_some(h)
            })
            .collect();
        list.sort_by(|a, b| {
            (a.brand.to_lowercase(), a.name.to_lowercase()).cmp(&(b.brand.to_lowercase(), b.name.to_lowercase()))
        });
        let n = list.len();
        let catalog = serde_json::to_vec_pretty(&Catalog { headphones: list }).unwrap();
        std::fs::write(staging.join("catalog.json"), catalog).map_err(io(&staging))?;
        let _ = std::fs::remove_dir_all(&out);
        std::fs::rename(&staging, &out).map_err(io(&out))?;
        Ok(n)
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockCipherEncrypt;

    const KEY: &[u8; 16] = b"TestKey-1234abcd";

    fn encrypt(plain: &[u8]) -> Vec<u8> {
        let c = Aes128::new(KEY.into());
        let pad = 16 - plain.len() % 16;
        let mut v = plain.to_vec();
        v.extend(std::iter::repeat_n(pad as u8, pad));
        for b in v.chunks_exact_mut(16) {
            c.encrypt_block(b.try_into().unwrap());
        }
        v
    }

    #[test]
    fn finds_key_decrypts_and_names() {
        let dir = std::env::temp_dir().join(format!("acerultrax-hp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let tuning = rusqlite::Connection::open(dir.join("src/dts_apo4_oem_offline_hptuning_acer.db")).unwrap();
        tuning
            .execute_batch(
                "CREATE TABLE AudioRouteEnum (Id INTEGER, Value TEXT); INSERT INTO AudioRouteEnum VALUES (2,'Line-Out'),(3,'Bluetooth');
                 CREATE TABLE Asset (Id INTEGER, Uri TEXT, Data BLOB, Filesize INTEGER, AudioRouteEnum INTEGER, SampleRateEnum INTEGER, Uuid TEXT);",
            )
            .unwrap();
        for (uri, route) in [
            ("profile_aaa_line-out_44k.bin", 2),
            ("profile_aaa_line-out_48k.bin", 2),
            ("profile_aaa_bluetooth_48k.bin", 3),
        ] {
            tuning
                .execute(
                    "INSERT INTO Asset VALUES (NULL, ?1, ?2, 11552, ?3, 1, 'aaa')",
                    rusqlite::params![uri, encrypt(&[7u8; EQ_SIZE]), route],
                )
                .unwrap();
        }
        let ui = rusqlite::Connection::open(dir.join("src/offlinedatabase_ui_x.db")).unwrap();
        ui.execute_batch(
            "CREATE TABLE Brand (Id INTEGER, Uuid TEXT, Name TEXT); INSERT INTO Brand VALUES (1,'b','HyperX');
             CREATE TABLE Product (Id INTEGER, Uuid TEXT, BrandId INTEGER, Name TEXT, SubType TEXT);
             INSERT INTO Product VALUES (1,'aaa',1,'Cloud Core','Over-Ear');
             CREATE TABLE BluetoothLookup (Id INTEGER, Name TEXT, Uuid TEXT); INSERT INTO BluetoothLookup VALUES (1,'HyperX Cloud','aaa');",
        )
        .unwrap();
        drop((tuning, ui));
        let mut dll = b"MZ....".to_vec();
        for c in KEY {
            dll.extend([*c, 0]);
        }
        dll.extend([0, 0, 1, 2]);
        std::fs::write(dir.join("src/DTSSoundUnbound2.dll"), dll).unwrap();

        let entries = crate::sources::collect(&[dir.join("src")]);
        let dts = dir.join("dts");
        assert_eq!(import(&entries, &dts).unwrap(), 1);
        let cat = Catalog::load(&dts);
        let h = cat.get("aaa").unwrap();
        assert_eq!((h.brand.as_str(), h.name.as_str()), ("HyperX", "Cloud Core"));
        assert_eq!(h.routes, ["line-out"], "a connection without both rates is left out");
        assert_eq!(std::fs::read(dts.join(h.eq_file("line-out", "48k"))).unwrap(), vec![7u8; EQ_SIZE]);
        assert_eq!(cat.match_device("hyperx cloud").map(|h| h.id.as_str()), Some("aaa"));
        assert_eq!(h.route_for("bluetooth"), Some("line-out"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wrong_key_is_rejected() {
        assert!(decrypt(b"0123456789abcdef", &encrypt(&[1u8; EQ_SIZE])).is_none());
        assert_eq!(decrypt(KEY, &encrypt(&[1u8; EQ_SIZE])).map(|p| p.len()), Some(EQ_SIZE));
    }
}
