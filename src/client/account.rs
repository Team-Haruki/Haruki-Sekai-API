use serde::{Deserialize, Deserializer, Serialize};

use crate::error::AppError;

fn null_to_empty_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::<String>::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

pub fn null_or_number_to_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNumber {
        String(String),
        Number(i64),
        Null,
    }
    match StringOrNumber::deserialize(deserializer)? {
        StringOrNumber::String(s) => Ok(s),
        StringOrNumber::Number(n) => Ok(n.to_string()),
        StringOrNumber::Null => Ok(String::new()),
    }
}

#[allow(dead_code)]
pub trait SekaiAccount: Send + Sync {
    fn user_id(&self) -> &str;
    fn set_user_id(&mut self, user_id: String);
    fn device_id(&self) -> &str;
    fn token(&self) -> &str;
    fn dump(&self) -> Result<Vec<u8>, AppError>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SekaiAccountCP {
    #[serde(
        rename = "userId",
        default,
        deserialize_with = "null_or_number_to_string"
    )]
    pub user_id: String,
    #[serde(
        rename = "deviceId",
        default,
        deserialize_with = "null_to_empty_string"
    )]
    pub device_id: String,
    #[serde(default, deserialize_with = "null_to_empty_string")]
    pub credential: String,
}

impl SekaiAccount for SekaiAccountCP {
    fn user_id(&self) -> &str {
        &self.user_id
    }

    fn set_user_id(&mut self, user_id: String) {
        self.user_id = user_id;
    }

    fn device_id(&self) -> &str {
        &self.device_id
    }

    fn token(&self) -> &str {
        &self.credential
    }

    fn dump(&self) -> Result<Vec<u8>, AppError> {
        #[derive(Serialize)]
        struct LoginPayload<'a> {
            #[serde(rename = "deviceId", skip_serializing_if = "Option::is_none")]
            device_id: Option<&'a str>,
            credential: &'a str,
            #[serde(rename = "authTriggerType")]
            auth_trigger_type: &'static str,
        }

        let payload = LoginPayload {
            device_id: if self.device_id.is_empty() {
                None
            } else {
                Some(&self.device_id)
            },
            credential: &self.credential,
            auth_trigger_type: "normal",
        };

        rmp_serde::to_vec_named(&payload).map_err(|e| AppError::ParseError(e.to_string()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SekaiAccountNuverse {
    #[serde(
        alias = "userId",
        alias = "userID",
        default,
        deserialize_with = "null_or_number_to_string"
    )]
    pub user_id: String,
    #[serde(
        rename = "deviceId",
        default,
        deserialize_with = "null_to_empty_string"
    )]
    pub device_id: String,
    #[serde(
        rename = "accessToken",
        default,
        deserialize_with = "null_to_empty_string"
    )]
    pub access_token: String,
}

impl SekaiAccount for SekaiAccountNuverse {
    fn user_id(&self) -> &str {
        &self.user_id
    }

    fn set_user_id(&mut self, user_id: String) {
        self.user_id = user_id;
    }

    fn device_id(&self) -> &str {
        &self.device_id
    }

    fn token(&self) -> &str {
        &self.access_token
    }

    fn dump(&self) -> Result<Vec<u8>, AppError> {
        #[derive(Serialize)]
        struct LoginPayload<'a> {
            #[serde(rename = "deviceId", skip_serializing_if = "Option::is_none")]
            device_id: Option<&'a str>,
            #[serde(rename = "accessToken")]
            access_token: &'a str,
            #[serde(rename = "userID")]
            user_id: i64,
        }

        let user_id_num: i64 = self
            .user_id
            .parse()
            .map_err(|_| AppError::ParseError(format!("Invalid user_id: {}", self.user_id)))?;

        let payload = LoginPayload {
            device_id: if self.device_id.is_empty() {
                None
            } else {
                Some(&self.device_id)
            },
            access_token: &self.access_token,
            user_id: user_id_num,
        };

        rmp_serde::to_vec_named(&payload).map_err(|e| AppError::ParseError(e.to_string()))
    }
}

#[derive(Debug, Clone)]
pub enum AccountType {
    CP(SekaiAccountCP),
    Nuverse(SekaiAccountNuverse),
}

impl SekaiAccount for AccountType {
    fn user_id(&self) -> &str {
        match self {
            AccountType::CP(a) => a.user_id(),
            AccountType::Nuverse(a) => a.user_id(),
        }
    }

    fn set_user_id(&mut self, user_id: String) {
        match self {
            AccountType::CP(a) => a.set_user_id(user_id),
            AccountType::Nuverse(a) => a.set_user_id(user_id),
        }
    }

    fn device_id(&self) -> &str {
        match self {
            AccountType::CP(a) => a.device_id(),
            AccountType::Nuverse(a) => a.device_id(),
        }
    }

    fn token(&self) -> &str {
        match self {
            AccountType::CP(a) => a.token(),
            AccountType::Nuverse(a) => a.token(),
        }
    }

    fn dump(&self) -> Result<Vec<u8>, AppError> {
        match self {
            AccountType::CP(a) => a.dump(),
            AccountType::Nuverse(a) => a.dump(),
        }
    }
}
