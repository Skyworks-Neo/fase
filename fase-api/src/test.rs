use super::*;

fn header<R>(body: &str) -> String
where
    R: AnyResource,
{
    format!(
        "apiVersion: {}\nkind: {}\n{}",
        R::API_VERSION,
        R::KIND,
        body
    )
}

#[test]
fn kustomize() {
    let kustomize = "resources:\n  - base.yaml\n  - ../../k/";
    let kustomize: Resource = serde_yml::from_str(kustomize).unwrap();
    assert!(matches!(kustomize, Resource::Kustomize(_)));
}

#[test]
fn package() {
    let package = header::<Package>("labels:\n  version: 0.1.0\n  rev: '1'");
    let package: Resource = serde_yml::from_str(&package).unwrap();
    assert!(matches!(package, Resource::Package(_)));
    let package = serde_yml::to_string(&package).unwrap();
    assert!(package.contains(&format!("apiVersion: {}\n", Package::API_VERSION)));
    assert!(package.contains(&format!("kind: {}\n", Package::KIND)));
}

#[test]
fn malvar() {
    let package = header::<Package>("labels:\n  foo/name: bar");
    let package: Result<Resource, serde_yml::Error> = serde_yml::from_str(&package);
    assert!(package.is_err());
}

#[test]
fn install() {
    let install = header::<Install>("must:\n - fooname: bar\nprefer: []");
    let install: Resource = serde_yml::from_str(&install).unwrap();
    assert!(matches!(install, Resource::Install(_)));
}
