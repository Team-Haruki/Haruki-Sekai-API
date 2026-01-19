use std::sync::Arc;

use parking_lot::Mutex;

use super::account::{AccountType, SekaiAccount};

#[derive(Clone)]
pub struct AccountSession {
    pub account: Arc<Mutex<AccountType>>,
    pub session_token: Arc<Mutex<Option<String>>>,
    api_lock: Arc<tokio::sync::Mutex<()>>,
}

impl AccountSession {
    pub fn new(account: AccountType) -> Self {
        Self {
            account: Arc::new(Mutex::new(account)),
            session_token: Arc::new(Mutex::new(None)),
            api_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub fn user_id(&self) -> String {
        self.account.lock().user_id().to_string()
    }

    pub fn set_user_id(&self, user_id: String) {
        self.account.lock().set_user_id(user_id);
    }

    pub async fn lock_api(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.api_lock.lock().await
    }

    pub fn get_session_token(&self) -> Option<String> {
        self.session_token.lock().clone()
    }

    pub fn set_session_token(&self, token: Option<String>) {
        *self.session_token.lock() = token;
    }

    pub fn dump_account(&self) -> Result<Vec<u8>, crate::error::AppError> {
        self.account.lock().dump()
    }
}
