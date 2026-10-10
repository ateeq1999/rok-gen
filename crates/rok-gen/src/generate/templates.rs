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
        "service/service.rs",
        include_str!("../../templates/service/service.rs.atom"),
    ),
    (
        "port/port.rs",
        include_str!("../../templates/port/port.rs.atom"),
    ),
    (
        "action/handler.rs",
        include_str!("../../templates/action/handler.rs.atom"),
    ),
    (
        "recipes/mail/Cargo.toml",
        include_str!("../../templates/recipes/mail/Cargo.toml.atom"),
    ),
    (
        "recipes/mail/lib.rs",
        include_str!("../../templates/recipes/mail/lib.rs.atom"),
    ),
    (
        "recipes/storage/Cargo.toml",
        include_str!("../../templates/recipes/storage/Cargo.toml.atom"),
    ),
    (
        "recipes/storage/lib.rs",
        include_str!("../../templates/recipes/storage/lib.rs.atom"),
    ),
    (
        "recipes/auth/Cargo.toml",
        include_str!("../../templates/recipes/auth/Cargo.toml.atom"),
    ),
    (
        "recipes/auth/lib.rs",
        include_str!("../../templates/recipes/auth/lib.rs.atom"),
    ),
    (
        "recipes/terraform/storage-s3/main.tf",
        include_str!("../../templates/recipes/terraform/storage-s3/main.tf.atom"),
    ),
    (
        "recipes/terraform/storage-s3/variables.tf",
        include_str!("../../templates/recipes/terraform/storage-s3/variables.tf.atom"),
    ),
    (
        "recipes/terraform/storage-s3/outputs.tf",
        include_str!("../../templates/recipes/terraform/storage-s3/outputs.tf.atom"),
    ),
];

pub fn renderer() -> Result<Renderer, GenError> {
    let mut renderer = Renderer::new(Trust::BuiltIn);
    for (name, source) in TEMPLATES {
        renderer.add_template(name, source)?;
    }
    Ok(renderer)
}
