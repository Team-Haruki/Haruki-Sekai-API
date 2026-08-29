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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cp_account_deserializes_legacy_values_and_builds_login_payload() {
        let mut account: SekaiAccountCP =
            sonic_rs::from_str(r#"{"userId":12345,"deviceId":null,"credential":"secret"}"#)
                .unwrap();
        assert_eq!(account.user_id(), "12345");
        assert_eq!(account.device_id(), "");
        assert_eq!(account.token(), "secret");

        account.set_user_id("54321".to_string());
        let payload: serde_json::Value = rmp_serde::from_slice(&account.dump().unwrap()).unwrap();
        assert_eq!(payload["credential"], "secret");
        assert_eq!(payload["authTriggerType"], "normal");
        assert!(payload.get("deviceId").is_none());
    }

    #[test]
    fn cp_payload_includes_nonempty_device_id() {
        let account = SekaiAccountCP {
            user_id: "1".to_string(),
            device_id: "device".to_string(),
            credential: "credential".to_string(),
        };
        let payload: serde_json::Value = rmp_serde::from_slice(&account.dump().unwrap()).unwrap();
        assert_eq!(payload["deviceId"], "device");
    }

    #[test]
    fn nuverse_account_supports_aliases_and_numeric_user_ids() {
        for key in ["user_id", "userId", "userID"] {
            let json = format!(r#"{{"{key}":42,"deviceId":"phone","accessToken":"token"}}"#);
            let account: SekaiAccountNuverse = sonic_rs::from_str(&json).unwrap();
            assert_eq!(account.user_id(), "42");
            assert_eq!(account.device_id(), "phone");
            assert_eq!(account.token(), "token");

            let payload: serde_json::Value =
                rmp_serde::from_slice(&account.dump().unwrap()).unwrap();
            assert_eq!(payload["userID"], 42);
            assert_eq!(payload["deviceId"], "phone");
            assert_eq!(payload["accessToken"], "token");
        }
    }

    #[test]
    fn nuverse_dump_rejects_non_numeric_user_id_and_omits_empty_device() {
        let invalid = SekaiAccountNuverse {
            user_id: "not-a-number".to_string(),
            device_id: String::new(),
            access_token: "token".to_string(),
        };
        assert!(matches!(invalid.dump(), Err(AppError::ParseError(_))));

        let valid = SekaiAccountNuverse {
            user_id: "7".to_string(),
            device_id: String::new(),
            access_token: "token".to_string(),
        };
        let payload: serde_json::Value = rmp_serde::from_slice(&valid.dump().unwrap()).unwrap();
        assert!(payload.get("deviceId").is_none());
    }

    #[test]
    fn account_type_delegates_trait_operations() {
        let mut cp = AccountType::CP(SekaiAccountCP {
            user_id: "1".to_string(),
            device_id: "cp-device".to_string(),
            credential: "cp-token".to_string(),
        });
        assert_eq!(cp.user_id(), "1");
        assert_eq!(cp.device_id(), "cp-device");
        assert_eq!(cp.token(), "cp-token");
        cp.set_user_id("2".to_string());
        assert_eq!(cp.user_id(), "2");
        assert!(cp.dump().is_ok());

        let mut nuverse = AccountType::Nuverse(SekaiAccountNuverse {
            user_id: "3".to_string(),
            device_id: "nv-device".to_string(),
            access_token: "nv-token".to_string(),
        });
        assert_eq!(nuverse.user_id(), "3");
        assert_eq!(nuverse.device_id(), "nv-device");
        assert_eq!(nuverse.token(), "nv-token");
        nuverse.set_user_id("4".to_string());
        assert_eq!(nuverse.user_id(), "4");
        assert!(nuverse.dump().is_ok());
    }

    #[test]
    fn null_account_fields_deserialize_as_empty_strings() {
        let cp: SekaiAccountCP =
            sonic_rs::from_str(r#"{"userId":null,"deviceId":null,"credential":null}"#).unwrap();
        assert_eq!(cp.user_id, "");
        assert_eq!(cp.device_id, "");
        assert_eq!(cp.credential, "");

        let nuverse: SekaiAccountNuverse =
            sonic_rs::from_str(r#"{"user_id":null,"deviceId":null,"accessToken":null}"#).unwrap();
        assert_eq!(nuverse.user_id, "");
        assert_eq!(nuverse.device_id, "");
        assert_eq!(nuverse.access_token, "");
    }
}
