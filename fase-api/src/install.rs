use super::*;

#[derive(Debug, Clone, Deserialize)]
pub struct Install {
    pub wants: Vec<LabelMap>,
}

impl AnyResource for Install {
    const KIND: &'static str = "Install";
}
