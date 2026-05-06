use super::*;

use regex::Regex;
use serde::{Deserializer, Serializer};

use std::sync::LazyLock;

static DNS_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)*$").unwrap()
});

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Var(Box<str>);

#[derive(Debug)]
#[non_exhaustive]
pub enum VarError {
    Empty,
    Format { val: Box<str> },
}

impl Var {
    pub fn new<T>(value: T) -> Result<Self, VarError>
    where
        T: Into<Box<str>>,
    {
        Self::validate(value.into())
    }

    fn validate(value: Box<str>) -> Result<Self, VarError> {
        if value.is_empty() {
            return Err(VarError::Empty);
        }
        if !DNS_NAME.is_match(&value) {
            return Err(VarError::Format { val: value });
        }
        Ok(Self(value))
    }
}

impl Serialize for Var {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.0.as_ref())
    }
}

impl<'de> Deserialize<'de> for Var {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = <Box<str>>::deserialize(deserializer)?;
        Var::validate(value).map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Display for VarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VarError::Empty => write!(f, "empty"),
            VarError::Format { val } => write!(f, "{val} is not a valid value"),
        }
    }
}

impl std::fmt::Display for Var {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl HashContent for Var {
    fn hash_content(&self, state: &mut sha2::Sha256) {
        hash_str(state, &self.0);
    }
}
