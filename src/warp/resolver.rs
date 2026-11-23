use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::platform;

static ACTIVE_429_COUNT: AtomicU32 = AtomicU32::new(0);
static RESET_IN_PROGRESS: Mutex<()> = Mutex::const_new(());

pub struct WarpResolver {
    pub max_retries: u32,
    pub reset_delay_ms: u64,
}

impl WarpResolver {
    pub fn new(max_retries: u32, reset_delay_ms: u64) -> Self {
        Self {
            max_retries,
            reset_delay_ms,
        }
    }

    /// Called when the upstream answers HTTP 429. Runs a platform-specific WARP
    /// reset (Linux service restart, Windows registration rebuild) and reports
    /// whether the caller should retry the request.
    pub async fn handle_429(&self, retry_count: u32) -> bool {
        if retry_count >= self.max_retries {
            warn!(
                "Max retry attempts ({}) reached, giving up",
                self.max_retries
            );
            return false;
        }

        let _guard = match RESET_IN_PROGRESS.try_lock() {
            Ok(g) => g,
            Err(_) => {
                info!("WARP reset already in progress, waiting...");
                tokio::time::sleep(Duration::from_millis(self.reset_delay_ms)).await;
                return true;
            }
        };

        let count = ACTIVE_429_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
        info!(
            "HTTP 429 detected (count: {}). Initiating WARP reset...",
            count
        );

        if let Err(e) = platform::reset_full(true) {
            error!("WARP reset failed: {}", e);
            ACTIVE_429_COUNT.fetch_sub(1, Ordering::SeqCst);
            return false;
        }

        info!("WARP reset complete. Waiting for connection stabilization...");
        tokio::time::sleep(Duration::from_millis(self.reset_delay_ms)).await;

        ACTIVE_429_COUNT.fetch_sub(1, Ordering::SeqCst);
        true
    }
}
