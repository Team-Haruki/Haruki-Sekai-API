use aes::Aes128;
use cbc::cipher::{block_padding::NoPadding, BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
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
        let encrypted = encryptor.encrypt_padded_vec::<NoPadding>(&padded);
        Ok(encrypted)
    }

    pub fn pack_bytes(&self, data: &[u8]) -> Result<Vec<u8>, AppError> {
        if data.is_empty() {
            return Err(AppError::CryptoError("Content cannot be empty".to_string()));
        }
        let padded = pkcs7_pad(data, 16);
        let encryptor = Aes128CbcEnc::new(&self.key.into(), &self.iv.into());
        let encrypted = encryptor.encrypt_padded_vec::<NoPadding>(&padded);
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
            .decrypt_padded::<NoPadding>(&mut buf)
            .map_err(|e| AppError::CryptoError(format!("Decryption failed: {}", e)))?;
        let unpadded = pkcs7_unpad(decrypted)?;
        let result: T = rmps::from_slice(unpadded)?;
        Ok(result)
    }

    pub fn unpack_ordered(
        &self,
        data: &[u8],
    ) -> Result<IndexMap<String, serde_json::Value>, AppError> {
        let unpadded = self.decrypt_msgpack(data)?;
        let result = msgpack_to_ordered_value(&unpadded)?;
        match result {
            serde_json::Value::Object(map) => {
                let ordered: IndexMap<String, serde_json::Value> = map.into_iter().collect();
                Ok(ordered)
            }
            _ => Err(AppError::UpstreamData(
                "Expected object at top level".to_string(),
            )),
        }
    }

    pub fn decrypt_msgpack(&self, data: &[u8]) -> Result<Vec<u8>, AppError> {
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
            .decrypt_padded::<NoPadding>(&mut buf)
            .map_err(|e| AppError::CryptoError(format!("Decryption failed: {}", e)))?;
        // Drop the padding in place instead of allocating a second full buffer.
        let plain_len = decrypted.len() - pkcs7_padding_len(decrypted)?;
        buf.truncate(plain_len);
        Ok(buf)
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
            .decrypt_padded::<NoPadding>(&mut buf)
            .map_err(|e| AppError::CryptoError(format!("Decryption failed: {}", e)))?;

        let unpadded = pkcs7_unpad(decrypted)?;
        msgpack_to_ordered_value(unpadded)
    }
}

fn pkcs7_pad(data: &[u8], block_size: usize) -> Vec<u8> {
    let padding_len = block_size - (data.len() % block_size);
    let mut padded = data.to_vec();
    padded.extend(std::iter::repeat_n(padding_len as u8, padding_len));
    padded
}

fn pkcs7_padding_len(data: &[u8]) -> Result<usize, AppError> {
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
    Ok(padding_len)
}

fn pkcs7_unpad(data: &[u8]) -> Result<&[u8], AppError> {
    let padding_len = pkcs7_padding_len(data)?;
    Ok(&data[..data.len() - padding_len])
}

fn msgpack_to_ordered_value(data: &[u8]) -> Result<serde_json::Value, AppError> {
    decode_msgpack_value(data)
}

/// Decode a MessagePack byte buffer into an ordered `serde_json::Value`.
///
/// Uses rmpv's borrowing reader (`read_value_ref`) so the intermediate rmpv
/// tree borrows strings/binaries from `data` instead of allocating owned copies
/// for them, halving the decode-time allocations versus `read_value`. Semantics
/// are identical to the owned path: integer map keys are stringified and
/// binary/ext payloads are base64-encoded. Shared by the crypto and Nuverse
/// master restore paths.
pub fn decode_msgpack_value(data: &[u8]) -> Result<serde_json::Value, AppError> {
    let mut reader: &[u8] = data;
    let value = rmpv::decode::read_value_ref(&mut reader)
        .map_err(|e| AppError::UpstreamData(format!("MsgPack decode error: {}", e)))?;
    rmpv_ref_to_json(value)
}

