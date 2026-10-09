//! The built-in templates, compiled into the binary.

use rok_render::{Renderer, Trust};

use super::GenError;

const TEMPLATES: &[(&str, &str)] = &[
    (
        "feature/Cargo.toml",
        include_str!("../../templates/feature/Cargo.toml.atom"),
    ),
    (
        "feature/lib.rs",
        include_str!("../../templates/feature/lib.rs.atom"),
    ),
    (
        "feature/entity.rs",
        include_str!("../../templates/feature/entity.rs.atom"),
    ),
    (
        "feature/dto/mod.rs",
        include_str!("../../templates/feature/dto/mod.rs.atom"),
    ),
    (
        "feature/dto/create.rs",
        include_str!("../../templates/feature/dto/create.rs.atom"),
    ),
    (
        "feature/dto/update.rs",
        include_str!("../../templates/feature/dto/update.rs.atom"),
    ),
    (
        "feature/repository.rs",
        include_str!("../../templates/feature/repository.rs.atom"),
    ),
    (
        "feature/test_support.rs",
        include_str!("../../templates/feature/test_support.rs.atom"),
    ),
    (
        "feature/service.rs",
        include_str!("../../templates/feature/service.rs.atom"),
    ),
    (
        "feature/action.rs",
        include_str!("../../templates/feature/action.rs.atom"),
    ),
    (
        "feature/module.rs",
        include_str!("../../templates/feature/module.rs.atom"),
    ),
    (
        "feature/error.rs",
        include_str!("../../templates/feature/error/error.rs.atom"),
    ),
    (
        "feature/migration.sql",
        include_str!("../../templates/feature/migration.sql.atom"),
    ),
    (
        "dto/dto.rs",
        include_str!("../../templates/dto/dto.rs.atom"),
    ),
    (
        "action/handler.rs",
        include_str!("../../templates/action/handler.rs.atom"),
    ),
];

pub fn renderer() -> Result<Renderer, GenError> {
    let mut renderer = Renderer::new(Trust::BuiltIn);
    for (name, source) in TEMPLATES {
        renderer.add_template(name, source)?;
    }
    Ok(renderer)
}
