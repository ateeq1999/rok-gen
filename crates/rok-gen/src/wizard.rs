//! `cargo rok integrate --wizard`: builds an integration plan by asking
//! numbered questions, then hands it to the same engine as a JSON plan.
//!
//! Answers are read line by line from stdin, so the wizard is as easy to
//! script (and test) as it is to use by hand.

use std::io::{BufRead, Write};
use std::path::Path;

use heck::ToSnakeCase;
use rok_locate::Source;
use rok_plan::integrate::{At, IntegrateArgs, Step, StmtIndex, StmtMatcher, TailKeyword};
use rok_plan::project::Project;
use serde_json::json;

use crate::CliError;

/// Answers that aren't a valid choice before the wizard gives up.
const RETRIES: usize = 3;

/// What to do with the finished plan.
pub enum Finish {
    Apply,
    DryRun,
    Save(String),
}

struct Prompter<'a> {
    input: &'a mut dyn BufRead,
    out: &'a mut dyn Write,
}

impl Prompter<'_> {
    fn say(&mut self, text: &str) {
        // A closed prompt stream isn't worth failing over; answers still work.
        let _ = writeln!(self.out, "{text}");
        let _ = self.out.flush();
    }

    fn line(&mut self) -> Result<String, CliError> {
        let _ = write!(self.out, "> ");
        let _ = self.out.flush();
        let mut line = String::new();
        let read = self
            .input
            .read_line(&mut line)
            .map_err(|err| CliError::Flag(format!("could not read an answer: {err}")))?;
        if read == 0 {
            return Err(CliError::Flag(
                "the wizard ran out of answers (stdin closed)".to_string(),
            ));
        }
        Ok(line.trim().to_string())
    }

    fn choose(&mut self, question: &str, options: &[String]) -> Result<usize, CliError> {
        if options.is_empty() {
            return Err(CliError::Flag(format!(
                "{question}: there's nothing to choose from"
            )));
        }
        self.say(question);
        for (index, option) in options.iter().enumerate() {
            self.say(&format!("  {}) {option}", index + 1));
        }
        for _ in 0..RETRIES {
            let answer = self.line()?;
            match answer.parse::<usize>() {
                Ok(number) if (1..=options.len()).contains(&number) => return Ok(number - 1),
                _ => self.say(&format!("pick a number from 1 to {}", options.len())),
            }
        }
        Err(CliError::Flag(format!("no valid answer to `{question}`")))
    }

    fn text(&mut self, question: &str, default: Option<&str>) -> Result<String, CliError> {
        match default {
            Some(default) => self.say(&format!("{question} [{default}]")),
            None => self.say(question),
        }
        let answer = self.line()?;
        Ok(if answer.is_empty() {
            default.unwrap_or_default().to_string()
        } else {
            answer
        })
    }

    /// Lines until an empty one.
    fn block(&mut self, question: &str) -> Result<String, CliError> {
        self.say(&format!("{question} (end with an empty line)"));
        let mut lines = Vec::new();
        loop {
            let line = self.line()?;
            if line.is_empty() {
                break;
            }
            lines.push(line);
        }
        Ok(lines.join("\n"))
    }

    fn confirm(&mut self, question: &str) -> Result<bool, CliError> {
        self.say(&format!("{question} [y/N]"));
        Ok(matches!(
            self.line()?.to_ascii_lowercase().as_str(),
            "y" | "yes"
        ))
    }
}

