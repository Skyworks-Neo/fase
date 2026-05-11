use fase_api::{HashContent, Label, Resource, Sha512, hash_str};
use serde::{Deserialize, Serialize};

pub type CliResource = Resource<Label, Expr>;

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Expr(Box<str>);

impl Expr {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl HashContent for Expr {
    fn hash_content(&self, state: &mut Sha512) {
        hash_str(state, self.as_str());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fase_api::Sha;

    #[test]
    fn parses_resource_with_interned_labels() {
        let package = include_str!("../../fase-api/contrib/package.yml");
        let resource: CliResource = serde_yml::from_str(package).unwrap();

        assert_eq!(resource.sha512().to_string().len(), 128);
    }
}
