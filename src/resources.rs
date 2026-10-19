//! Resource limits apply only to this application's work, never to other processes.
use std::{marker::PhantomData, rc::Rc};
use windows_sys::Win32::System::Threading::*;

/// Scoped Windows background I/O/CPU scheduling. Not Send: restore on the same thread.
pub struct BackgroundWork {
    background: bool,
    previous: i32,
    _same_thread: PhantomData<Rc<()>>,
}
impl BackgroundWork {
    pub fn enter() -> Self {
        unsafe {
            let thread = GetCurrentThread();
            let previous = GetThreadPriority(thread);
            let background = SetThreadPriority(thread, THREAD_MODE_BACKGROUND_BEGIN) != 0;
            if !background && previous != i32::MAX {
                SetThreadPriority(thread, THREAD_PRIORITY_BELOW_NORMAL);
            }
            Self {
                background,
                previous,
                _same_thread: PhantomData,
            }
        }
    }
}
impl Drop for BackgroundWork {
    fn drop(&mut self) {
        unsafe {
            if self.background {
                SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_END);
            } else if self.previous != i32::MAX {
                SetThreadPriority(GetCurrentThread(), self.previous);
            }
        }
    }
}

pub struct UiExecutor(tokio::runtime::Runtime);
impl iced::Executor for UiExecutor {
    fn new() -> std::io::Result<Self> {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(2)
            .thread_name("straja-sql-ui")
            .enable_all()
            .build()
            .map(Self)
    }
    fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) {
        // Dropping a JoinHandle detaches, rather than cancelling, the task.
        drop(self.0.spawn(future));
    }
    fn enter<R>(&self, f: impl FnOnce() -> R) -> R {
        let _guard = self.0.enter();
        f()
    }
    fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        self.0.block_on(future)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::Executor;
    #[test]
    fn background_priority_is_restored_on_the_worker_thread() {
        std::thread::spawn(|| {
            let before = unsafe { GetThreadPriority(GetCurrentThread()) };
            {
                let _background = BackgroundWork::enter();
            }
            assert_eq!(unsafe { GetThreadPriority(GetCurrentThread()) }, before);
        })
        .join()
        .unwrap();
    }
    #[test]
    fn bounded_executor_supports_timers_and_blocking_work() {
        let executor = UiExecutor::new().unwrap();
        assert_eq!(executor.0.metrics().num_workers(), 2);
        executor.block_on(async {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            assert_eq!(tokio::task::spawn_blocking(|| 42).await.unwrap(), 42);
        });
    }
}
