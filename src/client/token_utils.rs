use base64::Engine;
use serde::Deserialize;

use crate::error::AppError;

#[derive(Deserialize)]
struct JwtPayload {
    #[serde(alias = "userId", alias = "user_id")]
    user_id: Option<String>,
}

#[derive(Deserialize)]
struct NuverseTokenPayload {
    sdk_open_id: Option<String>,
}

pub fn extract_user_id_from_jwt(token: &str) -> Result<String, AppError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(AppError::ParseError("Invalid JWT format".to_string()));
    }
    let payload_b64 = parts[1];
    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(payload_b64))
        .map_err(|e| AppError::ParseError(format!("JWT base64 decode error: {}", e)))?;
    let payload: JwtPayload = sonic_rs::from_slice(&payload_bytes)
        .map_err(|e| AppError::ParseError(format!("JWT payload parse error: {}", e)))?;
    payload
        .user_id
        .ok_or_else(|| AppError::ParseError("userId not found in JWT".to_string()))
}

pub fn extract_user_id_from_nuverse_token(token: &str) -> Result<String, AppError> {
    let decoded_jwt = base64::engine::general_purpose::STANDARD
        .decode(token)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(token))
        .map_err(|e| AppError::ParseError(format!("Nuverse token base64 decode error: {}", e)))?;
    let jwt_str = String::from_utf8(decoded_jwt)
        .map_err(|e| AppError::ParseError(format!("Nuverse token UTF-8 error: {}", e)))?;
    let parts: Vec<&str> = jwt_str.split('.').collect();
    if parts.len() != 3 {
        return Err(AppError::ParseError(
            "Invalid Nuverse JWT format".to_string(),
        ));
    }
    let payload_b64 = parts[1];
    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(payload_b64))
        .map_err(|e| AppError::ParseError(format!("Nuverse JWT payload decode error: {}", e)))?;
    let payload: NuverseTokenPayload = sonic_rs::from_slice(&payload_bytes)
        .map_err(|e| AppError::ParseError(format!("Nuverse token parse error: {}", e)))?;
    payload
        .sdk_open_id
        .ok_or_else(|| AppError::ParseError("sdk_open_id not found in token".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_user_id_from_jwt() {
        let payload = r#"{"userId":"12345"}"#;
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload);
        let jwt = format!("header.{}.signature", encoded);
        let result = extract_user_id_from_jwt(&jwt);
        assert_eq!(result.unwrap(), "12345");
    }

    #[test]
    fn test_extract_user_id_from_nuverse_token() {
        let jwt_payload = r#"{"sdk_open_id":"67890","device_id":123}"#;
        let jwt_payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(jwt_payload);
        let jwt = format!(
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.{}.signature",
            jwt_payload_b64
        );
        let encoded = base64::engine::general_purpose::STANDARD.encode(&jwt);
        let result = extract_user_id_from_nuverse_token(&encoded);
        assert_eq!(result.unwrap(), "67890");
    }
}