/// Asks for a plan and what to do with it.
///
/// # Errors
///
/// [`CliError::Flag`] when stdin runs out or an answer can't be used.
pub fn plan(
    project: &Project,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<(IntegrateArgs, Finish), CliError> {
    let mut prompter = Prompter { input, out };
    let mut args = IntegrateArgs::default();
    loop {
        let kinds = [
            "inject a repository into a service",
            "insert code into a function",
            "add a field to a struct",
            "apply a preset (has-many, hook)",
        ]
        .map(str::to_string);
        match prompter.choose("What should this step do?", &kinds)? {
            0 => args.steps.push(inject(project, &mut prompter)?),
            1 => args.steps.push(insert(project, &mut prompter)?),
            2 => args.steps.push(add_field(project, &mut prompter)?),
            _ => {
                let (name, steps) = preset(project, &mut prompter)?;
                args.name = args.name.or(name);
                args.steps.extend(steps);
            }
        }
        if !prompter.confirm("Add another step?")? {
            break;
        }
    }
    let finishes = [
        "apply it",
        "dry run: show the diff",
        "save the plan to a file",
    ]
    .map(str::to_string);
    let finish = match prompter.choose("Then?", &finishes)? {
        0 => Finish::Apply,
        1 => Finish::DryRun,
        _ => Finish::Save(prompter.text("Save to", Some("plans/integration.json"))?),
    };
    Ok((args, finish))
}

fn inject(project: &Project, prompter: &mut Prompter<'_>) -> Result<Step, CliError> {
    let services = declared(project, "_service.rs", "Service", false);
    let labels: Vec<String> = services
        .iter()
        .map(|(alias, name)| format!("{name} ({alias})"))
        .collect();
    let (into, service) =
        services[prompter.choose("Which service gets the dependency?", &labels)?].clone();
    let ports = declared(project, "_repository.rs", "Repository", true);
    let labels: Vec<String> = ports.iter().map(|(_, path)| path.clone()).collect();
    let dep = ports[prompter.choose("Which repository does it need?", &labels)?]
        .1
        .clone();
    let entity = dep
        .rsplit("::")
        .next()
        .unwrap_or_default()
        .trim_end_matches("Repository")
        .to_snake_case();
    let default = rok_render::naming::plural(&entity);
    let name = prompter.text("Field name", Some(&default))?;
    Ok(Step::InjectDependency {
        into,
        service,
        dep,
        name,
    })
}

fn insert(project: &Project, prompter: &mut Prompter<'_>) -> Result<Step, CliError> {
    let file = prompter.text("Which file? (an alias like posts/service, or a path)", None)?;
    let source = parse(project, &file)?;
    let functions = source.function_names();
    let function = functions[prompter.choose("Which function?", &functions)?].clone();
    let inspection = source
        .inspect(Some(&function))
        .map_err(|err| CliError::Flag(err.to_string()))?;
    let statements = inspection
        .functions
        .first()
        .map(|info| info.statements.clone())
        .unwrap_or_default();
    let mut places = vec![
        "at the start".to_string(),
        "at the end (before the tail)".to_string(),
    ];
    for stmt in &statements {
        places.push(format!(
            "before [{}] L{}  {}",
            stmt.index, stmt.line, stmt.text
        ));
        places.push(format!(
            "after  [{}] L{}  {}",
            stmt.index, stmt.line, stmt.text
        ));
    }
    let choice = prompter.choose("Where?", &places)?;
    let mut at = At {
        file,
        function,
        ..At::default()
    };
    match choice {
        0 => at.start = true,
        1 => at.end = true,
        n => {
            let stmt = &statements[(n - 2) / 2];
            let matcher = if stmt.tail {
                StmtMatcher::Index(StmtIndex::Named(TailKeyword::Tail))
            } else {
                StmtMatcher::Index(StmtIndex::Position(stmt.index))
            };
            if (n - 2) % 2 == 0 {
                at.before = Some(matcher);
            } else {
                at.after = Some(matcher);
            }
        }
    }
    let code = prompter.block("The code to insert")?;
    Ok(Step::Insert { at, code })
}

fn add_field(project: &Project, prompter: &mut Prompter<'_>) -> Result<Step, CliError> {
    let file = prompter.text(
        "Which file? (an alias like posts/dto/create_post, or a path)",
        None,
    )?;
    let source = parse(project, &file)?;
    let structs: Vec<String> = source
        .inspect(None)
        .map_err(|err| CliError::Flag(err.to_string()))?
        .structs
        .into_iter()
        .map(|info| info.name)
        .collect();
    let struct_name = structs[prompter.choose("Which struct?", &structs)?].clone();
    let field = prompter.text("The field (e.g. `pub tags: Option<String>`)", None)?;
    let default = prompter.text(
        "A default for existing literals of the struct (empty for none)",
        Some(""),
    )?;
    Ok(Step::AddField {
        file,
        struct_name,
        field,
        default: (!default.is_empty()).then_some(default),
    })
}

fn preset(
    project: &Project,
    prompter: &mut Prompter<'_>,
) -> Result<(Option<String>, Vec<Step>), CliError> {
    let presets = ["has-many", "hook"].map(str::to_string);
    let chosen = prompter.choose("Which preset?", &presets)?;
    let features = features(project);
    let source =
        features[prompter.choose("The source feature (e.g. comments)", &features)?].clone();
    let target = features[prompter.choose("The target feature (e.g. posts)", &features)?].clone();
    let mut vars = json!({ "source": source, "target": target });
    if chosen == 1 {
        let ops = ["create", "update", "delete"].map(str::to_string);
        let on = ops[prompter.choose("Before which operation?", &ops)?].clone();
        vars["on"] = json!(on);
    }
    Ok(crate::generate::preset_steps(
        project,
        &presets[chosen],
        vars,
    )?)
}

fn parse(project: &Project, alias: &str) -> Result<Source, CliError> {
    let rel = crate::generate::resolve_alias(project, alias)?;
    let text = std::fs::read_to_string(project.root.join(&rel))
        .map_err(|err| CliError::Flag(format!("could not read {rel}: {err}")))?;
    Source::parse(&rel, &text).map_err(|err| CliError::Flag(err.to_string()))
}

/// Every crate under `crates/`.
fn features(project: &Project) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(project.root.join("crates"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().join("Cargo.toml").is_file())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .collect();
    names.sort();
    names
}

/// `(file alias, name)` for every top-level struct (or trait, when
/// `traits`) ending in `suffix` in files ending in `file_suffix`; for traits
/// the name is the crate-qualified path (`comments::CommentRepository`).
fn declared(
    project: &Project,
    file_suffix: &str,
    suffix: &str,
    traits: bool,
) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for feature in features(project) {
        let src = project.root.join("crates").join(&feature).join("src");
        let Ok(entries) = std::fs::read_dir(&src) else {
            continue;
        };
        let mut files: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        files.sort();
        for file in files {
            let Some(stem) = file_stem_with(&file, file_suffix) else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let Ok(source) = Source::parse(&stem, &text) else {
                continue;
            };
            for item in &source.file.items {
                let name = match item {
                    syn::Item::Struct(item) if !traits => item.ident.to_string(),
                    syn::Item::Trait(item) if traits => item.ident.to_string(),
                    _ => continue,
                };
                if name.ends_with(suffix) {
                    let alias = format!("{feature}/{stem}");
                    let label = if traits {
                        format!("{feature}::{name}")
                    } else {
                        name
                    };
                    found.push((alias, label));
                }
            }
        }
    }
    found
}

fn file_stem_with(file: &Path, suffix: &str) -> Option<String> {
    let name = file.file_name()?.to_str()?;
    name.ends_with(suffix)
        .then(|| name.trim_end_matches(".rs").to_string())
}
