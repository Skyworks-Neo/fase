use super::*;

use lasso::{Spur, ThreadedRodeo};
use serde::{Deserializer, Serializer};
use std::{
    cmp::Ordering,
    hash::{Hash, Hasher},
    sync::{Arc, OnceLock},
};

#[derive(Debug, Default)]
pub struct LabelPool {
    inner: ThreadedRodeo<Spur>,
}

impl LabelPool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    pub fn global() -> Arc<Self> {
        static GLOBAL: OnceLock<Arc<LabelPool>> = OnceLock::new();
        Arc::clone(GLOBAL.get_or_init(Self::shared))
    }

    pub fn intern(self: &Arc<Self>, value: &str) -> Label {
        let key = self.inner.get_or_intern(value);
        Label {
            id: key,
            pool: Arc::clone(self),
        }
    }

    pub fn resolve(&self, id: Spur) -> &str {
        self.inner.resolve(&id)
    }
}

#[derive(Debug, Clone)]
pub struct Label {
    id: Spur,
    pool: Arc<LabelPool>,
}

impl Label {
    pub fn intern(value: &str) -> Self {
        LabelPool::global().intern(value)
    }

    pub fn id(&self) -> Spur {
        self.id
    }

    pub fn pool(&self) -> &Arc<LabelPool> {
        &self.pool
    }

    pub fn as_str(&self) -> &str {
        self.pool.resolve(self.id)
    }

    pub fn override_value(&self, value: &str) -> Self {
        self.pool.intern(value)
    }

    fn assert_same_pool(&self, other: &Self) {
        debug_assert!(
            Arc::ptr_eq(&self.pool, &other.pool),
            "labels from different pools cannot be compared by id"
        );
    }
}

impl PartialEq for Label {
    fn eq(&self, other: &Self) -> bool {
        self.assert_same_pool(other);
        self.id == other.id
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
        self.assert_same_pool(other);
        self.id.cmp(&other.id)
    }
}

impl Hash for Label {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl Serialize for Label {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Label {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = <Box<str>>::deserialize(deserializer)?;
        Ok(Self::intern(&value))
    }
}

impl HashContent for Label {
    fn hash_content(&self, state: &mut Sha512) {
        hash_str(state, self.as_str());
    }
}
