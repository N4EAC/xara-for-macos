use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

pub(crate) const AUDIO_QUEUE: usize = 64;

pub trait Source: Send + 'static {
    fn events(&mut self) -> Vec<String>;
    fn status(&mut self) -> Option<String>;
    fn audio(&mut self, f: &mut dyn FnMut(&[f32]));
    fn want_audio(&mut self, on: bool);
}

pub struct Publisher<E = String, S = String> {
    events: SyncSender<E>,
    state: Arc<Mutex<S>>,
    audio: SyncSender<Vec<f32>>,
    want_audio: Arc<AtomicBool>,
    dropped: Arc<AtomicUsize>,
}

pub struct Subscriber<E = String, S = String> {
    pub events: Receiver<E>,
    state: Arc<Mutex<S>>,
    pub audio: Receiver<Vec<f32>>,
    want_audio: Arc<AtomicBool>,
    dropped: Arc<AtomicUsize>,
}

pub fn channel<E, S: Default>(queue: usize) -> (Publisher<E, S>, Subscriber<E, S>) {
    let (tx, rx) = sync_channel(queue);
    let (atx, arx) = sync_channel(AUDIO_QUEUE);
    let state = Arc::new(Mutex::new(S::default()));
    let want = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicUsize::new(0));
    (
        Publisher { events: tx, state: state.clone(), audio: atx, want_audio: want.clone(), dropped: dropped.clone() },
        Subscriber { events: rx, state, audio: arx, want_audio: want, dropped },
    )
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<E, S> Publisher<E, S> {
    pub fn event(&self, e: E) {
        if let Err(TrySendError::Full(_)) = self.events.try_send(e) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn status(&self, s: S) {
        *lock(&self.state) = s;
    }

    pub fn update(&self, f: impl FnOnce(&mut S)) {
        f(&mut lock(&self.state));
    }

    pub fn dropped(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn wants_audio(&self) -> bool {
        self.want_audio.load(Ordering::Relaxed)
    }

    pub fn audio(&self, x: &[f64]) {
        if self.wants_audio() {
            let _ = self.audio.try_send(x.iter().map(|&v| v as f32).collect());
        }
    }
}

impl<E, S> Subscriber<E, S> {
    pub(crate) fn state(&self) -> MutexGuard<'_, S> {
        lock(&self.state)
    }

    pub fn dropped(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn want_audio(&self, on: bool) {
        self.want_audio.store(on, Ordering::Relaxed);
    }

    pub(crate) fn drain_audio(&self, f: &mut dyn FnMut(&[f32])) {
        for b in self.audio.try_iter() {
            f(&b);
        }
    }
}

impl Source for Subscriber {
    fn events(&mut self) -> Vec<String> {
        self.events.try_iter().collect()
    }

    fn status(&mut self) -> Option<String> {
        let s = self.state();
        (!s.is_empty()).then(|| s.clone())
    }

    fn audio(&mut self, f: &mut dyn FnMut(&[f32])) {
        self.drain_audio(f);
    }

    fn want_audio(&mut self, on: bool) {
        Subscriber::want_audio(self, on);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_bounded_and_counted() {
        let (p, mut s): (Publisher, Subscriber) = channel(2);
        for i in 0..5 {
            p.event(i.to_string());
        }
        assert_eq!((Source::events(&mut s), p.dropped()), (vec!["0".to_string(), "1".to_string()], 3));
        drop(s);
        p.event("after".into());
    }

    #[test]
    fn status_is_the_latest_and_absent_until_set() {
        let (p, mut s): (Publisher, Subscriber) = channel(2);
        assert_eq!(Source::status(&mut s), None);
        p.status("a".into());
        p.status("b".into());
        assert_eq!((Source::status(&mut s), Source::status(&mut s)), (Some("b".into()), Some("b".into())));
    }

    #[test]
    fn audio_only_while_wanted() {
        let (p, mut s): (Publisher, Subscriber) = channel(2);
        p.audio(&[0.5; 4]);
        let mut got = Vec::new();
        Source::audio(&mut s, &mut |b| got.push(b.to_vec()));
        assert!(got.is_empty());
        Source::want_audio(&mut s, true);
        for _ in 0..AUDIO_QUEUE + 5 {
            p.audio(&[0.25; 2]);
        }
        Source::audio(&mut s, &mut |b| got.push(b.to_vec()));
        assert_eq!((got.len(), &got[0]), (AUDIO_QUEUE, &vec![0.25f32; 2]));
    }
}
