//! The callback lists. Lock-free to read, because the detour reads them every frame on the game
//! thread; registration happens a handful of times at startup.

use std::sync::atomic::{AtomicUsize, Ordering};

/// When a callback runs relative to the original update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    /// Before the original: whatever it writes, the original sees in the same frame.
    Before,
    /// After the original: whatever the update holds, it has let go of.
    After,
}

/// A callback. The argument is the net session manager the game passed.
pub type TickFn = fn(usize);

/// How many callbacks each list holds. Two features use it today.
pub const SLOTS: usize = 8;

/// Two fixed lists of function pointers, filled front to back. A zero slot ends the list.
pub struct Registry {
    before: [AtomicUsize; SLOTS],
    after: [AtomicUsize; SLOTS],
}

impl Registry {
    /// An empty registry.
    pub const fn new() -> Self {
        Self {
            before: [const { AtomicUsize::new(0) }; SLOTS],
            after: [const { AtomicUsize::new(0) }; SLOTS],
        }
    }

    fn list(&self, when: When) -> &[AtomicUsize; SLOTS] {
        match when {
            When::Before => &self.before,
            When::After => &self.after,
        }
    }

    /// Add `callback` to the `when` list. Registering the same function twice is a no-op.
    ///
    /// # Errors
    ///
    /// When the list is full.
    pub fn add(&self, when: When, callback: TickFn) -> Result<(), &'static str> {
        let raw = callback as usize;
        for slot in self.list(when) {
            match slot.compare_exchange(0, raw, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Ok(()),
                Err(existing) if existing == raw => return Ok(()),
                Err(_) => {}
            }
        }
        Err("the callback list is full")
    }

    /// Call every callback in the `when` list, in registration order, with `session`.
    pub fn run(&self, when: When, session: usize) {
        for slot in self.list(when) {
            let raw = slot.load(Ordering::Acquire);
            if raw == 0 {
                return;
            }
            // SAFETY: a non-zero slot is only ever written by `add`, from a `TickFn`.
            let callback: TickFn = unsafe { std::mem::transmute::<usize, TickFn>(raw) };
            let _ = std::panic::catch_unwind(|| callback(session));
        }
    }

    /// How many callbacks the `when` list holds.
    pub fn len(&self, when: When) -> usize {
        self.list(when)
            .iter()
            .take_while(|slot| slot.load(Ordering::Acquire) != 0)
            .count()
    }

    /// Whether both lists are empty.
    pub fn is_empty(&self) -> bool {
        self.len(When::Before) == 0 && self.len(When::After) == 0
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static SEEN: Mutex<Vec<(&str, usize)>> = Mutex::new(Vec::new());

    fn first(session: usize) {
        SEEN.lock().unwrap().push(("first", session));
    }
    fn second(session: usize) {
        SEEN.lock().unwrap().push(("second", session));
    }
    fn after(session: usize) {
        SEEN.lock().unwrap().push(("after", session));
    }
    fn panics(_: usize) {
        panic!("a feature's bug");
    }

    #[test]
    fn callbacks_run_in_registration_order_and_a_panic_stops_nobody() {
        let registry = Registry::new();
        registry.add(When::Before, first).unwrap();
        registry.add(When::Before, panics).unwrap();
        registry.add(When::Before, second).unwrap();
        registry.add(When::After, after).unwrap();
        SEEN.lock().unwrap().clear();
        registry.run(When::Before, 7);
        registry.run(When::After, 7);
        assert_eq!(
            *SEEN.lock().unwrap(),
            vec![("first", 7), ("second", 7), ("after", 7)]
        );
    }

    #[test]
    fn registering_twice_is_once() {
        let registry = Registry::new();
        registry.add(When::After, after).unwrap();
        registry.add(When::After, after).unwrap();
        assert_eq!(registry.len(When::After), 1);
        assert_eq!(registry.len(When::Before), 0);
        assert!(!registry.is_empty());
    }

    #[test]
    fn a_full_list_refuses() {
        // Distinct bodies, so the linker cannot fold them into one address.
        fn f0(s: usize) {
            SEEN.lock().unwrap().push(("f0", s));
        }
        fn f1(s: usize) {
            SEEN.lock().unwrap().push(("f1", s));
        }
        fn f2(s: usize) {
            SEEN.lock().unwrap().push(("f2", s));
        }
        fn f3(s: usize) {
            SEEN.lock().unwrap().push(("f3", s));
        }
        fn f4(s: usize) {
            SEEN.lock().unwrap().push(("f4", s));
        }
        fn f5(s: usize) {
            SEEN.lock().unwrap().push(("f5", s));
        }
        fn f6(s: usize) {
            SEEN.lock().unwrap().push(("f6", s));
        }
        fn f7(s: usize) {
            SEEN.lock().unwrap().push(("f7", s));
        }
        fn f8(s: usize) {
            SEEN.lock().unwrap().push(("f8", s));
        }
        let registry = Registry::new();
        for f in [f0, f1, f2, f3, f4, f5, f6, f7] {
            registry.add(When::Before, f).unwrap();
        }
        assert!(registry.add(When::Before, f8).is_err());
    }
}
