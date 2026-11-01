//! 简单自旋锁：`AtomicBool` + `pause`，no_std 可用。
//!
//! 用法：
//! ```ignore
//! static LOCK: Spinlock<u64> = Spinlock::new(0);
//! {
//!     let mut g = LOCK.lock();
//!     *g += 1;
//! } // Drop 自动解锁
//! ```

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

pub struct Spinlock<T: ?Sized> {
    locked: AtomicBool,
    inner: UnsafeCell<T>,
}

unsafe impl<T: ?Sized + Send + Sync> Sync for Spinlock<T> {}
unsafe impl<T: ?Sized + Send> Send for Spinlock<T> {}

impl<T> Spinlock<T> {
    pub const fn new(val: T) -> Self {
        Spinlock {
            locked: AtomicBool::new(false),
            inner: UnsafeCell::new(val),
        }
    }

    /// 尝试非阻塞获取（返回 None 表示已被他人持有）。
    pub fn try_lock(&self) -> Option<SpinGuard<'_, T>> {
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(SpinGuard { lock: self })
        } else {
            None
        }
    }

    /// 阻塞获取，`pause` 忙等（禁止中断的上下文可用）。
    #[inline(always)]
    pub fn lock(&self) -> SpinGuard<'_, T> {
        loop {
            if let Some(g) = self.try_lock() {
                return g;
            }
            while self.locked.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
    }

    /// 无条件释放（通常由 `SpinGuard` 的 `Drop` 自动做）。
    pub fn force_unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }
}

pub struct SpinGuard<'a, T: ?Sized> {
    lock: &'a Spinlock<T>,
}

impl<'a, T: ?Sized> Drop for SpinGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}

impl<'a, T: ?Sized> Deref for SpinGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.inner.get() }
    }
}

impl<'a, T: ?Sized> DerefMut for SpinGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.inner.get() }
    }
}
