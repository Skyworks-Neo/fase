use super::*;

use lasso::{Spur, ThreadedRodeo};
use serde::Serializer;
use std::{
    cmp::Ordering,
    hash::{Hash, Hasher},
    sync::Arc,
};

#[derive(Debug, Clone)]
pub struct LabelPool {
    inner: Arc<ThreadedRodeo<Spur>>,
}

impl LabelPool {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ThreadedRodeo::new()),
        }
    }

    pub fn intern(&self, value: &str) -> Label {
        let key = self.inner.get_or_intern(value);
        Label {
            id: key,
            pool: self.clone(),
        }
    }

    pub fn resolve(&self, id: Spur) -> &str {
        self.inner.resolve(&id)
    }
}

impl Default for LabelPool {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct Label {
    id: Spur,
    pool: LabelPool,
}

impl Label {
    pub fn id(&self) -> Spur {
        self.id
    }

    pub fn pool(&self) -> &LabelPool {
        &self.pool
    }

    pub fn override_value(&self, value: &str) -> Self {
        self.pool.intern(value)
    }
}

impl AsRef<str> for Label {
    fn as_ref(&self) -> &str {
        self.pool().resolve(self.id)
    }
}

impl PartialEq for Label {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for Label {}

impl PartialOrd for Label {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Label {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_ref().cmp(other.as_ref())
    }
}

impl Hash for Label {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state);
    }
}

impl Serialize for Label {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_ref())
    }
}

impl HashContent for Label {
    fn hash_content(&self, state: &mut Sha256) {
        self.as_ref().hash_content(state);
    }
}
