#![allow(dead_code)]

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use crate::error::AppError;

pub async fn retry_async<T, F, Fut, R>(
    max_retries: u32,
    delay: Duration,
    should_retry: R,
    mut operation: F,
) -> Result<T, AppError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, AppError>>,
    R: Fn(&AppError) -> bool,
{
    let mut last_error = None;
    for attempt in 1..=max_retries {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                if attempt < max_retries && should_retry(&e) {
                    tracing::debug!(
                        "Retry attempt {}/{} after error: {}",
                        attempt,
                        max_retries,
                        e
                    );
                    last_error = Some(e);
                    tokio::time::sleep(delay).await;
                } else {
                    return Err(e);
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| AppError::NetworkError("Max retries reached".to_string())))
}

pub fn is_retryable_error(error: &AppError) -> bool {
    matches!(
        error,
        AppError::NetworkError(_) | AppError::SessionError | AppError::CookieExpired
    )
}

pub struct CachedResource<T> {
    value: Arc<Mutex<T>>,
}

impl<T: Clone> CachedResource<T> {
    pub fn new(initial: T) -> Self {
        Self {
            value: Arc::new(Mutex::new(initial)),
        }
    }

    pub fn get(&self) -> T {
        self.value.lock().clone()
    }

    pub fn set(&self, new_value: T) {
        *self.value.lock() = new_value;
    }

    pub fn replace(&self, new_value: T) -> T {
        std::mem::replace(&mut *self.value.lock(), new_value)
    }
}

impl<T: Default + Clone> Default for CachedResource<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_retry_succeeds_first_try() {
        let result = retry_async(
            3,
            Duration::from_millis(10),
            |_| true,
            || async { Ok::<_, AppError>(42) },
        )
        .await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_retry_succeeds_after_failures() {
        let counter = AtomicU32::new(0);
        let result = retry_async(
            3,
            Duration::from_millis(10),
            |_| true,
            || {
                let count = counter.fetch_add(1, Ordering::SeqCst);
                async move {
                    if count < 2 {
                        Err(AppError::NetworkError("fail".to_string()))
                    } else {
                        Ok(42)
                    }
                }
            },
        )
        .await;
        assert_eq!(result.unwrap(), 42);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_fails_after_max_retries() {
        let counter = AtomicU32::new(0);
        let result = retry_async(
            3,
            Duration::from_millis(10),
            |_| true,
            || {
                counter.fetch_add(1, Ordering::SeqCst);
                async { Err::<i32, _>(AppError::NetworkError("fail".to_string())) }
            },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }
}
