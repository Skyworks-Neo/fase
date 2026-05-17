# fase

A selector-based Linux packager and distribution.

Fase is an experimental package system that models packages, build actions,
and installation requests as small declarative resources. Instead of naming one
exact package everywhere, resources are selected through labels, so builds and
installs can describe what they need while leaving room for policy, variants,
and repository composition.

The long-term CLI goal is a package tool that can collect, transform, report,
build, and manage resources in a style closer to `kubectl` than a single-purpose
build script.
