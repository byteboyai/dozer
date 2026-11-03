//! 消费者应用后 ack；正文许可直到最后一个消息引用释放，Clone 不复制正文。
use dozer_model::TerminalEvent;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Notify, OwnedSemaphorePermit, mpsc};

pub struct SessionStream {
    pub events: mpsc::Receiver<StreamItem>,
}

#[derive(Debug)]
pub struct StreamAck {
    pub notify: Notify,
    pub applied: AtomicBool,
}
impl StreamAck {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            notify: Notify::new(),
            applied: AtomicBool::new(false),
        })
    }
}
#[derive(Debug)]
struct Ticket {
    _permits: Vec<OwnedSemaphorePermit>,
    ack: Arc<StreamAck>,
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.ack.notify.notify_one();
    }
}

#[derive(Debug, Clone)]
pub struct StreamItem {
    pub event: Arc<TerminalEvent>,
    ticket: Arc<Ticket>,
}
impl PartialEq for StreamItem {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.event, &other.event)
    }
}
impl StreamItem {
    pub fn new(
        event: TerminalEvent,
        permits: Vec<OwnedSemaphorePermit>,
        ack: Arc<StreamAck>,
    ) -> Self {
        Self {
            event: Arc::new(event),
            ticket: Arc::new(Ticket {
                _permits: permits,
                ack,
            }),
        }
    }
    pub fn acknowledge(&self) {
        self.ticket.ack.applied.store(true, Ordering::Release);
        self.ticket.ack.notify.notify_one();
    }
}