fn rmpv_ref_to_json(value: rmpv::ValueRef) -> Result<serde_json::Value, AppError> {
    use rmpv::ValueRef;
    use serde_json::Map;
    use serde_json::Value as JsonValue;
    match value {
        ValueRef::Nil => Ok(JsonValue::Null),
        ValueRef::Boolean(b) => Ok(JsonValue::Bool(b)),
        ValueRef::Integer(i) => {
            if let Some(n) = i.as_i64() {
                Ok(JsonValue::Number(n.into()))
            } else if let Some(n) = i.as_u64() {
                Ok(JsonValue::Number(n.into()))
            } else {
                Ok(JsonValue::Null)
            }
        }
        ValueRef::F32(f) => serde_json::Number::from_f64(f as f64)
            .map(JsonValue::Number)
            .ok_or_else(|| AppError::UpstreamData("Invalid float".to_string())),
        ValueRef::F64(f) => serde_json::Number::from_f64(f)
            .map(JsonValue::Number)
            .ok_or_else(|| AppError::UpstreamData("Invalid float".to_string())),
        ValueRef::String(s) => Ok(JsonValue::String(
            s.as_str().unwrap_or_default().to_string(),
        )),
        ValueRef::Binary(b) => Ok(JsonValue::String(base64_encode(b))),
        ValueRef::Array(arr) => {
            let json_arr: Result<Vec<JsonValue>, _> =
                arr.into_iter().map(rmpv_ref_to_json).collect();
            Ok(JsonValue::Array(json_arr?))
        }
        ValueRef::Map(map) => {
            let mut json_map = Map::new();
            for (k, v) in map {
                let key = match k {
                    ValueRef::String(s) => s.as_str().unwrap_or_default().to_string(),
                    ValueRef::Integer(i) => i.to_string(),
                    _ => continue,
                };
                json_map.insert(key, rmpv_ref_to_json(v)?);
            }
            Ok(JsonValue::Object(json_map))
        }
        ValueRef::Ext(_, data) => Ok(JsonValue::String(base64_encode(data))),
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

    #[test]
    fn decode_msgpack_value_stringifies_int_keys_and_base64s_binary() {
        use rmpv::Value as RV;
        // A map with integer keys, a nested array, a string key, and a binary value.
        let value = RV::Map(vec![
            (RV::from(0i64), RV::from("alpha")),
            (
                RV::from(1i64),
                RV::Array(vec![RV::from(10i64), RV::from(20i64)]),
            ),
            (RV::from("name"), RV::from("bob")),
            (RV::from("bin"), RV::Binary(vec![0xDE, 0xAD])),
        ]);
        let mut buf = Vec::new();
        rmpv::encode::write_value(&mut buf, &value).unwrap();

        let decoded = decode_msgpack_value(&buf).unwrap();
        // Integer keys are stringified (matches the rmpv path semantics).
        assert_eq!(decoded["0"], serde_json::json!("alpha"));
        assert_eq!(decoded["1"], serde_json::json!([10, 20]));
        assert_eq!(decoded["name"], serde_json::json!("bob"));
        // Binary is base64-encoded.
        assert_eq!(decoded["bin"], serde_json::json!("3q0="));
    }

    #[test]
    fn decrypt_msgpack_truncates_padding_and_roundtrips() {
        let cryptor = SekaiCryptor::from_hex(
            "00112233445566778899aabbccddeeff",
            "ffeeddccbbaa99887766554433221100",
        )
        .unwrap();
        let original = serde_json::json!({"a": 1, "b": [2, 3], "c": "hello"});
        let packed = cryptor.pack(&original).unwrap();
        // decrypt_msgpack must yield the exact unpadded msgpack plaintext.
        let msgpack = cryptor.decrypt_msgpack(&packed).unwrap();
        assert!(msgpack.len() < packed.len(), "padding must be stripped");
        assert_eq!(decode_msgpack_value(&msgpack).unwrap(), original);
    }
}
