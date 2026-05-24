// seat.rs — org.freedesktop.login1.Seat interface (one seat, "seat0").
//
// We never expect multi-seat. Most desktop apps just need Seat to
// resolve so they can read .ActiveSession. seatd / consolekit-derived
// systems all expose this same shape.

use std::sync::{Arc, Mutex};

use zbus::interface;
use zbus::zvariant::OwnedObjectPath;

use crate::state::{session_object_path, AppState};

pub struct Seat {
    pub state: Arc<Mutex<AppState>>,
    pub seat_id: String,
}

impl Seat {
    pub fn new(state: Arc<Mutex<AppState>>, seat_id: String) -> Self {
        Self { state, seat_id }
    }
}

#[interface(name = "org.freedesktop.login1.Seat")]
impl Seat {
    #[zbus(property)]
    fn id(&self) -> String {
        self.seat_id.clone()
    }

    #[zbus(property)]
    fn active_session(&self) -> (String, OwnedObjectPath) {
        // First "active"-state session attached to us.
        let st = self.state.lock().unwrap();
        if let Some(s) = st
            .sessions
            .values()
            .find(|s| s.seat == self.seat_id && s.state == "active")
        {
            (s.id.clone(), session_object_path(&s.id))
        } else {
            (String::new(), zbus::zvariant::ObjectPath::from_static_str_unchecked("/").into())
        }
    }

    #[zbus(property)]
    fn sessions(&self) -> Vec<(String, OwnedObjectPath)> {
        let st = self.state.lock().unwrap();
        st.sessions
            .values()
            .filter(|s| s.seat == self.seat_id)
            .map(|s| (s.id.clone(), session_object_path(&s.id)))
            .collect()
    }

    #[zbus(property)]
    fn can_graphical(&self) -> bool {
        // T450 has a graphical iGPU; this is "the seat has a display".
        true
    }

    #[zbus(property, name = "CanTTY")]
    fn can_tty(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_multi_session(&self) -> bool {
        // Switch users via VT switching. Stub-supported.
        true
    }

    #[zbus(property)]
    fn idle_hint(&self) -> bool {
        let st = self.state.lock().unwrap();
        st.sessions
            .values()
            .filter(|s| s.seat == self.seat_id)
            .all(|s| s.idle_hint)
    }
}
