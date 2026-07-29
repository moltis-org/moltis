use {
    anyhow::Result,
    std::{
        collections::HashMap,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
    },
    tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore},
};

#[derive(Default)]
pub(super) struct OrderedDeliveryGates {
    gates: Mutex<HashMap<String, Arc<DeliveryGate>>>,
}

struct DeliveryGate {
    latest_order: AtomicU64,
    semaphore: Arc<Semaphore>,
}

impl DeliveryGate {
    fn new(order: u64) -> Self {
        Self {
            latest_order: AtomicU64::new(order),
            semaphore: Arc::new(Semaphore::new(1)),
        }
    }
}

pub(super) struct OrderedDeliveryGuard {
    _gate: Arc<DeliveryGate>,
    _permit: OwnedSemaphorePermit,
}

impl OrderedDeliveryGates {
    /// Register the order before waiting, then serialize delivery for a session.
    /// Completed high-water marks remain cached so a detached older task that
    /// starts late is still rejected.
    pub(super) async fn start(
        &self,
        session_key: &str,
        order: u64,
    ) -> Result<Option<OrderedDeliveryGuard>> {
        let gate = {
            let mut gates = self.gates.lock().await;
            Arc::clone(
                gates
                    .entry(session_key.to_string())
                    .or_insert_with(|| Arc::new(DeliveryGate::new(order))),
            )
        };

        gate.latest_order.fetch_max(order, Ordering::AcqRel);
        let permit = Arc::clone(&gate.semaphore).acquire_owned().await?;
        if gate.latest_order.load(Ordering::Acquire) > order {
            return Ok(None);
        }

        Ok(Some(OrderedDeliveryGuard {
            _gate: gate,
            _permit: permit,
        }))
    }

    #[cfg(test)]
    pub(super) async fn latest_order(&self, session_key: &str) -> Option<u64> {
        self.gates
            .lock()
            .await
            .get(session_key)
            .map(|gate| gate.latest_order.load(Ordering::Acquire))
    }
}
