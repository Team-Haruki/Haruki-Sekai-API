use std::sync::Arc;

use parking_lot::Mutex;

use super::account::{AccountType, SekaiAccount};

#[derive(Clone)]
pub struct AccountSession {
    pub account: Arc<Mutex<AccountType>>,
    pub session_token: Arc<Mutex<Option<String>>>,
    api_lock: Arc<tokio::sync::Mutex<()>>,
    login_lock: Arc<tokio::sync::Mutex<()>>,
}

impl AccountSession {
    pub fn new(account: AccountType) -> Self {
        Self {
            account: Arc::new(Mutex::new(account)),
            session_token: Arc::new(Mutex::new(None)),
            api_lock: Arc::new(tokio::sync::Mutex::new(())),
            login_lock: Arc::new(tokio::sync::Mutex::new(())),
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

    /// Non-blocking hint for session selection: true if this account's api_lock is
    /// currently free (no in-flight call). The guard is dropped immediately, so this
    /// only biases routing toward idle accounts; the real lock is taken in
    /// call_api_with_timeout.
    #[must_use]
    pub fn try_reserve(&self) -> bool {
        self.api_lock.try_lock().is_ok()
    }

    /// Serializes re-login for a single account so concurrent in-flight requests
    /// that all observe an expired token do not each issue their own login. This
    /// is a dedicated lock (not `api_lock`) because `tokio::sync::Mutex` is not
    /// reentrant and `call_api_with_timeout` re-acquires `api_lock`.
    pub async fn lock_login(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.login_lock.lock().await
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
