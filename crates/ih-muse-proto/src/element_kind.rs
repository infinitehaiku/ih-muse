// crates/ih-muse-proto/src/element_kind.rs

use serde::{Deserialize, Serialize};

use crate::types::*;
use crate::utils::deterministic_u64_from_str;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ElementKindRegistration {
    id: ElementKindId,
    pub code: String,
    parent_id: Option<ElementKindId>,
    pub name: String,
    pub description: String,
    forwarded: bool,
}

impl ElementKindRegistration {
    pub fn new(
        code: String,
        parent_code: Option<String>,
        name: String,
        description: String,
    ) -> Self {
        Self {
            id: deterministic_u64_from_str(&code),
            code,
            parent_id: parent_code.as_deref().map(deterministic_u64_from_str),
            name,
            description,
            forwarded: false,
        }
    }
}
