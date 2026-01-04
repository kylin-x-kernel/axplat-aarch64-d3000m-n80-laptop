/// Input event type
#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    /// Key press event with ASCII character
    KeyPress(u8),
    /// Key release event (not used yet)
    KeyRelease(u8),
}

/// Input driver operations
pub trait InputDriverOps: Send + Sync {
    /// Check if there is pending input
    fn pending_input(&self) -> bool;
    
    /// Read an input event
    fn read_event(&self) -> Option<InputEvent>;
}
