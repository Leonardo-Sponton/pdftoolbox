//! Execução de tarefas em segundo plano com progresso, cancelamento e fila.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use crate::error::{OpError, OpResult};
use crate::operations::Outcome;

#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub fn check(&self) -> OpResult<()> {
        if self.is_cancelled() { Err(OpError::Cancelled) } else { Ok(()) }
    }
}

type ProgressFn = Box<dyn Fn(f64, &str) + Send + Sync>;

/// Contexto passado às operações.
pub struct JobCtx {
    pub cancel: CancelToken,
    progress: ProgressFn,
}

impl JobCtx {
    pub fn new(cancel: CancelToken, progress: impl Fn(f64, &str) + Send + Sync + 'static) -> Self {
        JobCtx { cancel, progress: Box::new(progress) }
    }

    /// Contexto sem relato de progresso (testes, uso em linha de comando).
    pub fn silent() -> Self {
        Self::new(CancelToken::new(), |_, _| {})
    }

    pub fn report(&self, fraction: f64, text: &str) {
        (self.progress)(fraction.clamp(0.0, 1.0), text);
    }
}

pub enum JobEvent {
    Queued,
    Progress(f64, String),
    Finished(OpResult<Outcome>),
}

pub struct JobHandle {
    pub cancel: CancelToken,
    pub events: async_channel::Receiver<JobEvent>,
}

/// Número máximo de tarefas executando ao mesmo tempo; as demais aguardam.
const MAX_CONCURRENT: usize = 2;

struct Slots {
    used: Mutex<usize>,
    freed: Condvar,
}

fn slots() -> &'static Slots {
    static SLOTS: OnceLock<Slots> = OnceLock::new();
    SLOTS.get_or_init(|| Slots { used: Mutex::new(0), freed: Condvar::new() })
}

struct SlotGuard;

impl SlotGuard {
    fn acquire(cancel: &CancelToken) -> Option<Self> {
        let s = slots();
        let mut used = s.used.lock().unwrap_or_else(|e| e.into_inner());
        while *used >= MAX_CONCURRENT {
            if cancel.is_cancelled() {
                return None;
            }
            used =
                s.freed.wait_timeout(used, std::time::Duration::from_millis(200)).unwrap_or_else(|e| e.into_inner()).0;
        }
        *used += 1;
        Some(SlotGuard)
    }
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        let s = slots();
        *s.used.lock().unwrap_or_else(|e| e.into_inner()) -= 1;
        s.freed.notify_one();
    }
}

/// Inicia uma tarefa em uma thread de trabalho.
pub fn spawn<F>(work: F) -> JobHandle
where
    F: FnOnce(&JobCtx) -> OpResult<Outcome> + Send + 'static,
{
    let (tx, rx) = async_channel::unbounded();
    let cancel = CancelToken::new();
    let token = cancel.clone();
    std::thread::Builder::new()
        .name("pdftoolbox-tarefa".into())
        .spawn(move || {
            let _ = tx.send_blocking(JobEvent::Queued);
            let result = match SlotGuard::acquire(&token) {
                None => Err(OpError::Cancelled),
                Some(_slot) => {
                    let progress_tx = tx.clone();
                    let ctx = JobCtx::new(token, move |f, text| {
                        let _ = progress_tx.send_blocking(JobEvent::Progress(f, text.to_string()));
                    });
                    catch_unwind(AssertUnwindSafe(|| work(&ctx))).unwrap_or_else(|_| {
                        Err(OpError::OutputInvalid("falha interna inesperada durante o processamento".into()))
                    })
                }
            };
            let _ = tx.send_blocking(JobEvent::Finished(result));
        })
        .expect("criar thread de trabalho");
    JobHandle { cancel, events: rx }
}
