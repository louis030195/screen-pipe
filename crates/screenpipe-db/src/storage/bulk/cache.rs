// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::{storage_error, Cached, HybridStorage, Record, Value};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, OnceLock, Weak},
};

type Flight = OnceLock<Result<Cached, String>>;

#[derive(Default)]
pub(super) struct Cache(Mutex<State>);

#[derive(Default)]
struct State {
    ready: VecDeque<(String, usize, Cached)>,
    loading: HashMap<String, Weak<Flight>>,
}

impl Cached {
    fn bytes(&self) -> usize {
        match self {
            #[cfg(feature = "storage-bench-experiments")]
            Self::Frames(rows) => {
                rows.capacity() * std::mem::size_of::<super::super::FramePayload>()
                    + rows
                        .iter()
                        .map(super::super::FramePayload::bytes)
                        .sum::<usize>()
            }
            Self::ElementIndex(index) => index.bytes(),
            Self::Records(rows) => {
                rows.capacity() * std::mem::size_of::<Record>()
                    + rows
                        .iter()
                        .map(|row| {
                            row.values.capacity() * std::mem::size_of::<Value>()
                                + row
                                    .values
                                    .iter()
                                    .map(|v| match v {
                                        Value::Text(s) => s.capacity(),
                                        _ => 0,
                                    })
                                    .sum::<usize>()
                        })
                        .sum::<usize>()
            }
        }
    }
}

impl Cache {
    #[cfg(test)]
    pub(super) fn max_in_flight_readers(&self) -> usize {
        self.0
            .lock()
            .unwrap()
            .loading
            .values()
            .map(Weak::strong_count)
            .max()
            .unwrap_or(0)
    }

    pub(super) fn get_or_load(
        &self,
        storage: &HybridStorage,
        key: String,
        load: impl FnOnce() -> Result<Cached, sqlx::Error>,
    ) -> Result<Cached, sqlx::Error> {
        let flight = {
            let mut state = self.0.lock().map_err(storage_error)?;
            if let Some(index) = state.ready.iter().position(|(k, _, _)| k == &key) {
                let hit = state.ready.remove(index).unwrap();
                let value = hit.2.clone();
                state.ready.push_back(hit);
                return Ok(value);
            }
            state.loading.retain(|_, flight| flight.strong_count() != 0);
            match state.loading.get(&key).and_then(Weak::upgrade) {
                Some(flight) => flight,
                None => {
                    let flight = Arc::new(OnceLock::new());
                    state.loading.insert(key.clone(), Arc::downgrade(&flight));
                    flight
                }
            }
        };
        // Only readers of this exact immutable projection wait on its flight.
        // SQLite invokes payload callbacks synchronously on its worker. The
        // runtime-independent executor shares frame decoder admission without
        // nesting a Tokio runtime or holding the cache mutex across I/O.
        flight.get_or_init(|| {
            let result = (|| {
                let _permit = futures::executor::block_on(async {
                    tokio::select! {
                        biased;
                        _ = storage.closing.cancelled() => Err(sqlx::Error::PoolClosed),
                        permit = storage.decoder.acquire() => permit.map_err(|_| sqlx::Error::PoolClosed),
                    }
                })?;
                #[cfg(test)]
                {
                    let hook = storage.bulk.decode_hook.lock().unwrap().clone();
                    if let Some(hook) = hook { hook(&key); }
                }
                let value = load()?;
                let bytes = value.bytes();
                let mut evicted = Vec::new();
                {
                    let mut state = self.0.lock().map_err(storage_error)?;
                    let mut retained: usize = state.ready.iter().map(|entry| entry.1).sum();
                    while retained + bytes > storage.descriptor.budget.decode_bytes {
                        let Some(entry) = state.ready.pop_front() else { break };
                        retained -= entry.1;
                        evicted.push(entry);
                    }
                    if bytes <= storage.descriptor.budget.decode_bytes {
                        state.ready.push_back((key, bytes, value.clone()));
                    }
                }
                // Large record allocations are freed outside the cache lock.
                drop(evicted);
                Ok(value)
            })();
            result.map_err(|e: sqlx::Error| e.to_string())
        }).clone().map_err(storage_error)
    }

    pub(super) fn retire(&self, paths: &[String]) -> Result<(), sqlx::Error> {
        let mut evicted = Vec::new();
        {
            let mut state = self.0.lock().map_err(storage_error)?;
            for i in (0..state.ready.len()).rev() {
                if paths.iter().any(|p| state.ready[i].0.contains(p)) {
                    evicted.push(state.ready.remove(i).unwrap());
                }
            }
        }
        drop(evicted);
        Ok(())
    }
}
