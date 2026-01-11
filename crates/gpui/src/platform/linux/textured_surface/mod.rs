mod client;
mod display;
mod window;

pub(crate) use client::*;
pub(crate) use display::*;
pub(crate) use window::*;

use std::sync::{Mutex, MutexGuard, OnceLock};

/// Global mutex for serializing GPU context operations.
///
/// This prevents concurrent Vulkan access which can crash NVIDIA drivers
/// when multiple TexturedView instances are created simultaneously.
static GPU_CONTEXT_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

/// Acquires the GPU context lock.
///
/// Returns a guard that will release the lock when dropped.
pub(crate) fn acquire_gpu_lock() -> MutexGuard<'static, ()> {
    let mutex = GPU_CONTEXT_MUTEX.get_or_init(|| Mutex::new(()));
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
