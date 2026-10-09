//! `cargo rok make:migration`: the next numbered SQL file.

use rok_edit::Transaction;
use rok_plan::make::MakeMigrationArgs;
use rok_plan::project::Project;

use super::GenError;
use super::names::snake_ident;

pub fn migration_into(
    tx: &mut Transaction,
    project: &Project,
    args: &MakeMigrationArgs,
) -> Result<(), GenError> {
    snake_ident(&args.name, "migration name")?;
    let dir = &project.config.project.migrations;
    if !project.root.join(dir).is_dir() {
        return Err(GenError::Invalid(format!(
            "migrations directory `{dir}` doesn't exist; set [project].migrations in rok.toml"
        )));
    }
    let body = if args.sql.trim().is_empty() {
        format!("-- {}\n", args.name.replace('_', " "))
    } else {
        format!("{}\n", args.sql.trim_end())
    };
    create_next(tx, dir, &args.name, &body)?;
    Ok(())
}

/// Adds `<dir>/<NNNN>_<name>.sql`, numbered one past the highest existing
/// migration (and any already added in this transaction).
pub fn create_next(
    tx: &mut Transaction,
    dir: &str,
    name: &str,
    body: &str,
) -> Result<String, GenError> {
    let number_of = |file: &str| {
        file.split_once('_')
            .and_then(|(number, _)| number.parse::<u32>().ok())
    };
    let on_disk = std::fs::read_dir(tx.root().join(dir))
        .map_err(|err| GenError::Invalid(format!("could not read `{dir}`: {err}")))?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str().and_then(number_of))
        .max()
        .unwrap_or(0);
    let prefix = format!("{dir}/");
    let pending = tx
        .created()
        .iter()
        .filter_map(|rel| rel.strip_prefix(&prefix).and_then(number_of))
        .max()
        .unwrap_or(0);
    let number = on_disk.max(pending) + 1;
    let rel = format!("{dir}/{number:04}_{name}.sql");
    tx.create(&rel, body)?;
    Ok(rel)
}
