use fase_api::{Label, LabelPool, Map, Resource};

use super::*;

#[derive(Debug, Clone, Default)]
pub struct Runtime {
    labels: LabelPool,
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            labels: LabelPool::new(),
        }
    }

    pub fn labels(&self) -> &LabelPool {
        &self.labels
    }

    pub fn label(&self, value: &str) -> Label {
        self.labels.intern(value)
    }

    pub fn intern<E>(&self, resource: Resource<String, E>) -> Resource<Label, E> {
        resource.map(|label| self.label(&label))
    }

    pub fn evaluator(&self) -> Evaluator {
        Evaluator::new()
    }

    pub fn evaluator_with<I, K, V>(&self, bindings: I) -> Evaluator
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Evaluator::with(bindings)
    }

    pub async fn apply(&self, map: Map, context: Context) -> Result<Vec<Artifact>> {
        apply(map, context).await
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn runtime_owns_label_pool() {
        let runtime = Runtime::new();
        let other = Runtime::new();
        let first = runtime.label("name");
        let second = runtime.label("name");

        assert_eq!(first, second);
        assert_eq!(first, other.label("name"));
        assert_ne!(first, other.label("version"));
    }
}
