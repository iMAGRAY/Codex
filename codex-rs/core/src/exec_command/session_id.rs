use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub u32);

impl SessionId {
    pub fn new(id: u32) -> Self {
        Self(id)
    }
}
