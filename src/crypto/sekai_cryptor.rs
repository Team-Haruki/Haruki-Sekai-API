use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use aes::Aes128;
use indexmap::IndexMap;
use rmp_serde as rmps;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

type Aes128CbcEnc = cbc::Encryptor<Aes128>;
type Aes128CbcDec = cbc::Decryptor<Aes128>;

#[derive(Clone)]
pub struct SekaiCryptor {
    key: [u8; 16],
    iv: [u8; 16],
}

impl SekaiCryptor {
    pub fn from_hex(key_hex: &str, iv_hex: &str) -> Result<Self, AppError> {
        let key = hex::decode(key_hex)
            .map_err(|e| AppError::CryptoError(format!("Invalid AES key hex: {}", e)))?;
        let iv = hex::decode(iv_hex)
            .map_err(|e| AppError::CryptoError(format!("Invalid AES IV hex: {}", e)))?;
        if key.len() != 16 {
            return Err(AppError::CryptoError(format!(
                "Invalid key length: got {}, want 16",
                key.len()
            )));
        }
        if iv.len() != 16 {
            return Err(AppError::CryptoError(format!(
                "Invalid IV length: got {}, want 16",
                iv.len()
            )));
        }
        let mut key_arr = [0u8; 16];
        let mut iv_arr = [0u8; 16];
        key_arr.copy_from_slice(&key);
        iv_arr.copy_from_slice(&iv);

        Ok(Self {
            key: key_arr,
            iv: iv_arr,
        })
    }

    pub fn pack<T: Serialize>(&self, data: &T) -> Result<Vec<u8>, AppError> {
        let msgpack_data = rmps::to_vec(data)?;
        let padded = pkcs7_pad(&msgpack_data, 16);
        let encryptor = Aes128CbcEnc::new(&self.key.into(), &self.iv.into());
        let encrypted =
            encryptor.encrypt_padded_vec_mut::<aes::cipher::block_padding::NoPadding>(&padded);
        Ok(encrypted)
    }

    pub fn pack_bytes(&self, data: &[u8]) -> Result<Vec<u8>, AppError> {
        if data.is_empty() {
            return Err(AppError::CryptoError("Content cannot be empty".to_string()));
        }
        let padded = pkcs7_pad(data, 16);
        let encryptor = Aes128CbcEnc::new(&self.key.into(), &self.iv.into());
        let encrypted =
            encryptor.encrypt_padded_vec_mut::<aes::cipher::block_padding::NoPadding>(&padded);
        Ok(encrypted)
    }

    pub fn unpack<T: for<'de> Deserialize<'de>>(&self, data: &[u8]) -> Result<T, AppError> {
        if data.is_empty() {
            return Err(AppError::CryptoError("Content cannot be empty".to_string()));
        }
        if data.len() % 16 != 0 {
            return Err(AppError::CryptoError(
                "Content length is not a multiple of AES block size".to_string(),
            ));
        }
        let decryptor = Aes128CbcDec::new(&self.key.into(), &self.iv.into());
        let mut buf = data.to_vec();
        let decrypted = decryptor
            .decrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut buf)
            .map_err(|e| AppError::CryptoError(format!("Decryption failed: {}", e)))?;
        let unpadded = pkcs7_unpad(decrypted)?;
        let result: T = rmps::from_slice(unpadded)?;
        Ok(result)
    }

    pub fn unpack_ordered(
        &self,
        data: &[u8],
    ) -> Result<IndexMap<String, serde_json::Value>, AppError> {
        if data.is_empty() {
            return Err(AppError::CryptoError("Content cannot be empty".to_string()));
        }
        if data.len() % 16 != 0 {
            return Err(AppError::CryptoError(
                "Content length is not a multiple of AES block size".to_string(),
            ));
        }
        let decryptor = Aes128CbcDec::new(&self.key.into(), &self.iv.into());
        let mut buf = data.to_vec();
        let decrypted = decryptor
            .decrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut buf)
            .map_err(|e| AppError::CryptoError(format!("Decryption failed: {}", e)))?;
        let unpadded = pkcs7_unpad(decrypted)?;
        let result = msgpack_to_ordered_value(unpadded)?;
        match result {
            serde_json::Value::Object(map) => {
                let ordered: IndexMap<String, serde_json::Value> = map.into_iter().collect();
                Ok(ordered)
            }
            _ => Err(AppError::CryptoError(
                "Expected object at top level".to_string(),
            )),
        }
    }

    pub fn unpack_value(&self, data: &[u8]) -> Result<serde_json::Value, AppError> {
        if data.is_empty() {
            return Err(AppError::CryptoError("Content cannot be empty".to_string()));
        }
        if data.len() % 16 != 0 {
            return Err(AppError::CryptoError(
                "Content length is not a multiple of AES block size".to_string(),
            ));
        }
        let decryptor = Aes128CbcDec::new(&self.key.into(), &self.iv.into());
        let mut buf = data.to_vec();
        let decrypted = decryptor
            .decrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut buf)
            .map_err(|e| AppError::CryptoError(format!("Decryption failed: {}", e)))?;

        let unpadded = pkcs7_unpad(decrypted)?;
        msgpack_to_ordered_value(unpadded)
    }
}

