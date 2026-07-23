use aes::Aes128;
use cipher::block_padding::Pkcs7;
use cipher::{BlockDecryptMut, KeyIvInit};
use pbkdf2::pbkdf2_hmac;
use sha1::Sha1;
use std::fs;
use std::path::Path;

type Aes128CbcDec = cbc::Decryptor<Aes128>;

const ES3_PASSWORD: &str = "emuMqG3bLYJ938ZDCfieWJ";
const PBKDF2_ITERS: u32 = 100;
const KEY_SIZE: usize = 16;
const IV_SIZE: usize = 16;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SaveData {
    #[serde(rename = "AccountSaveData")]
    pub account: RawEs3Field,
    #[serde(rename = "PlayerSaveData")]
    pub player: RawEs3Field,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RawEs3Field {
    #[serde(rename = "__type")]
    pub _type: Option<String>,
    pub value: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlayerData {
    #[serde(flatten)]
    pub other: serde_json::Value,
}

impl SaveData {
    pub fn parse_player(&self) -> Result<PlayerData, serde_json::Error> {
        serde_json::from_str(&self.player.value)
    }
}

pub fn es3_decrypt(raw: &[u8], password: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if raw.len() < IV_SIZE {
        return Err("File too small".into());
    }

    let iv = &raw[..IV_SIZE];
    let ct = &raw[IV_SIZE..];

    let mut key = [0u8; KEY_SIZE];
    pbkdf2_hmac::<Sha1>(password.as_bytes(), iv, PBKDF2_ITERS, &mut key);

    let decryptor = Aes128CbcDec::new(&key.into(), iv.into());
    let mut buf = ct.to_vec();
    buf = decryptor.decrypt_padded_vec_mut::<Pkcs7>(&buf).map_err(|e| format!("Unpad error: {:?}", e))?;

    Ok(buf)
}

pub fn load_save_file(path: &Path) -> Result<SaveData, Box<dyn std::error::Error>> {
    let raw = fs::read(path)?;
    let decrypted = es3_decrypt(&raw, ES3_PASSWORD)?;
    let json_str = String::from_utf8(decrypted)?;
    let save_data: SaveData = serde_json::from_str(&json_str)?;
    Ok(save_data)
}

pub fn get_default_save_path() -> std::path::PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    home.join("AppData")
        .join("LocalLow")
        .join("TesseractStudio")
        .join("TaskbarHero")
        .join("SaveFile_Live.es3")
}
