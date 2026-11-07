// SPDX-License-Identifier: MIT
//! Which reader an anchored file gets, and the readers for JSON, TOML and HCL
//! (`SEMANTICS.md` §Resolving values).

mod hcl;
mod json;
mod toml;
mod yaml_file;

pub use hcl::{HclBlock, HclBody, merge_files, read_hcl};
pub use json::read_json;
pub use toml::read_toml;
pub use yaml_file::read_yaml_file;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Yaml,
    Json,
    Toml,
    Hcl,
}

/// the format of a file, by its last path segment, case-sensitive. `None` is `unverifiable: format`.
pub fn format_of(path: &str) -> Option<Format> {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name {
        "Cargo.lock" | "poetry.lock" | "uv.lock" | "Pipfile" => return Some(Format::Toml),
        "Pipfile.lock" | "flake.lock" | "composer.lock" => return Some(Format::Json),
        _ => {}
    }
    let (_, ext) = name.rsplit_once('.')?;
    match ext {
        "yaml" | "yml" => Some(Format::Yaml),
        "json" => Some(Format::Json),
        "toml" => Some(Format::Toml),
        "tf" | "tfvars" | "hcl" => Some(Format::Hcl),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_table() {
        assert_eq!(format_of("Cargo.lock"), Some(Format::Toml));
        assert_eq!(format_of("a/x.tfvars.json"), Some(Format::Json));
        assert_eq!(format_of("x.yaml.j2"), None);
        assert_eq!(format_of("X.YAML"), None);
    }

    #[test]
    fn every_c1_name() {
        let table = [
            ("a/b.yaml", Some(Format::Yaml)),
            ("b.yml", Some(Format::Yaml)),
            ("x.json", Some(Format::Json)),
            ("pyproject.toml", Some(Format::Toml)),
            ("main.tf", Some(Format::Hcl)),
            ("prod.tfvars", Some(Format::Hcl)),
            ("terragrunt.hcl", Some(Format::Hcl)),
            ("Cargo.lock", Some(Format::Toml)),
            ("poetry.lock", Some(Format::Toml)),
            ("uv.lock", Some(Format::Toml)),
            ("Pipfile", Some(Format::Toml)),
            ("Pipfile.lock", Some(Format::Json)),
            ("flake.lock", Some(Format::Json)),
            ("composer.lock", Some(Format::Json)),
            ("sub/dir/Cargo.lock", Some(Format::Toml)),
            // The last segment decides; a directory's name never does.
            ("x.yaml/README", None),
            ("Cargo.lock/x", None),
            ("cargo.lock", None),
            ("Cargo.lock.bak", None),
            ("package-lock.json", Some(Format::Json)),
            ("yarn.lock", None),
            ("x.JSON", None),
            ("x.tf.bak", None),
            ("Makefile", None),
            ("", None),
            ("x.", None),
        ];
        for (p, want) in table {
            assert_eq!(format_of(p), want, "{p:?}");
        }
    }
}
