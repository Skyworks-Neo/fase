use super::*;

pub trait HashContent {
    fn hash_content(&self, sha: &mut Sha512);
}

pub trait Sha {
    fn sha512(&self) -> ShaSum;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShaSum([u8; 64]);

impl ShaSum {
    pub fn as_bytes(&self) -> &[u8; 64] {
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
        if value.len() != 128 {
            return Err("invalid sha512 sum: expect length is 128");
        }

        let mut bytes = [0; 64];
        for (index, byte) in bytes.iter_mut().enumerate() {
            let start = index * 2;
            // TODO: remove unwrap
            *byte = u8::from_str_radix(&value[start..start + 2], 16).unwrap();
        }
        Ok(Self(bytes))
    }
}

impl Serialize for ShaSum {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ShaSum {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = <Box<str>>::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

pub fn hash_field(sha: &mut Sha512, name: &str) {
    hash_bytes(sha, b"field", name.as_bytes());
}

pub fn hash_str(sha: &mut Sha512, value: &str) {
    hash_bytes(sha, b"str", value.as_bytes());
}

pub fn hash_len(sha: &mut Sha512, len: usize) {
    sha.update((len as u64).to_be_bytes());
}

fn hash_bytes(sha: &mut Sha512, tag: &[u8], value: &[u8]) {
    sha.update(tag);
    hash_len(sha, value.len());
    sha.update(value);
}

impl<K, E> Sha for Resource<K, E>
where
    K: HashContent,
    E: HashContent,
{
    fn sha512(&self) -> ShaSum {
        match self {
            Resource::Act(resource) => resource.sha512(),
            Resource::Package(resource) => resource.sha512(),
            Resource::Kustomize(resource) => resource.sha512(),
            Resource::Install(resource) => resource.sha512(),
            Resource::Build(resource) => resource.sha512(),
            Resource::Realize(resource) => resource.sha512(),
        }
    }
}

fn sha_resource<R>(resource: &R) -> ShaSum
where
    R: ResourceKind + HashContent,
{
    let mut sha = Sha512::new();
    hash_field(&mut sha, "apiVersion");
    hash_str(&mut sha, R::API_VERSION);
    hash_field(&mut sha, "kind");
    hash_str(&mut sha, R::KIND);
    resource.hash_content(&mut sha);
    ShaSum(sha.finalize().into())
}

impl<K, E> Sha for Act<K, E>
where
    K: HashContent,
    E: HashContent,
{
    fn sha512(&self) -> ShaSum {
        sha_resource(self)
    }
}

impl<K> Sha for Package<K> {
    fn sha512(&self) -> ShaSum {
        sha_resource(self)
    }
}

impl<K> Sha for Kustomize<K>
where
    K: HashContent,
{
    fn sha512(&self) -> ShaSum {
        sha_resource(self)
    }
}

impl<K> Sha for Install<K>
where
    K: HashContent,
{
    fn sha512(&self) -> ShaSum {
        sha_resource(self)
    }
}

impl<K, E> Sha for Build<K, E>
where
    K: HashContent,
    E: HashContent,
{
    fn sha512(&self) -> ShaSum {
        sha_resource(self)
    }
}

impl<K, E> Sha for Realize<K, E>
where
    K: HashContent,
    E: HashContent,
{
    fn sha512(&self) -> ShaSum {
        sha_resource(self)
    }
}

impl<T> HashContent for Vec<T>
where
    T: HashContent,
{
    fn hash_content(&self, sha: &mut Sha512) {
        hash_len(sha, self.len());
        for value in self {
            value.hash_content(sha);
        }
    }
}

impl<K, V> HashContent for BTreeMap<K, V>
where
    K: HashContent,
    V: HashContent,
{
    fn hash_content(&self, sha: &mut Sha512) {
        hash_len(sha, self.len());
        for (key, value) in self {
            key.hash_content(sha);
            value.hash_content(sha);
        }
    }
}

impl HashContent for ShaSum {
    fn hash_content(&self, sha: &mut Sha512) {
        hash_bytes(sha, b"sha512", self.as_bytes());
    }
}

impl<K> HashContent for LabelMap<K>
where
    K: HashContent,
{
    fn hash_content(&self, sha: &mut Sha512) {
        hash_len(sha, self.inner.len());
        for (key, value) in &self.inner {
            key.hash_content(sha);
            value.hash_content(sha);
        }
    }
}
