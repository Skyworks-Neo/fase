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
fn shasum() {
    let package = include_str!("../contrib/package.yml");
    let package: Resource = serde_yml::from_str(package).unwrap();
    let reordered = include_str!("../contrib/package-reordered.yml");
    let reordered: Resource = serde_yml::from_str(reordered).unwrap();
    let other_labels = include_str!("../contrib/package-other-labels.yml");
    let other_labels: Resource = serde_yml::from_str(other_labels).unwrap();
    let package_sum = package.sha256();
    assert_eq!(package_sum, reordered.sha256());
    assert_eq!(package_sum, other_labels.sha256());
    assert_eq!(package_sum.to_string().len(), 64);
    assert_eq!(
        package_sum.to_string().parse::<ShaSum>().unwrap(),
        package_sum
    );
    let package = include_str!("../contrib/package.yml");
    let resource: Resource = serde_yml::from_str(package).unwrap();
    let package: Package = serde_yml::from_str(package).unwrap();
    assert_eq!(package.sha256(), resource.sha256());
    let package = include_str!("../contrib/package.yml");
    let package: Resource = serde_yml::from_str(package).unwrap();
    let act = include_str!("../contrib/act.yml");
    let act: Resource = serde_yml::from_str(act).unwrap();
    assert_ne!(package.sha256(), act.sha256());
    let relabeled_act = include_str!("../contrib/act-other-labels.yml");
    let relabeled_act: Resource = serde_yml::from_str(relabeled_act).unwrap();
    assert_eq!(act.sha256(), relabeled_act.sha256());
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

#[test]
fn realize() {
    let realize = include_str!("../contrib/realize.yml");
    let realize: Resource = serde_yml::from_str(realize).unwrap();
    assert!(matches!(realize, Resource::Realize(_)));
}
