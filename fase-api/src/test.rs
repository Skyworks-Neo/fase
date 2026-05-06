use super::*;

use std::collections::BTreeMap;

#[test]
fn kustomize() {
    let kustomize = include_str!("../contrib/kustomize.yml");
    let kustomize: Resource = serde_yml::from_str(kustomize).unwrap();
    assert!(matches!(kustomize, Resource::Kustomize(_)));
}

#[test]
fn package() {
    let package = include_str!("../contrib/package.yml");
    let package: Resource = serde_yml::from_str(package).unwrap();
    assert!(matches!(package, Resource::Package(_)));
    let package = serde_yml::to_string(&package).unwrap();
    assert!(package.contains(&format!("apiVersion: {}\n", Package::API_VERSION)));
    assert!(package.contains(&format!("kind: {}\n", Package::KIND)));
}

#[test]
fn act() {
    let act = include_str!("../contrib/act.yml");
    let act: Resource = serde_yml::from_str(act).unwrap();
    assert!(matches!(act, Resource::Act(_)));
}

#[test]
fn expr() {
    let expr: Expr = serde_yml::from_str("https://example.com/${path}\n").unwrap();
    let mut vars = BTreeMap::new();
    vars.insert(Var::new("path").unwrap(), "source.tar.gz".to_owned());

    assert_eq!(
        expr.expand(&vars).unwrap(),
        "https://example.com/source.tar.gz"
    );
}

#[test]
fn malvar() {
    let package = include_str!("../contrib/package-invalid-var.yml");
    let package: Result<Resource, serde_yml::Error> = serde_yml::from_str(package);
    assert!(package.is_err());
}

#[test]
fn install() {
    let install = include_str!("../contrib/install.yml");
    let install: Resource = serde_yml::from_str(install).unwrap();
    assert!(matches!(install, Resource::Install(_)));
}

#[test]
fn build() {
    let build = include_str!("../contrib/build.yml");
    let build: Resource = serde_yml::from_str(build).unwrap();
    assert!(matches!(build, Resource::Build(_)));
}
