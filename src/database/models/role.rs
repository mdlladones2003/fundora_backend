use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Guest,
    Donor,
    Organizer,
    Administrator,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Guest => "guest",
            Self::Donor => "donor",
            Self::Organizer => "organizer",
            Self::Administrator => "administrator",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "guest" => Some(Self::Guest),
            "donor" => Some(Self::Donor),
            "organizer" => Some(Self::Organizer),
            "administrator" => Some(Self::Administrator),
            _ => None,
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str((*self).as_str())
    }
}
