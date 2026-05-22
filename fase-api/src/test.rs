use super::*;

type RawName = Box<str>;
type RawExpr = Box<str>;
type RawResource = Resource<RawName, RawExpr>;
type RawPackage = Package<RawName>;

#[test]
fn kustomize() {
    let kustomize = include_str!("../contrib/kustomize.yml");
    let kustomize: RawResource = serde_yml::from_str(kustomize).unwrap();
    assert!(matches!(kustomize, Resource::Kustomize(_)));
}

#[test]
fn package() {
    let package = include_str!("../contrib/package.yml");
    let package: RawResource = serde_yml::from_str(package).unwrap();
    assert!(matches!(package, Resource::Package(_)));
    let package = serde_yml::to_string(&package).unwrap();
    assert!(package.contains(&format!("apiVersion: {}\n", RawPackage::API_VERSION)));
    assert!(package.contains(&format!("kind: {}\n", RawPackage::KIND)));
}

#[test]
fn shasum() {
    let package = include_str!("../contrib/package.yml");
    let package: RawResource = serde_yml::from_str(package).unwrap();
    let reordered = include_str!("../contrib/package-reordered.yml");
    let reordered: RawResource = serde_yml::from_str(reordered).unwrap();
    let other_labels = include_str!("../contrib/package-other-labels.yml");
    let other_labels: RawResource = serde_yml::from_str(other_labels).unwrap();
    let package_sum = package.sha256();
    assert_eq!(package_sum, reordered.sha256());
    assert_eq!(package_sum, other_labels.sha256());
    assert_eq!(package_sum.to_string().len(), 64);
    assert_eq!(
        package_sum.to_string().parse::<ShaSum>().unwrap(),
        package_sum
    );
    let package = include_str!("../contrib/package.yml");
    let resource: RawResource = serde_yml::from_str(package).unwrap();
    let package: RawPackage = serde_yml::from_str(package).unwrap();
    assert_eq!(package.sha256(), resource.sha256());
    let package = include_str!("../contrib/package.yml");
    let package: RawResource = serde_yml::from_str(package).unwrap();
    let act = include_str!("../contrib/act.yml");
    let act: RawResource = serde_yml::from_str(act).unwrap();
    assert_ne!(package.sha256(), act.sha256());
    let relabeled_act = include_str!("../contrib/act-other-labels.yml");
    let relabeled_act: RawResource = serde_yml::from_str(relabeled_act).unwrap();
    assert_eq!(act.sha256(), relabeled_act.sha256());
}

#[test]
fn act() {
    let act = include_str!("../contrib/act.yml");
    let act: RawResource = serde_yml::from_str(act).unwrap();
    assert!(matches!(act, Resource::Act(_)));
}

#[test]
fn input_sources() {
    let input: Input<RawExpr> = serde_yml::from_str("typ: env\nname: TOKEN\n").unwrap();
    assert!(matches!(input, Input::Env { name } if name.as_ref() == "TOKEN"));

    let input: Input<RawExpr> = serde_yml::from_str("typ: file\npath: source.txt\n").unwrap();
    assert!(matches!(input, Input::File { path } if path.as_ref() == "source.txt"));

    let input: Input<RawExpr> =
        serde_yml::from_str("typ: http\nurl: https://example.com/source.txt\n").unwrap();
    assert!(
        matches!(input, Input::Http { url } if url.as_ref() == "https://example.com/source.txt")
    );

    let input: Input<RawExpr> = serde_yml::from_str("typ: dir\npath: sources\n").unwrap();
    assert!(matches!(input, Input::Dir { path } if path.as_ref() == "sources"));
}

#[test]
fn expr() {
    let expr: RawExpr = serde_yml::from_str("https://example.com/${path}\n").unwrap();
    assert_eq!(expr.as_ref(), "https://example.com/${path}");
}

#[test]
fn raw_name() {
    let package = include_str!("../contrib/package-invalid-var.yml");
    let package: Result<RawResource, serde_yml::Error> = serde_yml::from_str(package);
    assert!(package.is_ok());
}

#[test]
fn install() {
    let install = include_str!("../contrib/install.yml");
    let install: RawResource = serde_yml::from_str(install).unwrap();
    assert!(matches!(install, Resource::Install(_)));
}

#[test]
fn build() {
    let build = include_str!("../contrib/build.yml");
    let build: RawResource = serde_yml::from_str(build).unwrap();
    assert!(matches!(build, Resource::Build(_)));
}

#[test]
fn realize() {
    let realize = include_str!("../contrib/realize.yml");
    let realize: RawResource = serde_yml::from_str(realize).unwrap();
    assert!(matches!(realize, Resource::Realize(_)));
}

#[test]
fn label() {
    use std::sync::Arc;
    let pool = LabelPool::shared();
    let first = pool.intern("name");
    let second = pool.intern("name");
    let other = pool.intern("version");
    assert_eq!(first.id(), second.id());
    assert_ne!(first.id(), other.id());
    assert_eq!(first.as_ref(), "name");

    let pool = LabelPool::shared();
    let label = pool.intern("old");
    let overridden = label.override_value("new");
    assert_eq!(overridden.as_ref(), "new");
    assert!(Arc::ptr_eq(label.pool(), overridden.pool()));

    let label = Label::intern("source-url");
    assert_eq!(serde_yml::to_string(&label).unwrap(), "source-url\n");
}
