//! Mapping attribution only; no mapping, locking or protection decisions.
use std::time::Instant;
use victory_gpu::framelog::{self, T};

#[derive(Clone, Copy)]
pub(super) enum Origin {
    Direct, Scatter, Alias, Batch, Tail,
    #[cfg(test)]
    Other,
}

pub(super) struct Attempt(Option<(Instant, T)>);

impl Attempt {
    pub(super) fn new(origin: Origin, bytes: usize) -> Self {
        if !framelog::on() { return Self(None); }
        let (calls, volume, time) = match origin {
            Origin::Direct => (T::DirectMapCalls, T::DirectMapBytes, T::DirectMapUs),
            Origin::Scatter => (T::ScatterMapCalls, T::ScatterMapBytes, T::ScatterMapUs),
            Origin::Alias => (T::AliasMapCalls, T::AliasMapBytes, T::AliasMapUs),
            Origin::Batch => (T::BatchMapCalls, T::BatchMapBytes, T::BatchMapUs),
            Origin::Tail => (T::TailMapCalls, T::TailMapBytes, T::TailMapUs),
            #[cfg(test)]
            Origin::Other => (T::OtherMapCalls, T::OtherMapBytes, T::OtherMapUs),
        };
        framelog::add(calls, 1);
        framelog::add(volume, bytes as u64);
        Self(Some((Instant::now(), time)))
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        if let Some((start, counter)) = self.0 {
            framelog::add(counter, start.elapsed().as_micros() as u64);
        }
    }
}

pub(super) fn measured<R>(counter: T, f: impl FnOnce() -> R) -> R {
    if !framelog::on() { return f(); }
    let start = Instant::now();
    let result = f();
    framelog::add(counter, start.elapsed().as_micros() as u64);
    result
}
