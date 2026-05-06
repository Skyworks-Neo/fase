use super::*;

pub trait HashContent {
    fn hash_content(&self, sha: &mut sha2::Sha256);
}

pub trait Sha {
    fn sha256(&self) -> ShaSum;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShaSum([u8; 32]);

impl ShaSum {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Display for ShaSum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl std::str::FromStr for ShaSum {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 64 {
            return Err("invalid sha256 sum");
        }

        let mut bytes = [0; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            let start = index * 2;
            *byte = u8::from_str_radix(&value[start..start + 2], 16)
                .map_err(|_| "invalid sha256 sum")?;
        }
        Ok(Self(bytes))
    }
}

pub fn hash_field(sha: &mut sha2::Sha256, name: &str) {
    hash_bytes(sha, b"field", name.as_bytes());
}

pub fn hash_str(sha: &mut sha2::Sha256, value: &str) {
    hash_bytes(sha, b"str", value.as_bytes());
}

pub fn hash_len(sha: &mut sha2::Sha256, len: usize) {
    sha.update((len as u64).to_be_bytes());
}

fn hash_bytes(sha: &mut sha2::Sha256, tag: &[u8], value: &[u8]) {
    sha.update(tag);
    hash_len(sha, value.len());
    sha.update(value);
}

impl Sha for Resource {
    fn sha256(&self) -> ShaSum {
        match self {
            Resource::Act(resource) => resource.sha256(),
            Resource::Package(resource) => resource.sha256(),
            Resource::Kustomize(resource) => resource.sha256(),
            Resource::Install(resource) => resource.sha256(),
            Resource::Build(resource) => resource.sha256(),
        }
    }
}

macro_rules! impl_sha {
    ($($resource:ty),* $(,)?) => {
        $(
            impl Sha for $resource {
                fn sha256(&self) -> ShaSum {
                    let mut sha = sha2::Sha256::new();
                    hash_field(&mut sha, "apiVersion");
                    hash_str(&mut sha, <Self as ResourceKind>::API_VERSION);
                    hash_field(&mut sha, "kind");
                    hash_str(&mut sha, <Self as ResourceKind>::KIND);
                    self.hash_content(&mut sha);
                    ShaSum(sha.finalize().into())
                }
            }
        )*
    };
}

impl_sha!(Act, Package, Kustomize, Install, Build);

impl HashContent for LabelMap {
    fn hash_content(&self, sha: &mut sha2::Sha256) {
        hash_len(sha, self.inner.len());
        for (key, value) in &self.inner {
            key.hash_content(sha);
            value.hash_content(sha);
        }
    }
}
