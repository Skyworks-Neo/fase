use super::*;

#[test]
fn kustomize() {
    let kustomize = "resources:\n  - base.yaml\n  - ../../k/";
    let kustomize: Resource = serde_yml::from_str(kustomize).unwrap();
    assert!(matches!(kustomize, Resource::Kustomize(_)));
}

#[test]
fn package() {
    let package = "apiVersion: v1alpha1\nkind: Package\nlabels:\n  version: 0.1.0\n  rev: '1'";
    let package: Resource = serde_yml::from_str(package).unwrap();
    assert!(matches!(package, Resource::Package(_)));
}

#[test]
fn malvar() {
    let package = "apiVersion: v1alpha1\nkind: Package\nlabels:\n  foo/name: bar";
    let package: Result<Resource, serde_yml::Error> = serde_yml::from_str(package);
    assert!(package.is_err());
}

#[test]
fn install() {
    let install = "apiVersion: v1alpha1\nkind: Install\nwants:\n - fooname: bar";
    let install: Resource = serde_yml::from_str(install).unwrap();
    assert!(matches!(install, Resource::Install(_)));
}

#[test]
fn source() {
    let source = "apiVersion: v1alpha1\nkind: Source\nlabels:\n  fooname: bar";
    let source: Resource = serde_yml::from_str(source).unwrap();
    assert!(matches!(source, Resource::Source(_)));
}
