use super::*;

pub trait HashContent {
    fn hash_content(&self, sha: &mut Sha256);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaParseError {
    Length,
    Hex,
}

impl std::fmt::Display for ShaParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShaParseError::Length => f.write_str("invalid sha256 sum: expected 64 hex digits"),
            ShaParseError::Hex => f.write_str("invalid sha256 sum: invalid hex digit"),
        }
    }
}

impl std::error::Error for ShaParseError {}

impl std::fmt::Display for ShaSum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl std::str::FromStr for ShaSum {
    type Err = ShaParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 64 {
            return Err(ShaParseError::Length);
        }

        let mut bytes = [0; 32];
        for (byte, pair) in bytes.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
            *byte = hex(pair[0])? << 4 | hex(pair[1])?;
        }
        Ok(Self(bytes))
    }
}

fn hex(value: u8) -> Result<u8, ShaParseError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(ShaParseError::Hex),
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

pub(crate) trait HashWrite {
    fn bytes(&mut self, tag: &[u8], value: &[u8]);
    fn write_len(&mut self, len: usize);
    fn field(&mut self, name: &str) {
        self.bytes(b"field", name.as_bytes());
    }
    fn text(&mut self, value: &str) {
        self.bytes(b"str", value.as_bytes());
    }
}

impl HashWrite for Sha256 {
    fn bytes(&mut self, tag: &[u8], value: &[u8]) {
        self.update(tag);
        self.write_len(value.len());
        self.update(value);
    }

    fn write_len(&mut self, len: usize) {
        self.update((len as u64).to_be_bytes());
    }
}

impl<K, E> Sha for Resource<K, E>
where
    K: HashContent,
    E: HashContent,
{
    fn sha256(&self) -> ShaSum {
        match self {
            Resource::Act(resource) => resource.sha256(),
            Resource::Package(resource) => resource.sha256(),
            Resource::Kustomize(resource) => resource.sha256(),
            Resource::Install(resource) => resource.sha256(),
            Resource::Build(resource) => resource.sha256(),
            Resource::Realize(resource) => resource.sha256(),
        }
    }
}

fn sha_resource<R>(resource: &R) -> ShaSum
where
    R: ResourceKind + HashContent,
{
    let mut sha = Sha256::new();
    sha.field("apiVersion");
    sha.text(R::API_VERSION);
    sha.field("kind");
    sha.text(R::KIND);
    resource.hash_content(&mut sha);
    ShaSum(sha.finalize().into())
}

impl<K, E> Sha for Act<K, E>
where
    K: HashContent,
    E: HashContent,
{
    fn sha256(&self) -> ShaSum {
        sha_resource(self)
    }
}

impl<K> Sha for Package<K> {
    fn sha256(&self) -> ShaSum {
        sha_resource(self)
    }
}

impl<K> Sha for Kustomize<K>
where
    K: HashContent,
{
    fn sha256(&self) -> ShaSum {
        sha_resource(self)
    }
}

impl<K> Sha for Install<K>
where
    K: HashContent,
{
    fn sha256(&self) -> ShaSum {
        sha_resource(self)
    }
}

impl<K, E> Sha for Build<K, E>
where
    K: HashContent,
    E: HashContent,
{
    fn sha256(&self) -> ShaSum {
        sha_resource(self)
    }
}

impl<K, E> Sha for Realize<K, E>
where
    K: HashContent,
    E: HashContent,
{
    fn sha256(&self) -> ShaSum {
        sha_resource(self)
    }
}

impl HashContent for str {
    fn hash_content(&self, sha: &mut Sha256) {
        sha.text(self);
    }
}

impl HashContent for String {
    fn hash_content(&self, sha: &mut Sha256) {
        self.as_str().hash_content(sha);
    }
}

impl<T> HashContent for Box<T>
where
    T: HashContent + ?Sized,
{
    fn hash_content(&self, sha: &mut Sha256) {
        self.as_ref().hash_content(sha);
    }
}

impl<T> HashContent for [T]
where
    T: HashContent,
{
    fn hash_content(&self, sha: &mut Sha256) {
        sha.write_len(self.len());
        for value in self {
            value.hash_content(sha);
        }
    }
}

impl<T> HashContent for Vec<T>
where
    T: HashContent,
{
    fn hash_content(&self, sha: &mut Sha256) {
        self.as_slice().hash_content(sha);
    }
}

impl HashContent for std::path::PathBuf {
    fn hash_content(&self, sha: &mut Sha256) {
        self.to_string_lossy().as_ref().hash_content(sha);
    }
}

impl<K, V> HashContent for BTreeMap<K, V>
where
    K: HashContent,
    V: HashContent,
{
    fn hash_content(&self, sha: &mut Sha256) {
        sha.write_len(self.len());
        for (key, value) in self {
            key.hash_content(sha);
            value.hash_content(sha);
        }
    }
}

impl HashContent for ShaSum {
    fn hash_content(&self, sha: &mut Sha256) {
        sha.bytes(b"sha256", self.as_bytes());
    }
}

impl<K> HashContent for LabelMap<K>
where
    K: HashContent,
{
    fn hash_content(&self, sha: &mut Sha256) {
        sha.write_len(self.len());
        for (key, value) in self.iter() {
            key.hash_content(sha);
            value.hash_content(sha);
        }
    }
}