fn pkcs7_pad(data: &[u8], block_size: usize) -> Vec<u8> {
    let padding_len = block_size - (data.len() % block_size);
    let mut padded = data.to_vec();
    padded.extend(std::iter::repeat(padding_len as u8).take(padding_len));
    padded
}

fn pkcs7_unpad(data: &[u8]) -> Result<&[u8], AppError> {
    if data.is_empty() {
        return Err(AppError::CryptoError(
            "Empty data for unpadding".to_string(),
        ));
    }
    let padding_len = data[data.len() - 1] as usize;
    if padding_len == 0 || padding_len > 16 || padding_len > data.len() {
        return Err(AppError::CryptoError("Invalid PKCS7 padding".to_string()));
    }
    for &byte in &data[data.len() - padding_len..] {
        if byte != padding_len as u8 {
            return Err(AppError::CryptoError(
                "Invalid PKCS7 padding bytes".to_string(),
            ));
        }
    }
    Ok(&data[..data.len() - padding_len])
}

fn msgpack_to_ordered_value(data: &[u8]) -> Result<serde_json::Value, AppError> {
    use std::io::Cursor;
    let mut cursor = Cursor::new(data);
    let value = rmpv::decode::read_value(&mut cursor)
        .map_err(|e| AppError::CryptoError(format!("MsgPack decode error: {}", e)))?;
    rmpv_to_json(value)
}

fn rmpv_to_json(value: rmpv::Value) -> Result<serde_json::Value, AppError> {
    use rmpv::Value;
    use serde_json::Map;
    use serde_json::Value as JsonValue;
    match value {
        Value::Nil => Ok(JsonValue::Null),
        Value::Boolean(b) => Ok(JsonValue::Bool(b)),
        Value::Integer(i) => {
            if let Some(n) = i.as_i64() {
                Ok(JsonValue::Number(n.into()))
            } else if let Some(n) = i.as_u64() {
                Ok(JsonValue::Number(n.into()))
            } else {
                Ok(JsonValue::Null)
            }
        }
        Value::F32(f) => serde_json::Number::from_f64(f as f64)
            .map(JsonValue::Number)
            .ok_or_else(|| AppError::CryptoError("Invalid float".to_string())),
        Value::F64(f) => serde_json::Number::from_f64(f)
            .map(JsonValue::Number)
            .ok_or_else(|| AppError::CryptoError("Invalid float".to_string())),
        Value::String(s) => {
            let s = s.into_str().unwrap_or_default();
            Ok(JsonValue::String(s.to_string()))
        }
        Value::Binary(b) => Ok(JsonValue::String(base64_encode(&b))),
        Value::Array(arr) => {
            let json_arr: Result<Vec<JsonValue>, _> = arr.into_iter().map(rmpv_to_json).collect();
            Ok(JsonValue::Array(json_arr?))
        }
        Value::Map(map) => {
            let mut json_map = Map::new();
            for (k, v) in map {
                let key = match k {
                    Value::String(s) => s.into_str().unwrap_or_default().to_string(),
                    Value::Integer(i) => i.to_string(),
                    _ => continue,
                };
                json_map.insert(key, rmpv_to_json(v)?);
            }
            Ok(JsonValue::Object(json_map))
        }
        Value::Ext(_, data) => Ok(JsonValue::String(base64_encode(&data))),
    }
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_pkcs7_padding() {
        let data = b"hello";
        let padded = pkcs7_pad(data, 16);
        assert_eq!(padded.len(), 16);
        assert_eq!(padded[5..], [11u8; 11]);
        let unpadded = pkcs7_unpad(&padded).unwrap();
        assert_eq!(unpadded, data);
    }
    #[test]
    fn test_cryptor_roundtrip() {
        let key_hex = "00112233445566778899aabbccddeeff";
        let iv_hex = "ffeeddccbbaa99887766554433221100";
        let cryptor = SekaiCryptor::from_hex(key_hex, iv_hex).unwrap();
        let original = serde_json::json!({
            "test": "value",
            "number": 42
        });
        let packed = cryptor.pack(&original).unwrap();
        let unpacked: serde_json::Value = cryptor.unpack(&packed).unwrap();
        assert_eq!(original, unpacked);
    }
}
